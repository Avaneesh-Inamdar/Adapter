use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=ADAPTER_LLVM");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let llvm = PathBuf::from(
        env::var_os("ADAPTER_LLVM").unwrap_or_else(|| r"C:\Program Files\LLVM\bin".into()),
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut objects = Vec::new();
    for source in ["bridge.c", "format.c", "system.c", "entry.s"] {
        let input = PathBuf::from("src/darwin/host").join(source);
        let object = out.join(source.replace('.', "_") + ".obj");
        println!("cargo:rerun-if-changed={}", input.display());
        let mut command = Command::new(llvm.join("clang.exe"));
        command.args(["--target=x86_64-pc-windows-msvc", "-c"]);
        if source.ends_with(".c") {
            command.args([
                "-std=c11",
                "-O2",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-fms-runtime-lib=dll",
                "-D_CRT_SECURE_NO_WARNINGS",
            ]);
        }
        let status = command
            .arg(&input)
            .arg("-o")
            .arg(&object)
            .status()
            .expect("LLVM clang is required for the Darwin libc ABI bridge; set ADAPTER_LLVM");
        assert!(
            status.success(),
            "native ABI bridge compilation failed: {source}"
        );
        objects.push(object);
    }
    println!("cargo:rerun-if-changed=src/darwin/host/bridge.h");
    let status = Command::new(llvm.join("llvm-ar.exe"))
        .arg("rcs")
        .arg(out.join("db_libc.lib"))
        .args(objects)
        .status()
        .unwrap();
    assert!(status.success(), "ABI bridge archive failed");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=db_libc");
    println!("cargo:rustc-link-lib=bcrypt");
}
