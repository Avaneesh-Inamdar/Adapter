"""Capture a guest stop through LLDB's Windows debugger, without detaching a worker.

The same file runs as the command-line launcher and inside LLDB's Python runtime.
Only lldb_driver imports the LLDB module.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def read_images(path):
    return [json.loads(line) for line in Path(path).read_text(encoding="utf-8").splitlines() if line]


def symbolize(address, images):
    """Return a containing segment and nearest earlier label in that segment."""
    for image in images:
        for segment in image["segments"]:
            if segment["start"] <= address < segment["end"]:
                candidates = [s for s in image["symbols"]
                              if segment["start"] <= s["address"] <= address]
                nearest = max(candidates, key=lambda s: s["address"], default=None)
                return {
                    "image": image["path"], "image_index": image["index"],
                    "segment": segment["name"],
                    "vmaddr": address - image["slide"],
                    "nearest_symbol": nearest,
                    "symbol_offset": address - nearest["address"] if nearest else None,
                    "symbol_is_address_hint": True,
                }
    return None


def lldb_driver(debugger, config_path):
    """Executed by the installed LLDB, using its own embedded Python module."""
    import lldb
    config = json.loads(Path(config_path).read_text(encoding="utf-8"))
    report = {"schema": 1, "status": "tool_error", "images": [], "diagnostics": {},
              "lldb_version": lldb.SBDebugger.GetVersionString()}
    process = None
    try:
        debugger.SetAsync(False)
        target = debugger.GetSelectedTarget()
        if not target.IsValid():
            raise RuntimeError("LLDB could not create the adapter executable target")
        debugger.SetSelectedTarget(target)
        hook = target.BreakpointCreateByName("adapter_debug_mapped")
        if hook.GetNumLocations() == 0:
            raise RuntimeError("adapter_debug_mapped is absent; rebuild Adapter with debug-map support and retain its PDB")
        launch = lldb.SBLaunchInfo(config["worker_args"])
        launch.SetWorkingDirectory(config["cwd"])
        launch.SetEnvironmentEntries([f"{key}={value}" for key, value in os.environ.items()], False)
        for fd, path in enumerate((config["stdin"], config["stdout"], config["stderr"])):
            if not launch.AddOpenFileAction(fd, path, fd == 0, fd != 0):
                raise RuntimeError(f"LLDB could not configure guest descriptor {fd}")
        error = lldb.SBError()
        process = target.Launch(launch, error)
        if error.Fail() or not process.IsValid():
            raise RuntimeError("LLDB launch failed: " + str(error))
        report["pid"] = process.GetProcessID()
        report["images"] = read_images(config["map"])
        if any(image["pid"] != process.GetProcessID() for image in report["images"]):
            raise RuntimeError("map PID differs from the debugged worker")
        if process.GetState() == lldb.eStateStopped:
            thread = process.GetSelectedThread()
            at_hook = (thread.GetStopReason() == lldb.eStopReasonBreakpoint
                       and thread.GetStopReasonDataCount() >= 1
                       and thread.GetStopReasonDataAtIndex(0) == hook.GetID())
            if at_hook:
                report["mapped_hook_hit"] = True
                target.BreakpointDelete(hook.GetID())
                if not report["images"]:
                    raise RuntimeError("mapped-image hook stopped without image records")
                if config["interactive"] or config["break_vmaddr"] is not None or config["step_instructions"]:
                    image = next((i for i in report["images"] if i["index"] == config["image"]), None)
                    if image is None:
                        raise RuntimeError("requested image index is absent")
                    vmaddr = config["break_vmaddr"]
                    address = image["entry"] if vmaddr is None else vmaddr + image["slide"]
                    if address is None or not any(s["protection"] & 4 and s["start"] <= address < s["end"] for s in image["segments"]):
                        raise RuntimeError("requested breakpoint is outside this image's executable segments")
                    breakpoint = target.BreakpointCreateByAddress(address)
                    if breakpoint.GetNumLocations() == 0:
                        raise RuntimeError("LLDB did not resolve the guest breakpoint")
                    report["requested_breakpoint"] = {"address": address, "image": image["index"]}
                    print(f"Guest breakpoint: 0x{address:x}; map: {config['map']}", flush=True)
                error = process.Continue()
                if error.Fail():
                    raise RuntimeError("LLDB continue failed: " + str(error))
        state = process.GetState()
        report["process_state"] = lldb.SBDebugger.StateAsCString(state)
        if state == lldb.eStateExited:
            report.update(status="exited", guest_exit_code=process.GetExitStatus(),
                          exit_description=process.GetExitDescription())
        elif state == lldb.eStateStopped:
            thread = process.GetSelectedThread()
            steps = []
            for _ in range(config["step_instructions"]):
                if thread.GetStopReason() in (lldb.eStopReasonException, lldb.eStopReasonSignal):
                    break
                before = thread.GetFrameAtIndex(0).GetPC()
                thread.StepInstruction(False)
                if process.GetState() != lldb.eStateStopped:
                    break
                thread = process.GetSelectedThread()
                steps.append({"before": before, "after": thread.GetFrameAtIndex(0).GetPC(),
                              "stop_reason": int(thread.GetStopReason())})
            report["instruction_steps"] = steps
            if process.GetState() != lldb.eStateStopped:
                raise RuntimeError("process stopped being inspectable during instruction stepping")
            frame = thread.GetFrameAtIndex(0)
            reason = thread.GetStopReason()
            pc = frame.GetPC()
            report.update(status="crashed" if reason in (lldb.eStopReasonException, lldb.eStopReasonSignal) else "stopped",
                          stop_reason=int(reason), stop_description=thread.GetStopDescription(2048),
                          pc=pc, location=symbolize(pc, report["images"]))
            if report["status"] == "crashed":
                report["fault_domain"] = "guest" if report["location"] else "host_or_unmapped"
            interpreter = debugger.GetCommandInterpreter()
            commands = {
                "registers": "register read --all",
                "disassembly": "disassemble --start-address $rip --count 16",
                "backtrace": "thread backtrace all -c 32",
                "stack": "memory read --format x --size 8 --count 32 $rsp",
                "threads": "thread list",
            }
            for label, command in commands.items():
                result = lldb.SBCommandReturnObject()
                interpreter.HandleCommand(command, lldb.SBExecutionContext(frame), result)
                text = (result.GetOutput() or "") + (result.GetError() or "")
                report["diagnostics"][label] = {"command": command, "success": result.Succeeded(), "text": text[:131072]}
                print(text, flush=True)
            if config["interactive"]:
                print("Stopped in the worker. Use thread step-inst, breakpoint set -a ADDRESS, continue, or quit.", flush=True)
        else:
            raise RuntimeError("unexpected LLDB process state: " + str(report["process_state"]))
    except Exception as error:
        report.update(status="tool_error", error=str(error))
        print("Adapter debugger: " + str(error), flush=True)
    finally:
        write_json(config["report"], report)
        if not config["interactive"] and process is not None and process.IsValid():
            if process.GetState() not in (lldb.eStateExited, lldb.eStateDetached, lldb.eStateInvalid):
                process.Kill()


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def kill_owned_tree(process):
    if os.name == "nt":
        subprocess.run(["taskkill.exe", "/PID", str(process.pid), "/T", "/F"],
                       capture_output=True, timeout=15, check=False)
    else:
        import signal
        os.killpg(process.pid, signal.SIGKILL)
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=10)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adapter", type=Path, default=ROOT / "target/debug/adapter-run.exe")
    parser.add_argument("--lldb", type=Path, default=Path(os.environ.get("ADAPTER_LLVM", r"C:\Program Files\LLVM\bin")) / "lldb.exe")
    parser.add_argument("--output", type=Path, required=True, help="new output directory; existing directories are refused")
    parser.add_argument("--stdin", type=Path, help="guest stdin file; defaults to an empty file")
    parser.add_argument("--config", type=Path, help="Adapter loader configuration")
    parser.add_argument("--timeout", type=float, default=30, help="whole debugger session limit in seconds")
    parser.add_argument("--interactive", action="store_true", help="stop at guest entry (or specified VM address), then open the LLDB prompt")
    parser.add_argument("--break-vmaddr", type=lambda x: int(x, 0), help="Mach-O virtual address, relocated after the map hook")
    parser.add_argument("--step-instructions", type=int, choices=range(0, 33), default=0,
                        metavar="0..32", help="stop at guest entry/address, then single-step this many instructions")
    parser.add_argument("--image", type=int, default=0, help="map image index for the requested breakpoint")
    parser.add_argument("guest", type=Path)
    parser.add_argument("guest_args", nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    if args.timeout <= 0 or args.timeout > 86400:
        parser.error("timeout must be greater than zero and at most 86400")
    for path in (args.adapter, args.lldb, args.guest, args.stdin, args.config):
        if path is not None and not path.is_file():
            parser.error("file does not exist: " + str(path))
    output = args.output.resolve()
    try:
        output.mkdir(parents=True, exist_ok=False)
    except FileExistsError:
        parser.error("output already exists; choose a new directory: " + str(output))
    (output / "images.jsonl").write_bytes(b"")
    (output / "stdin.empty").write_bytes(b"")
    guest_args = args.guest_args[1:] if args.guest_args[:1] == ["--"] else args.guest_args
    worker_args = ["--worker", "--trace"]
    if args.config:
        worker_args += ["--config", str(args.config.resolve())]
    worker_args += [str(args.guest.resolve()), "--", *guest_args]
    config = {
        "adapter": str(args.adapter.resolve()), "cwd": str(ROOT), "worker_args": worker_args,
        "map": str(output / "images.jsonl"), "report": str(output / "report.json"),
        "stdin": str(args.stdin.resolve()) if args.stdin else str(output / "stdin.empty"),
        "stdout": str(output / "guest.stdout"), "stderr": str(output / "guest.stderr"),
        "interactive": args.interactive, "break_vmaddr": args.break_vmaddr, "image": args.image,
        "step_instructions": args.step_instructions,
    }
    write_json(output / "session.json", config)
    command = [str(args.lldb.resolve()), "--no-lldbinit", "--no-use-colors", "--file", str(args.adapter.resolve())]
    if not args.interactive:
        command += ["--batch"]
    expression = f"script import runpy; runpy.run_path({str(Path(__file__).resolve())!r})['lldb_driver'](lldb.debugger, {str(output / 'session.json')!r})"
    command += ["--one-line", expression]
    manifest = {
        "schema": 1, "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "command": command, "worker_args": worker_args, "timeout_seconds": args.timeout,
        "adapter_sha256": sha256(args.adapter), "guest_sha256": sha256(args.guest),
        "debugger": str(args.lldb.resolve()), "interactive": args.interactive,
    }
    write_json(output / "manifest.json", manifest)
    environment = {**os.environ, "ADAPTER_DEBUG_MAP": config["map"]}
    started = time.monotonic()
    try:
        with (output / "lldb.log").open("wb") as log:
            process = subprocess.Popen(command, cwd=ROOT, env=environment,
                                       stdin=None if args.interactive else subprocess.DEVNULL,
                                       stdout=None if args.interactive else log,
                                       stderr=None if args.interactive else subprocess.STDOUT,
                                       start_new_session=os.name != "nt")
            try:
                process.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                kill_owned_tree(process)
                try:
                    images = read_images(config["map"])
                except (OSError, ValueError):
                    images = []
                write_json(config["report"], {"schema": 1, "status": "timeout", "timeout_seconds": args.timeout, "images": images})
            manifest["lldb_exit_code"] = process.returncode
    except OSError as error:
        write_json(config["report"], {"schema": 1, "status": "tool_error", "error": str(error)})
    if not Path(config["report"]).exists():
        write_json(config["report"], {"schema": 1, "status": "tool_error", "error": "LLDB did not produce a report; inspect lldb.log"})
    manifest["seconds"] = round(time.monotonic() - started, 3)
    write_json(output / "manifest.json", manifest)
    report = json.loads(Path(config["report"]).read_text(encoding="utf-8"))
    print(f"{report['status']}: {config['report']}")
    return 0 if report["status"] in ("crashed", "stopped", "exited") else 1


if __name__ == "__main__":
    raise SystemExit(main())
