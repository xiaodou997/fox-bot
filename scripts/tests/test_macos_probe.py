import json
import tempfile
from pathlib import Path
import sys
import time
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from macos_probe import safe_report, supervise, locate_probe


def metadata():
    return {"schema_version":"foxbot.macos-probe.v1", "app":"qq", "bundle_id":"com.tencent.qq",
            "os_version":"27.0.0", "snapshot_id":"00000000-0000-4000-8000-000000000001",
            "read_only":True, "raw_text_included":False, "screenshot_taken":False, "network_requests":0,
            "ax_read_requested":False, "accessibility_trusted":False, "screen_capture_preflight":False,
            "running_instances":0, "status":"NOT_RUNNING", "focused_window_title_state":"NOT_READ",
            "account_identity":"UNVERIFIED", "conversation_identity":"UNVERIFIED", "write_capability":"NOT_IMPLEMENTED"}


def summary_report():
    report = metadata()
    report.update(ax_read_requested=True, accessibility_trusted=True, running_instances=1,
                  status="AX_SUMMARY", window_stable=True, focused_window_title_state="NONEMPTY")
    report["tree"] = {
        "visited_nodes": 2, "static_text_nodes": 1, "readable_static_text_nodes": 1,
        "protected_nodes": 0, "message_candidates": 1, "readable_message_candidates": 1,
        "editor_candidates": 1, "read_errors": 0, "editor_state": "EMPTY",
        "partial_reasons": [], "complete_traversal": True,
        "account_identity": "UNVERIFIED", "conversation_identity": "UNVERIFIED",
        "send_capability": "NOT_IMPLEMENTED", "screenshot_taken": False, "raw_text_included": False,
    }
    return report


class ProbeSupervisorTests(unittest.TestCase):
    def test_native_sources_contain_no_input_capture_or_network_actions(self):
        sources = Path(__file__).resolve().parents[2] / "native/macos-probe/Sources"
        # G2a remains capture-free. The separate OCRCLI/OCRKit product has its own safety checks.
        text = "\n".join(path.read_text() for name in ("ProbeCLI", "ProbeKit")
                         for path in (sources / name).rglob("*.swift"))
        forbidden = ["AXUIElementPerformAction", "AXUIElementSetAttributeValue", "AXUIElementPostKeyboardEvent",
                     "CGEventPost", "CGRequestScreenCaptureAccess", "AXIsProcessTrustedWithOptions",
                     "NSPasteboard", "URLSession", "SCStream", "SCScreenshotManager", "AXManualAccessibility"]
        for name in forbidden:
            self.assertNotIn(name, text)

    def test_metadata_report_is_accepted(self):
        self.assertEqual(safe_report(json.dumps(metadata()).encode(), "qq", False), metadata())

    def test_raw_text_extension_is_rejected_not_printed(self):
        report = metadata(); report["chat_text"] = "synthetic private content"
        with self.assertRaises(ValueError):
            safe_report(json.dumps(report).encode(), "qq", False)

    def test_target_and_permission_mode_are_bound(self):
        for app, allow in [("wechat", False), ("qq", True)]:
            with self.assertRaises(ValueError):
                safe_report(json.dumps(metadata()).encode(), app, allow)

    def test_text_disguised_as_status_is_rejected(self):
        report = metadata(); report["status"] = "synthetic private content"
        with self.assertRaises(ValueError):
            safe_report(json.dumps(report).encode(), "qq", False)

    def test_duplicate_json_fields_are_rejected(self):
        data = json.dumps(metadata()).encode()
        with self.assertRaises(ValueError):
            safe_report(b'{"status":"AX_SUMMARY",' + data[1:], "qq", False)

    def test_report_size_is_bounded_even_without_supervisor(self):
        with self.assertRaises(ValueError):
            safe_report(b" " * 20000, "qq", False)

    def test_valid_complete_and_partial_summaries(self):
        report = summary_report()
        self.assertEqual(safe_report(json.dumps(report).encode(), "qq", True), report)
        report["status"] = "AX_PARTIAL_SUMMARY"
        report["tree"].update(partial_reasons=["CHILD_LIMIT"], complete_traversal=False, editor_state="AMBIGUOUS")
        self.assertEqual(safe_report(json.dumps(report).encode(), "qq", True), report)

    def test_tree_cannot_claim_read_without_gate(self):
        for changes in ({"accessibility_trusted": False}, {"running_instances": 0},
                        {"status": "PERMISSION_REQUIRED"}, {"window_stable": False}):
            report = summary_report(); report.update(changes)
            with self.assertRaises(ValueError):
                safe_report(json.dumps(report).encode(), "qq", True)

    def test_counts_and_editor_states_cannot_contradict(self):
        changes = [{"readable_static_text_nodes": 3}, {"editor_candidates": 0},
                   {"editor_candidates": 2}, {"visited_nodes": 513}, {"read_errors": 1},
                   {"partial_reasons": ["DEADLINE"]}]
        for change in changes:
            report = summary_report(); report["tree"].update(change)
            with self.assertRaises(ValueError):
                safe_report(json.dumps(report).encode(), "qq", True)

    def test_partial_tree_cannot_export_empty_draft(self):
        report = summary_report(); report["status"] = "AX_PARTIAL_SUMMARY"
        report["tree"].update(partial_reasons=["NODE_LIMIT"], complete_traversal=False)
        with self.assertRaises(ValueError):
            safe_report(json.dumps(report).encode(), "qq", True)

    def test_window_change_discards_tree(self):
        report = summary_report(); report.update(status="WINDOW_CHANGED", window_stable=False)
        with self.assertRaises(ValueError):
            safe_report(json.dumps(report).encode(), "qq", True)
        del report["tree"]
        self.assertEqual(safe_report(json.dumps(report).encode(), "qq", True), report)

    def test_build_resolution_rejects_ambiguity_and_outside_symlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); scratch = root / "scratch"; scratch.mkdir()
            one = scratch / "one"; two = scratch / "two"
            self.assertEqual(locate_probe([one], scratch)[0], "BUILD_REQUIRED")
            one.write_bytes(b"fixture")
            self.assertEqual(locate_probe([one], scratch)[1], one.resolve())
            two.write_bytes(b"fixture2")
            self.assertEqual(locate_probe([one, two], scratch)[0], "AMBIGUOUS_BUILD")
            two.unlink(); two.symlink_to(root / "outside")
            (root / "outside").write_bytes(b"not a probe")
            self.assertEqual(locate_probe([two], scratch)[0], "UNSAFE_BUILD_PATH")

    def test_native_failure_discards_output(self):
        status, data = supervise([sys.executable, "-c", "print('synthetic private'); raise SystemExit(2)"])
        self.assertEqual((status, data), ("PROCESS_FAILED", b""))

    def test_stderr_is_not_forwarded(self):
        status, data = supervise([sys.executable, "-c", "import sys; print('sensitive',file=sys.stderr); print('{}')"])
        self.assertEqual((status, data), ("OK", b"{}\n"))

    def test_output_cap_discards_partial_payload(self):
        status, data = supervise([sys.executable, "-c", "print('x'*20000)"], max_bytes=1024)
        self.assertEqual((status, data), ("OUTPUT_LIMIT", b""))

    def test_timeout_kills_and_waits_for_child(self):
        start = time.monotonic()
        status, data = supervise([sys.executable, "-c", "import time; time.sleep(30)"], timeout=0.15)
        self.assertEqual((status, data), ("TIMEOUT", b""))
        self.assertLess(time.monotonic() - start, 5)

    def test_eof_without_process_exit_still_times_out(self):
        status, _ = supervise([sys.executable, "-c", "import os,time; os.close(1); time.sleep(30)"], timeout=0.15)
        self.assertEqual(status, "TIMEOUT")


if __name__ == "__main__":
    unittest.main()
