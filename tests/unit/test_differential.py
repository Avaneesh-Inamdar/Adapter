"""Evidence validation tests; synthetic captures are not native Mac results."""
import copy
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
import differential as diff


def records(count=8):
    return [{"id": f"{diff.FAMILIES[i % 8]}:{i}", "family": diff.FAMILIES[i % 8],
             "index": i, "result": [0] * diff.RESULT_LENGTHS[diff.FAMILIES[i % 8]]} for i in range(count)]


def stdout_for(items):
    return "".join(json.dumps(item) + "\n" for item in items)


def manifest(count=8, program_hash="a" * 64):
    return {"schema": 1, "suite": diff.SUITE, "count": count, "seed": diff.SEED,
            "families": list(diff.FAMILIES), "program": {"file": diff.PROGRAM, "sha256": program_hash},
            "source": {"file": "libc_cases.c", "sha256": "b" * 64},
            "build": {"compiler": {"sha256": "c" * 64, "version": "synthetic test compiler"}}}


def capture(count=8, windows=False, program_hash="a" * 64):
    items = records(count)
    return {"schema": 1, "suite": diff.SUITE, "mode": "adapter" if windows else "native",
            "host": {"system": "Windows" if windows else "Darwin"},
            "adapter": {"sha256": "d" * 64} if windows else None,
            "program_sha256": program_hash, "manifest": manifest(count, program_hash),
            "environment": diff.CHILD_ENV.copy(), "status": "captured", "errors": [],
            "returncode": 0, "timed_out": False, "stdout": stdout_for(items), "records": items}


class DifferentialTests(unittest.TestCase):
    def compare(self, reference, candidate):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            diff.write_json(root / "reference.json", reference)
            diff.write_json(root / "candidate.json", candidate)
            result = diff.compare_captures(root / "reference.json", root / "candidate.json", root / "report.json")
            self.assertEqual(result, diff.read_json(root / "report.json"))
            return result

    def test_complete_4096_balanced_records(self):
        items, errors = diff.parse_records(stdout_for(records(4096)), 4096)
        self.assertEqual(errors, [])
        self.assertEqual(len(items), 4096)
        for family in diff.FAMILIES:
            self.assertEqual(sum(item["family"] == family for item in items), 512)

    def test_registered_suites_pin_their_own_record_and_manifest_identity(self):
        for identifier in ("adapter-objc-v1", "adapter-thread-v1", "adapter-objc-metadata-v1", "adapter-objc-arc-o0-v1", "adapter-objc-arc-o2-v1"):
            suite = diff.SUITES[identifier]
            items = [{"id": f"{suite.families[i % 8]}:{i}", "family": suite.families[i % 8], "index": i,
                      "result": [0] * suite.result_lengths[suite.families[i % 8]]} for i in range(16)]
            parsed, errors = diff.parse_records(stdout_for(items), 16, suite)
            self.assertEqual((parsed, errors), (items, []), identifier)
            # The libc contract must not accept another suite's record vectors.
            self.assertTrue(diff.parse_records(stdout_for(items), 16)[1], identifier)
            manifest_value = {"schema": 1, "suite": suite.id, "count": 16, "seed": diff.SEED,
                              "families": list(suite.families),
                              "program": {"file": suite.program, "sha256": "a" * 64},
                              "source": {"file": Path(suite.source).name, "sha256": "b" * 64},
                              "build": {"compiler": {"sha256": "c" * 64}}}
            self.assertEqual(diff.manifest_errors(manifest_value), [], identifier)
            self.assertIn("invalid manifest source identity",
                          diff.manifest_errors({**manifest_value, "source": {"file": "libc_cases.c", "sha256": "b" * 64}}),
                          identifier)
        self.assertEqual(diff.manifest_errors({"schema": 1, "suite": "adapter-unknown-v1"}),
                         ["manifest schema/suite mismatch"])

    def test_same_program_complete_evidence_compares_equal(self):
        result = self.compare(capture(), capture(windows=True))
        self.assertEqual(result["status"], "equal")
        self.assertEqual(result["matched"], 8)

    def test_semantic_mismatch_is_reported(self):
        candidate = capture(windows=True)
        candidate["records"][4]["result"] = [123, 2, 34]
        candidate["stdout"] = stdout_for(candidate["records"])
        result = self.compare(capture(), candidate)
        self.assertEqual(result["status"], "different")
        self.assertEqual(result["mismatched"], 1)
        self.assertEqual(result["differences"][0]["id"], "strtol:4")

    def test_duplicates_and_missing_case_fail_even_when_counts_match(self):
        items = records()
        items[-1] = copy.deepcopy(items[0])
        _, errors = diff.parse_records(stdout_for(items), 8)
        self.assertTrue(any("duplicate" in error for error in errors))
        self.assertTrue(any("missing" in error for error in errors))
        candidate = capture(windows=True)
        candidate["records"], candidate["stdout"] = items, stdout_for(items)
        self.assertEqual(self.compare(capture(), candidate)["status"], "invalid")

    def test_extra_missing_reordered_and_boolean_results_fail(self):
        for items in (records()[:-1], records() + records()[:1], list(reversed(records()))):
            self.assertTrue(diff.parse_records(stdout_for(items), 8)[1])
        items = records()
        items[0]["result"] = [True]
        self.assertTrue(diff.parse_records(stdout_for(items), 8)[1])
        items[0]["result"] = [1.0]
        self.assertTrue(diff.parse_records(stdout_for(items), 8)[1])

    def test_program_hash_and_case_count_cannot_be_bypassed(self):
        for candidate in (capture(windows=True, program_hash="e" * 64), capture(16, windows=True)):
            self.assertEqual(self.compare(capture(), candidate)["status"], "invalid")

    def test_windows_reference_is_not_accepted(self):
        result = self.compare(capture(windows=True), capture(windows=True))
        self.assertEqual(result["status"], "invalid")
        self.assertTrue(any("native Darwin" in error for error in result["errors"]))

    def test_missing_reference_writes_invalid_report(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            diff.write_json(root / "candidate.json", capture(windows=True))
            result = diff.compare_captures(root / "missing.json", root / "candidate.json", root / "report.json")
            self.assertEqual(result["status"], "invalid")
            self.assertEqual(result["matched"], 0)
            self.assertTrue((root / "report.json").is_file())

    def test_tampered_records_and_duplicate_json_keys_fail(self):
        candidate = capture(windows=True)
        candidate["records"][0]["result"] = [9]
        self.assertEqual(self.compare(capture(), candidate)["status"], "invalid")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            diff.strict_json('{"id": 1, "id": 2}')
        with self.assertRaisesRegex(ValueError, "non-JSON"):
            diff.strict_json('{"result": NaN}')

    def test_malformed_manifest_is_a_failure_not_a_crash(self):
        for bad in (None, {}, {"schema": True}, {**manifest(), "count": True},
                    {**manifest(), "source": None}, {**manifest(), "build": []}):
            self.assertTrue(diff.manifest_errors(bad))
        bad = capture()
        del bad["manifest"]["source"]
        self.assertEqual(self.compare(bad, capture(windows=True))["status"], "invalid")

    def test_build_only_calls_compiler_and_never_runs_guest(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            compiler = root / "fake-clang"
            compiler.write_bytes(b"synthetic compiler identity")
            def compiler_run(command, **kwargs):
                self.assertEqual(command[0], str(compiler))
                if "--version" in command:
                    return subprocess.CompletedProcess(command, 0, "synthetic clang\n", "")
                output = Path(command[command.index("-o") + 1])
                output.write_bytes(struct.pack("<IIIIIIII", 0xFEEDFACF, 0x01000007, 3, 2, 0, 0, 0, 0))
                return subprocess.CompletedProcess(command, 0, "", "")
            with patch.object(diff.platform, "system", return_value="Darwin"), \
                 patch.object(diff.shutil, "which", return_value=str(compiler)), \
                 patch.object(diff.subprocess, "run", side_effect=compiler_run) as run:
                result = diff.build_bundle(root / "bundle", 8)
            self.assertEqual(run.call_count, 2)
            self.assertEqual(result["count"], 8)
            self.assertIn("-fno-builtin", result["build"]["command"])
            self.assertIn("-Wl,-no_uuid", result["build"]["command"])

    def make_bundle(self, root):
        program = root / diff.PROGRAM
        program.write_bytes(struct.pack("<IIIIIIII", 0xFEEDFACF, 0x01000007, 3, 2, 0, 0, 0, 0))
        diff.write_json(root / "manifest.json", manifest(program_hash=diff.sha256(program)))

    def test_explicit_capture_executes_once_and_validates_all_records(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.make_bundle(root)
            process = subprocess.CompletedProcess([], 0, stdout_for(records()).encode(), b"")
            with patch.object(diff, "host_metadata", return_value={"system": "Darwin"}), \
                 patch.object(diff.subprocess, "run", return_value=process) as run:
                result = diff.capture_bundle(root, root / "capture.json")
            self.assertEqual(result["status"], "captured")
            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.kwargs["env"]["TZ"], "UTC0")

    def test_capture_timeout_and_hash_tampering_never_pass(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.make_bundle(root)
            with patch.object(diff, "host_metadata", return_value={"system": "Darwin"}), \
                 patch.object(diff.subprocess, "run", side_effect=subprocess.TimeoutExpired([], 60)):
                result = diff.capture_bundle(root, root / "timeout.json")
            self.assertEqual(result["status"], "invalid")
            self.assertTrue(result["timed_out"])
            with (root / diff.PROGRAM).open("ab") as program:
                program.write(b"tamper")
            with patch.object(diff.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "SHA-256"):
                    diff.capture_bundle(root, root / "tampered.json")
                run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
