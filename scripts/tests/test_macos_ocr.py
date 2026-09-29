import json
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from macos_ocr import safe_ocr_report, failure_report


def report():
    return {"schema_version": "foxbot.macos-ocr.v1", "app": "wechat", "bundle_id": "com.tencent.xinWeChat",
            "os_version": "27.0.0", "snapshot_id": "00000000-0000-4000-8000-000000000001",
            "read_only": True, "raw_text_included": False, "image_saved": False, "network_requests": 0,
            "capture_scope": "SINGLE_WINDOW", "content_scope": "WINDOW_NOT_CHAT", "selection_mode": "UNIQUE_WINDOW",
            "account_identity": "UNVERIFIED", "conversation_identity": "UNVERIFIED", "send_capability": "NOT_IMPLEMENTED",
            "capture_requested": True, "ocr_requested": True, "screen_capture_preflight": True, "running_instances": 1,
            "eligible_windows": 1, "status": "OCR_SUMMARY", "capture_state": "IMAGE_OBTAINED", "ocr_attempted": True,
            "window_stable": True, "image": {"width": 1200, "height": 700, "downscaled": False},
            "ocr": {"line_count": 2, "character_count": 20, "low_confidence_lines": 1, "partial_reasons": [],
                    "complete_recognition": True, "engine": "APPLE_VISION", "request_revision": 3, "language_correction": False}}


class WindowOCRSchemaTests(unittest.TestCase):
    def verify(self, value):
        return safe_ocr_report(json.dumps(value).encode(), "wechat", True)

    def test_valid_summary_empty_and_partial_are_distinct(self):
        self.verify(report())
        empty = report(); empty["ocr"].update(line_count=0, character_count=0, low_confidence_lines=0); empty["status"] = "OCR_EMPTY"
        self.verify(empty)
        partial = report(); partial["ocr"].update(complete_recognition=False, partial_reasons=["LINE_LIMIT"])
        partial["status"] = "OCR_PARTIAL_SUMMARY"; self.verify(partial)

    def test_raw_text_cannot_be_added_to_report_or_nested_results(self):
        for location in (None, "image", "ocr"):
            value = report(); (value if location is None else value[location])["text"] = "SYNTHETIC_PRIVATE"
            with self.assertRaises(ValueError): self.verify(value)

    def test_duplicate_keys_and_large_reports_are_rejected(self):
        raw = json.dumps(report()).encode()
        with self.assertRaises(ValueError): safe_ocr_report(b'{"status":"OCR_EMPTY",' + raw[1:], "wechat", True)
        with self.assertRaises(ValueError): safe_ocr_report(b" " * 17000 + raw, "wechat", True)

    def test_unrequested_capture_and_other_app_are_rejected(self):
        raw = json.dumps(report()).encode()
        for app, capture in [("wechat", False), ("qq", True)]:
            with self.assertRaises(ValueError): safe_ocr_report(raw, app, capture)
        for key, value in [("screen_capture_preflight", False), ("running_instances", 2), ("eligible_windows", 2)]:
            item = report(); item[key] = value
            with self.assertRaises(ValueError): self.verify(item)

    def test_default_metadata_cannot_contain_capture_evidence(self):
        item = report()
        for key in ("eligible_windows", "window_stable", "image", "ocr"): item.pop(key)
        item.update(status="METADATA_ONLY", capture_requested=False, ocr_requested=False,
                    capture_state="NOT_ATTEMPTED", ocr_attempted=False)
        safe_ocr_report(json.dumps(item).encode(), "wechat", False, ocr=False)
        item["capture_state"] = "IMAGE_OBTAINED"
        with self.assertRaises(ValueError): safe_ocr_report(json.dumps(item).encode(), "wechat", False)

    def test_no_raw_payload_disguised_as_status_or_ocr_reason(self):
        for field in ("status", "capture_state"):
            item = report(); item[field] = "SYNTHETIC_PRIVATE"
            with self.assertRaises(ValueError): self.verify(item)
        item = report(); item["ocr"]["partial_reasons"] = ["SYNTHETIC_PRIVATE"]
        with self.assertRaises(ValueError): self.verify(item)

    def test_image_and_ocr_budgets_reject_bool_or_huge_counts(self):
        for part, key, value in [("image", "width", 5000), ("image", "height", True),
                                 ("ocr", "line_count", 513), ("ocr", "character_count", 99999),
                                 ("ocr", "low_confidence_lines", 3), ("ocr", "request_revision", True)]:
            item = report(); item[part][key] = value
            with self.assertRaises(ValueError): self.verify(item)
        item = report(); item["image"].update(width=4096, height=4096)
        with self.assertRaises(ValueError): self.verify(item)

    def test_stale_target_must_discard_ocr_and_unknown_is_not_false(self):
        item = report(); item.update(status="TARGET_CHANGED", window_stable=False)
        with self.assertRaises(ValueError): self.verify(item)
        item.pop("ocr"); self.verify(item)
        item = report(); item.update(status="CAPTURE_FAILED", capture_state="UNKNOWN", ocr_attempted=False)
        for key in ("image", "ocr", "window_stable"): item.pop(key)
        self.verify(item)
        item["capture_state"] = "NOT_ATTEMPTED"
        with self.assertRaises(ValueError): self.verify(item)

    def test_focused_mode_is_explicit_and_cannot_be_silently_substituted(self):
        item = report(); item["selection_mode"] = "FOCUSED_WINDOW"
        with self.assertRaises(ValueError): self.verify(item)
        safe_ocr_report(json.dumps(item).encode(), "wechat", True, True)

    def test_capture_only_has_image_evidence_without_ocr(self):
        item = report()
        item.pop("ocr")
        item.update(status="CAPTURE_SUMMARY", ocr_requested=False, ocr_attempted=False)
        safe_ocr_report(json.dumps(item).encode(), "wechat", True, False, False)
        item["ocr_attempted"] = True
        with self.assertRaises(ValueError):
            safe_ocr_report(json.dumps(item).encode(), "wechat", True, False, False)

    def test_matching_diagnostics_do_not_allow_raw_geometry_or_contradictory_counts(self):
        item = report(); item["selection_mode"] = "FOCUSED_WINDOW"
        item["window_matching"] = {"candidate_count": 2, "origin_matches": 1, "size_matches": 1, "frame_matches": 1}
        safe_ocr_report(json.dumps(item).encode(), "wechat", True, True)
        for key, value in [("coordinates", [1, 2]), ("frame_matches", 3), ("size_matches", 0)]:
            changed = json.loads(json.dumps(item)); changed["window_matching"][key] = value
            with self.assertRaises(ValueError): safe_ocr_report(json.dumps(changed).encode(), "wechat", True, True)

    def test_timeout_cannot_claim_no_screenshot(self):
        self.assertEqual(failure_report("TIMEOUT", "wechat", True, True)["capture_state"], "UNKNOWN")
        self.assertEqual(failure_report("BUILD_REQUIRED", "wechat", True, False)["capture_state"], "NOT_ATTEMPTED")
        self.assertEqual(failure_report("INVALID_REPORT", "wechat", False, True)["ocr_state"], "NOT_ATTEMPTED")

    def test_false_complete_and_invented_identity_are_rejected(self):
        for mutate in [lambda r: r.update(conversation_identity="VERIFIED"),
                       lambda r: r.update(send_capability="READY"),
                       lambda r: r["ocr"].update(complete_recognition=False),
                       lambda r: r.update(status="OCR_EMPTY")]:
            item = report(); mutate(item)
            with self.assertRaises(ValueError): self.verify(item)

    def test_capture_sources_have_no_write_upload_or_display_fallback(self):
        root = Path(__file__).resolve().parents[2] / "native/macos-probe/Sources"
        text = "\n".join(path.read_text() for name in ("OCRKit", "OCRCLI") for path in (root / name).rglob("*.swift"))
        for forbidden in ("AXUIElementPerformAction", "AXUIElementSetAttributeValue", "CGEventPost", "NSPasteboard",
                          "URLSession", "CGRequestScreenCaptureAccess", "CGWindowListCreateImage", "SCStream(",
                          "SCContentFilter(display:", "captureImage(in:", "CGImageDestination", "write(to:"):
            self.assertNotIn(forbidden, text)
        self.assertIn("SCContentFilter(desktopIndependentWindow:", text)
        self.assertIn("config.includeChildWindows = false", text)
        self.assertIn("NSApplication.shared", text)
        self.assertIn("application.setActivationPolicy(.prohibited)", text)


if __name__ == "__main__":
    unittest.main()
