"""Actual Windows debugger captures, including failure and process cleanup paths."""
import ctypes
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
FIX = ROOT / "tests/fixtures/generated"
BIN = Path(os.environ.get("ADAPTER_BIN", ROOT / "target/debug"))
LLDB = Path(os.environ.get("ADAPTER_LLVM", r"C:\Program Files\LLVM\bin")) / "lldb.exe"


@unittest.skipUnless(os.name == "nt" and LLDB.is_file(), "Windows LLVM LLDB is required for guest debugger tests")
class GuestDebuggerTests(unittest.TestCase):
    def capture(self, name, *options, guest_args=()):
        directory = tempfile.TemporaryDirectory(prefix="adapter-debug-")
        self.addCleanup(directory.cleanup)
        out = Path(directory.name) / "capture"
        result = subprocess.run([sys.executable, str(ROOT / "tools/guest_debug.py"),
                                 "--adapter", str(BIN / "adapter-run.exe"), "--lldb", str(LLDB),
                                 "--output", str(out), *options, str(FIX / name), "--", *guest_args],
                                text=True, capture_output=True, timeout=20)
        report = json.loads((out / "report.json").read_text(encoding="utf-8"))
        return result, report, out

    def test_real_illegal_instruction_fault_and_guest_symbol(self):
        result, report, out = self.capture("crash")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(report["status"], "crashed")
        self.assertEqual(report["fault_domain"], "guest")
        self.assertIn("0xc000001d", report["stop_description"])
        self.assertTrue(report["mapped_hook_hit"])
        self.assertEqual(report["location"]["nearest_symbol"]["name"], "_main")
        self.assertEqual(report["location"]["symbol_offset"], 4)
        self.assertIn("ud2", report["diagnostics"]["disassembly"]["text"])
        self.assertTrue(all(x["success"] for x in report["diagnostics"].values()))
        self.assertIn("rip", report["diagnostics"]["registers"]["text"])
        manifest = json.loads((out / "manifest.json").read_text())
        self.assertEqual(manifest["worker_args"][0], "--worker")
        self.assertEqual(len(manifest["guest_sha256"]), 64)

    def test_single_instruction_changes_guest_pc(self):
        result, report, _ = self.capture("crash", "--step-instructions", "1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(report["status"], "stopped")
        self.assertEqual(len(report["instruction_steps"]), 1)
        step = report["instruction_steps"][0]
        self.assertEqual(step["after"], step["before"] + 1)
        self.assertEqual(report["location"]["symbol_offset"], 1)

    def test_explicit_guest_vm_breakpoint(self):
        result, report, _ = self.capture("crash", "--break-vmaddr", "0x1000002a4")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(report["status"], "stopped")
        self.assertEqual(report["location"]["vmaddr"], 0x1000002A4)
        self.assertIn("breakpoint", report["stop_description"])

    def test_normal_nonzero_exit_is_not_a_crash(self):
        result, report, _ = self.capture("return37")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(report["status"], "exited")
        self.assertEqual(report["guest_exit_code"], 37)

    def test_guest_stdout_and_trace_are_retained(self):
        result, report, out = self.capture("hello", guest_args=("hello",))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(report["status"], "exited")
        self.assertEqual(report["guest_exit_code"], 0)
        self.assertIn(b"Mach-O code executed", (out / "guest.stdout").read_bytes())
        self.assertIn(b"[map]", (out / "guest.stderr").read_bytes())

    def test_bad_guest_breakpoint_is_tool_error(self):
        result, report, _ = self.capture("crash", "--break-vmaddr", "0x1")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["status"], "tool_error")
        self.assertIn("outside", report["error"])

    def test_missing_debug_hook_is_not_a_guest_crash(self):
        result, report, _ = self.capture("crash", "--adapter", str(BIN / "adapter-inspect.exe"))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["status"], "tool_error")
        self.assertIn("adapter_debug_mapped is absent", report["error"])
        self.assertNotIn("pc", report)

    def test_timeout_kills_owned_worker(self):
        result, report, _ = self.capture("loop", "--timeout", "2")
        self.assertEqual(result.returncode, 1)
        self.assertEqual(report["status"], "timeout")
        self.assertTrue(report["images"], "timeout must occur after worker mapping for cleanup evidence")
        pid = report["images"][0]["pid"]
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
        kernel.OpenProcess.restype = ctypes.c_void_p
        kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        handle = kernel.OpenProcess(0x00100000, False, pid)
        if handle:
            try:
                self.assertEqual(kernel.WaitForSingleObject(handle, 0), 0, "worker survived debugger timeout")
            finally:
                kernel.CloseHandle(handle)

    def test_existing_output_is_refused(self):
        with tempfile.TemporaryDirectory(prefix="adapter-debug-existing-") as directory:
            path = Path(directory)
            sentinel = path / "keep.txt"
            sentinel.write_text("preserve")
            result = subprocess.run([sys.executable, str(ROOT / "tools/guest_debug.py"),
                                     "--adapter", str(BIN / "adapter-run.exe"), "--lldb", str(LLDB),
                                     "--output", directory, str(FIX / "crash")],
                                    capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertIn("output already exists", result.stderr)
            self.assertEqual(sentinel.read_text(), "preserve")
            self.assertEqual(list(path.iterdir()), [sentinel])


if __name__ == "__main__":
    unittest.main()
