//! launchd-style job registration for the broker.
//!
//! The supported keys are read from a property list; submission through
//! `launch_msg` and launchd's socket/event activation are not implemented.
use plist::Value;
use std::{
    collections::BTreeSet,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

const SUPPORTED: &[&str] = &[
    "Label",
    "Program",
    "ProgramArguments",
    "MachServices",
    "EnvironmentVariables",
    "StandardOutPath",
    "StandardErrorPath",
    "RunAtLoad",
];

#[derive(Clone, Debug)]
pub struct Job {
    pub label: String,
    pub program: String,
    pub arguments: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub run_at_load: bool,
    pub services: Vec<String>,
}

impl Job {
    pub fn load(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("launchd job {}: {e}", path.display()))?;
        let value = Value::from_reader(BufReader::new(file))
            .map_err(|e| format!("launchd job {}: {e}", path.display()))?;
        Self::parse(value).map_err(|e| format!("launchd job {}: {e}", path.display()))
    }

    fn parse(value: Value) -> Result<Self, String> {
        let dictionary = value
            .as_dictionary()
            .ok_or("job plist must be a dictionary")?;
        for key in dictionary.keys() {
            if !SUPPORTED.contains(&key.as_str()) {
                return Err(format!("unsupported launchd key {key}"));
            }
        }
        let label = string(dictionary, "Label")?;
        if label.is_empty() || label.len() > 255 || label.contains('\0') {
            return Err("Label must be 1-255 bytes".into());
        }
        let program_key = dictionary
            .get("Program")
            .map(|value| value.as_string().ok_or("Program must be a string"))
            .transpose()?;
        let program_arguments = match dictionary.get("ProgramArguments") {
            None => Vec::new(),
            Some(value) => value
                .as_array()
                .ok_or("ProgramArguments must be an array")?
                .iter()
                .map(|entry| {
                    entry
                        .as_string()
                        .map(str::to_string)
                        .ok_or_else(|| "ProgramArguments entries must be strings".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        let (program, arguments) = match (program_key, program_arguments.as_slice()) {
            (Some(program), arguments) => {
                if program.is_empty() || program.contains('\0') {
                    return Err("Program must not be empty".into());
                }
                // adapter-run supplies argv[0] from the guest path, so the
                // declared argv[0] element is not forwarded.
                let arguments = if arguments.is_empty() {
                    Vec::new()
                } else {
                    arguments[1..].to_vec()
                };
                (program.to_string(), arguments)
            }
            (None, [program, arguments @ ..]) => {
                if program.is_empty() || program.contains('\0') {
                    return Err("ProgramArguments[0] must not be empty".into());
                }
                (program.clone(), arguments.to_vec())
            }
            (None, []) => return Err("job has neither Program nor ProgramArguments".into()),
        };
        let environment = match dictionary.get("EnvironmentVariables") {
            None => Vec::new(),
            Some(value) => value
                .as_dictionary()
                .ok_or("EnvironmentVariables must be a dictionary")?
                .iter()
                .map(|(name, value)| {
                    if name.is_empty() || name.contains(['=', '\0']) {
                        return Err(
                            "environment variable names must be nonempty and contain no '='"
                                .to_string(),
                        );
                    }
                    let value = value
                        .as_string()
                        .ok_or_else(|| format!("EnvironmentVariables {name} must be a string"))?;
                    if value.contains('\0') {
                        return Err(format!("EnvironmentVariables {name} contains NUL"));
                    }
                    Ok((name.clone(), value.to_string()))
                })
                .collect::<Result<Vec<_>, String>>()?,
        };
        let stdout = optional_path(dictionary, "StandardOutPath")?;
        let stderr = optional_path(dictionary, "StandardErrorPath")?;
        let run_at_load = match dictionary.get("RunAtLoad") {
            None => false,
            Some(value) => value.as_boolean().ok_or("RunAtLoad must be a boolean")?,
        };
        let mut services = Vec::new();
        if let Some(value) = dictionary.get("MachServices") {
            let services_dictionary = value
                .as_dictionary()
                .ok_or("MachServices must be a dictionary")?;
            for (name, enabled) in services_dictionary {
                let advertised =
                    enabled.as_boolean().unwrap_or(false) || enabled.as_dictionary().is_some();
                if !advertised {
                    continue;
                }
                if name.is_empty() || name.len() >= 128 || name.contains('\0') {
                    return Err("MachServices names must be 1-127 bytes".into());
                }
                services.push(name.clone());
            }
        }
        services.sort();
        let unique: BTreeSet<_> = services.iter().collect();
        if unique.len() != services.len() {
            return Err("MachServices contains duplicate names".into());
        }
        Ok(Self {
            label,
            program,
            arguments,
            environment,
            stdout,
            stderr,
            run_at_load,
            services,
        })
    }
}

fn string(dictionary: &plist::Dictionary, key: &str) -> Result<String, String> {
    dictionary
        .get(key)
        .and_then(Value::as_string)
        .map(str::to_string)
        .ok_or_else(|| format!("job has no string {key}"))
}

fn optional_path(dictionary: &plist::Dictionary, key: &str) -> Result<Option<String>, String> {
    dictionary
        .get(key)
        .map(|value| {
            let path = value
                .as_string()
                .ok_or_else(|| format!("{key} must be a string"))?;
            if path.is_empty() || path.contains('\0') {
                return Err(format!("{key} must be a nonempty path"));
            }
            Ok(path.to_string())
        })
        .transpose()
}

/// Map `/C/Users/...` to `C:\Users\...`; other paths are used as given.
pub fn host_path(path: &str) -> PathBuf {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b'/' {
        let mut text = String::with_capacity(path.len());
        text.push(bytes[1] as char);
        text.push(':');
        text.push_str(&path[2..].replace('/', "\\"));
        return PathBuf::from(text);
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse(xml: &str) -> Result<Job, String> {
        Job::parse(Value::from_reader(Cursor::new(xml.as_bytes())).unwrap())
    }

    const HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>"#;
    const FOOTER: &str = "</dict></plist>";

    fn job(body: &str) -> Result<Job, String> {
        parse(&format!("{HEADER}{body}{FOOTER}"))
    }

    #[test]
    fn parses_program_arguments_and_services() {
        let job = job(r#"<key>Label</key><string>com.example.echo</string>
               <key>Program</key><string>/C/Adapter/mach_server</string>
               <key>ProgramArguments</key><array>
                 <string>mach_server</string><string>echo</string><string>svc</string>
               </array>
               <key>MachServices</key><dict>
                 <key>com.example.echo</key><true/>
                 <key>com.example.optional</key><false/>
                 <key>com.example.dict</key><dict/>
               </dict>
               <key>EnvironmentVariables</key><dict><key>MODE</key><string>test</string></dict>
               <key>StandardOutPath</key><string>/C/Adapter/out.log</string>
               <key>RunAtLoad</key><true/>"#)
        .unwrap();
        assert_eq!(job.label, "com.example.echo");
        assert_eq!(job.program, "/C/Adapter/mach_server");
        assert_eq!(job.arguments, ["echo", "svc"]);
        assert_eq!(job.services, ["com.example.dict", "com.example.echo"]);
        assert_eq!(job.environment, [("MODE".into(), "test".into())]);
        assert_eq!(job.stdout.as_deref(), Some("/C/Adapter/out.log"));
        assert!(job.run_at_load);
    }

    #[test]
    fn program_arguments_can_supply_the_program() {
        let job = job(r#"<key>Label</key><string>com.example.tool</string>
               <key>ProgramArguments</key><array>
                 <string>/C/Adapter/tool</string><string>--flag</string>
               </array>"#)
        .unwrap();
        assert_eq!(job.program, "/C/Adapter/tool");
        assert_eq!(job.arguments, ["--flag"]);
        assert!(job.services.is_empty());
    }

    #[test]
    fn rejects_missing_and_invalid_fields() {
        assert!(job("").unwrap_err().contains("Label"));
        assert!(
            job("<key>Label</key><string>x</string>")
                .unwrap_err()
                .contains("Program")
        );
        assert!(
            job(r#"<key>Label</key><string>x</string><key>KeepAlive</key><true/>"#)
                .unwrap_err()
                .contains("unsupported launchd key KeepAlive")
        );
        assert!(
            job(
                r#"<key>Label</key><string>x</string><key>Program</key><string>/x</string>
                   <key>EnvironmentVariables</key><dict><key>A=B</key><string>v</string></dict>"#
            )
            .unwrap_err()
            .contains("environment variable names")
        );
    }

    #[test]
    fn maps_guest_drive_paths() {
        assert_eq!(
            host_path("/C/Users/me/log.txt"),
            PathBuf::from("C:\\Users\\me\\log.txt")
        );
        assert_eq!(host_path("relative.txt"), PathBuf::from("relative.txt"));
        assert_eq!(host_path("/usr/local"), PathBuf::from("/usr/local"));
    }
}
