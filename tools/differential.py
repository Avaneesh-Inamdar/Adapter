#!/usr/bin/env python3
"""Build, explicitly capture, and compare the identical Intel Mach-O consumer.

Only capture executes guest code. No expected libc values are synthesized here.
The standard-library-only verifier treats incomplete/invalid evidence as failure.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "tests/differential/libc_cases.c"
SCHEMA = 1
SUITE = "adapter-libc-v1"
SEED = "0x6d2b79f5"
PROGRAM = "libc-differential"
FAMILIES = ("strlen", "strcmp", "memcmp", "memmove", "strtol", "snprintf", "gmtime_r", "timegm")
RESULT_LENGTHS = dict(zip(FAMILIES, (1, 1, 1, 1, 3, 2, 11, 11)))
CHILD_ENV = {"LC_ALL": "C", "LANG": "C", "TZ": "UTC0"}
SHA = re.compile(r"[0-9a-f]{64}\Z")


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def unique_object(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise ValueError(f"duplicate JSON key: {key}")
        out[key] = value
    return out


def strict_json(text):
    def reject_constant(value):
        raise ValueError(f"non-JSON number: {value}")
    return json.loads(text, object_pairs_hook=unique_object, parse_constant=reject_constant)


def read_json(path):
    return strict_json(Path(path).read_text(encoding="utf-8"))


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n", encoding="utf-8")
    temporary.replace(path)


def case_count(value):
    value = int(value)
    if value < 8 or value > 65536 or value % 8:
        raise ValueError("count must be a multiple of 8 in 8..65536")
    return value


def host_metadata():
    return {"system": platform.system(), "machine": platform.machine(),
            "release": platform.release(), "version": platform.version(),
            "python": platform.python_version()}


def macho_x64(path):
    with Path(path).open("rb") as source:
        header = source.read(32)
    if len(header) < 32:
        raise ValueError("program is truncated, expected thin x86-64 Mach-O executable")
    magic, cpu, _, filetype = struct.unpack_from("<IIII", header)
    if (magic, cpu, filetype) != (0xFEEDFACF, 0x01000007, 2):
        raise ValueError("program is not a thin x86-64 Mach-O executable")


def pe_x64(path):
    with Path(path).open("rb") as source:
        header = source.read(64)
        if len(header) < 64 or header[:2] != b"MZ":
            raise ValueError("adapter is not a Windows PE executable")
        offset = struct.unpack_from("<I", header, 60)[0]
        source.seek(offset)
        pe = source.read(6)
    if len(pe) != 6 or pe[:4] != b"PE\0\0" or struct.unpack_from("<H", pe, 4)[0] != 0x8664:
        raise ValueError("adapter is not a Windows x86-64 PE executable")


def manifest_errors(manifest):
    errors = []
    if not isinstance(manifest, dict):
        return ["manifest is not an object"]
    if type(manifest.get("schema")) is not int or manifest.get("schema") != SCHEMA or manifest.get("suite") != SUITE:
        errors.append("manifest schema/suite mismatch")
    try:
        count = manifest["count"]
        if type(count) is not int or case_count(count) != count:
            errors.append("invalid manifest count")
    except (KeyError, TypeError, ValueError):
        errors.append("invalid manifest count")
    if manifest.get("families") != list(FAMILIES) or manifest.get("seed") != SEED:
        errors.append("manifest generator/families mismatch")
    program = manifest.get("program", {})
    if not isinstance(program, dict) or program.get("file") != PROGRAM or not SHA.fullmatch(str(program.get("sha256", ""))):
        errors.append("invalid manifest program identity")
    source = manifest.get("source", {})
    if not isinstance(source, dict) or source.get("file") != "libc_cases.c" or not SHA.fullmatch(str(source.get("sha256", ""))):
        errors.append("invalid manifest source identity")
    compiler = manifest.get("build", {}).get("compiler", {}) if isinstance(manifest.get("build"), dict) else {}
    if not isinstance(compiler, dict) or not SHA.fullmatch(str(compiler.get("sha256", ""))):
        errors.append("missing/invalid build compiler hash")
    return errors


def build_bundle(output, count=4096):
    count = case_count(count)
    if platform.system() != "Darwin":
        raise ValueError("build requires macOS and its host libSystem linker; no guest is executed")
    output = Path(output).resolve()
    if (output / "manifest.json").exists() or (output / PROGRAM).exists():
        raise ValueError("bundle already contains a program/manifest; choose a fresh output directory")
    compiler = shutil.which("clang")
    if not compiler:
        raise ValueError("clang was not found")
    compiler = str(Path(compiler).resolve())
    version = subprocess.run([compiler, "--version"], capture_output=True, text=True, timeout=15, check=True).stdout
    output.mkdir(parents=True, exist_ok=True)
    copied_source = output / "libc_cases.c"
    shutil.copyfile(SOURCE, copied_source)
    command = [compiler, "-target", "x86_64-apple-macos10.13", "-std=c11", "-O1",
               "-fno-builtin", "-fno-stack-protector", "-fno-asynchronous-unwind-tables",
               "-Wl,-no_uuid", f"-DDIFF_CASE_COUNT={count}", str(copied_source), "-o", str(output / PROGRAM)]
    process = subprocess.run(command, capture_output=True, text=True, timeout=60)
    if process.returncode:
        raise ValueError(f"compile/link failed ({process.returncode}):\n{process.stdout}\n{process.stderr}")
    macho_x64(output / PROGRAM)
    manifest = {"schema": SCHEMA, "suite": SUITE, "seed": SEED, "count": count,
                "families": list(FAMILIES),
                "program": {"file": PROGRAM, "sha256": sha256(output / PROGRAM)},
                "source": {"file": copied_source.name, "sha256": sha256(copied_source)},
                "build": {"command": command, "compiler": {"path": compiler, "sha256": sha256(compiler), "version": version},
                          "host": host_metadata(), "stdout": process.stdout, "stderr": process.stderr}}
    write_json(output / "manifest.json", manifest)
    return manifest


def parse_records(stdout, count):
    """Reject duplicates, wrong schema/types, missing IDs, and reordered lines."""
    records, errors, seen = [], [], set()
    lines = stdout.splitlines()
    if len(lines) != count:
        errors.append(f"expected {count} JSONL records, got {len(lines)}")
    for line_number, line in enumerate(lines, 1):
        try:
            item = strict_json(line)
        except (ValueError, TypeError) as exc:
            errors.append(f"line {line_number}: invalid JSON: {exc}")
            continue
        if not isinstance(item, dict) or set(item) != {"id", "family", "index", "result"}:
            errors.append(f"line {line_number}: invalid record keys")
            continue
        index, family, identifier, result = item["index"], item["family"], item["id"], item["result"]
        if type(index) is not int or index < 0 or index >= count:
            errors.append(f"line {line_number}: invalid case index")
            continue
        expected_family = FAMILIES[index % len(FAMILIES)]
        expected_id = f"{expected_family}:{index}"
        if family != expected_family or identifier != expected_id:
            errors.append(f"line {line_number}: family/id mismatch for index {index}")
            continue
        if identifier in seen:
            errors.append(f"duplicate case ID: {identifier}")
        seen.add(identifier)
        if index != line_number - 1:
            errors.append(f"line {line_number}: unexpected case order/index {index}")
        if (not isinstance(result, list) or len(result) != RESULT_LENGTHS[family]
                or any(type(value) is not int or not -(1 << 63) <= value < (1 << 63) for value in result)):
            errors.append(f"line {line_number}: invalid {family} result vector")
            continue
        records.append(item)
    expected = {f"{FAMILIES[i % 8]}:{i}" for i in range(count)}
    missing = sorted(expected - seen)
    if missing:
        errors.append(f"missing {len(missing)} case IDs: {', '.join(missing[:8])}")
    return records, errors


def capture_bundle(bundle, output, adapter=None, timeout=60):
    bundle, output = Path(bundle).resolve(), Path(output).resolve()
    manifest = read_json(bundle / "manifest.json")
    errors = manifest_errors(manifest)
    if errors:
        raise ValueError("; ".join(errors))
    program = bundle / PROGRAM
    macho_x64(program)
    digest = sha256(program)
    if digest != manifest["program"]["sha256"]:
        raise ValueError("bundle program SHA-256 does not match its manifest")
    host = host_metadata()
    adapter_info = None
    if adapter:
        if host["system"] != "Windows":
            raise ValueError("adapter capture requires Windows; use native capture on Darwin")
        adapter_path = Path(adapter).resolve()
        pe_x64(adapter_path)
        adapter_info = {"path": str(adapter_path), "sha256": sha256(adapter_path)}
        command = [str(adapter_path), "--timeout", str(timeout), str(program)]
    else:
        if host["system"] != "Darwin":
            raise ValueError("native reference capture requires Darwin; Windows requires --adapter")
        command = [str(program)]
    payload = {"schema": SCHEMA, "suite": SUITE, "mode": "adapter" if adapter else "native",
               "program_sha256": digest, "manifest": manifest, "host": host,
               "adapter": adapter_info, "command": command, "environment": CHILD_ENV.copy(),
               "returncode": None, "timed_out": False, "stdout": "", "stderr": "", "records": [], "errors": []}
    try:
        process = subprocess.run(command, capture_output=True, input=b"", env={**os.environ, **CHILD_ENV},
                                 timeout=timeout + (5 if adapter else 0))
        stdout, stderr = process.stdout, process.stderr
        payload["returncode"] = process.returncode
    except subprocess.TimeoutExpired as exc:
        stdout, stderr = exc.stdout or b"", exc.stderr or b""
        payload["timed_out"] = True
        payload["errors"].append(f"execution exceeded {timeout}s timeout")
    except OSError as exc:
        stdout, stderr = b"", b""
        payload["errors"].append(f"execution failed: {exc}")
    for name, raw in (("stdout", stdout), ("stderr", stderr)):
        try:
            payload[name] = raw.decode("utf-8")
        except UnicodeDecodeError:
            payload[name] = raw.decode("utf-8", errors="replace")
            payload["errors"].append(f"{name} is not UTF-8")
    payload["records"], validation = parse_records(payload["stdout"], manifest["count"])
    payload["errors"].extend(validation)
    if payload["returncode"] != 0:
        payload["errors"].append(f"program exit status is {payload['returncode']}, expected 0")
    payload["status"] = "captured" if not payload["errors"] else "invalid"
    write_json(output, payload)
    return payload


def capture_errors(capture, reference=False):
    errors = []
    if not isinstance(capture, dict):
        return ["capture is not an object"]
    if type(capture.get("schema")) is not int or capture.get("schema") != SCHEMA or capture.get("suite") != SUITE:
        errors.append("capture schema/suite mismatch")
    manifest = capture.get("manifest")
    validation = manifest_errors(manifest)
    errors.extend(validation)
    if capture.get("status") != "captured" or capture.get("errors") != []:
        errors.append("capture did not validate successfully")
    if type(capture.get("returncode")) is not int or capture["returncode"] != 0 or capture.get("timed_out") is not False:
        errors.append("capture did not finish with exit 0")
    host = capture.get("host", {})
    system = host.get("system") if isinstance(host, dict) else None
    mode = capture.get("mode")
    if mode == "native":
        if system != "Darwin" or capture.get("adapter") is not None:
            errors.append("native capture host is not Darwin")
    elif mode == "adapter":
        adapter = capture.get("adapter")
        if system != "Windows" or not isinstance(adapter, dict) or not SHA.fullmatch(str(adapter.get("sha256", ""))):
            errors.append("adapter capture lacks Windows/adapter identity")
    else:
        errors.append("unknown capture execution mode")
    if reference and (system != "Darwin" or mode != "native"):
        errors.append("reference must be a native Darwin capture")
    digest = capture.get("program_sha256")
    if not SHA.fullmatch(str(digest or "")):
        errors.append("invalid capture program SHA-256")
    if not validation:
        if digest != manifest["program"]["sha256"]:
            errors.append("capture/manifest program SHA-256 mismatch")
        stdout = capture.get("stdout")
        if not isinstance(stdout, str):
            errors.append("missing stdout evidence")
        else:
            records, record_errors = parse_records(stdout, manifest["count"])
            errors.extend(record_errors)
            if capture.get("records") != records:
                errors.append("stored records do not match stdout evidence")
    if capture.get("environment") != CHILD_ENV:
        errors.append("capture environment does not match C/UTC fixture contract")
    return errors


def compare_captures(reference_path, candidate_path, output):
    report = {"schema": SCHEMA, "suite": SUITE, "status": "invalid", "errors": [],
              "reference": str(reference_path), "candidate": str(candidate_path),
              "case_count": 0, "matched": 0, "mismatched": 0, "differences": []}
    captures = []
    for label, path in (("reference", reference_path), ("candidate", candidate_path)):
        try:
            value = read_json(path)
            captures.append(value)
            report["errors"].extend(f"{label}: {error}" for error in capture_errors(value, reference=(label == "reference")))
        except (OSError, ValueError, TypeError) as exc:
            captures.append(None)
            report["errors"].append(f"{label}: {exc}")
    if not report["errors"]:
        reference, candidate = captures
        if reference["program_sha256"] != candidate["program_sha256"]:
            report["errors"].append("reference and candidate program SHA-256 differ")
        if reference["manifest"]["count"] != candidate["manifest"]["count"]:
            report["errors"].append("reference and candidate case counts differ")
        if reference["manifest"]["source"] != candidate["manifest"]["source"]:
            report["errors"].append("reference and candidate source identities differ")
    if not report["errors"]:
        report["program_sha256"] = reference["program_sha256"]
        report["case_count"] = reference["manifest"]["count"]
        report["families"] = {name: {"matched": 0, "mismatched": 0} for name in FAMILIES}
        for expected, actual in zip(reference["records"], candidate["records"]):
            key = "matched" if expected == actual else "mismatched"
            report[key] += 1
            report["families"][expected["family"]][key] += 1
            if key == "mismatched" and len(report["differences"]) < 100:
                report["differences"].append({"id": expected["id"], "reference": expected, "candidate": actual})
        report["differences_truncated"] = report["mismatched"] > len(report["differences"])
        report["status"] = "different" if report["mismatched"] else "equal"
    write_json(output, report)
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    build = sub.add_parser("build", help="compile on macOS without executing the program")
    build.add_argument("--output", required=True, type=Path)
    build.add_argument("--count", default=4096, type=int)
    capture = sub.add_parser("capture", help="explicitly execute a bundle and validate all JSONL cases")
    capture.add_argument("--bundle", required=True, type=Path)
    capture.add_argument("--output", required=True, type=Path)
    capture.add_argument("--adapter", type=Path)
    capture.add_argument("--timeout", default=60, type=int)
    compare = sub.add_parser("compare", help="require a complete Darwin reference and compare the identical binary")
    compare.add_argument("--reference", required=True, type=Path)
    compare.add_argument("--candidate", required=True, type=Path)
    compare.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "build":
            result = build_bundle(args.output, args.count)
            print(f"built {result['count']} cases; program SHA-256 {result['program']['sha256']}; execution not attempted")
            return 0
        if args.command == "capture":
            if not 1 <= args.timeout <= 3600:
                raise ValueError("timeout must be in 1..3600 seconds")
            result = capture_bundle(args.bundle, args.output, args.adapter, args.timeout)
            print(f"{result['status']}: {len(result['records'])} records; evidence {args.output}")
            return 0 if result["status"] == "captured" else 2
        result = compare_captures(args.reference, args.candidate, args.output)
        print(f"{result['status']}: {result['matched']} matched, {result['mismatched']} mismatched; report {args.output}")
        return 0 if result["status"] == "equal" else 1 if result["status"] == "different" else 2
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        print(f"differential: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
