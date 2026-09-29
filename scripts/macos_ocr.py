#!/usr/bin/env python3
"""Single-window local OCR probe. No capture by default; never export text or image bytes."""
from __future__ import annotations
import argparse
import json
import re
import sys
import uuid
from macos_probe import ROOT, locate_probe, supervise, unique_object

BINARIES = [ROOT / "target/macos-probe/out/Products/Debug/foxbot-macos-ocr",
            ROOT / "target/macos-probe/debug/foxbot-macos-ocr"]
SUMMARY = {"OCR_SUMMARY", "OCR_PARTIAL_SUMMARY", "OCR_EMPTY"}
CAPTURE_SUMMARY = {"CAPTURE_SUMMARY"}
STATUS = SUMMARY | CAPTURE_SUMMARY | {"METADATA_ONLY", "NOT_RUNNING", "AMBIGUOUS_INSTANCE", "PERMISSION_REQUIRED",
                    "NO_ELIGIBLE_WINDOW", "AMBIGUOUS_WINDOW", "TARGET_CHANGED", "INVALID_GEOMETRY",
                    "RESOURCE_LIMIT", "ENUMERATION_FAILED", "CAPTURE_FAILED", "CAPTURE_SIZE_MISMATCH",
                    "OCR_FAILED", "LANGUAGE_UNAVAILABLE", "TIME_BUDGET_EXCEEDED", "ACCESSIBILITY_REQUIRED", "NO_FOCUSED_WINDOW"}
REASONS = {"LINE_LIMIT", "TEXT_LIMIT", "INVALID_OBSERVATION"}


def integer(value, low, high):
    return type(value) is int and low <= value <= high


def safe_ocr_report(data: bytes, app: str, capture: bool, focused: bool = False, ocr: bool = True) -> dict:
    if len(data) > 16384:
        raise ValueError("report size")
    report = json.loads(data, object_pairs_hook=unique_object)
    required = {"schema_version", "app", "bundle_id", "os_version", "snapshot_id", "read_only",
                "raw_text_included", "image_saved", "network_requests", "capture_scope", "content_scope", "selection_mode",
                "account_identity", "conversation_identity", "send_capability", "capture_requested",
                "ocr_requested", "screen_capture_preflight", "running_instances", "status", "capture_state", "ocr_attempted"}
    optional = {"application_version", "eligible_windows", "window_stable", "image", "ocr", "window_matching"}
    if not isinstance(report, dict) or not required <= report.keys() or report.keys() - required - optional:
        raise ValueError("report fields")
    expected = {"schema_version": "foxbot.macos-ocr.v1", "app": app,
                "bundle_id": {"wechat": "com.tencent.xinWeChat", "qq": "com.tencent.qq"}[app],
                "capture_scope": "SINGLE_WINDOW", "content_scope": "WINDOW_NOT_CHAT",
                "selection_mode": "FOCUSED_WINDOW" if focused else "UNIQUE_WINDOW",
                "account_identity": "UNVERIFIED", "conversation_identity": "UNVERIFIED",
                "send_capability": "NOT_IMPLEMENTED"}
    if any(report[key] != value for key, value in expected.items()):
        raise ValueError("bound identity")
    if (report["read_only"] is not True or report["raw_text_included"] is not False
            or report["image_saved"] is not False or report["capture_requested"] is not capture
            or report["ocr_requested"] is not (capture and ocr)
            or not integer(report["network_requests"], 0, 0)
            or type(report["screen_capture_preflight"]) is not bool or type(report["ocr_attempted"]) is not bool
            or not integer(report["running_instances"], 0, 1024)
            or report["status"] not in STATUS
            or report["capture_state"] not in {"NOT_ATTEMPTED", "UNKNOWN", "IMAGE_OBTAINED"}):
        raise ValueError("report invariant")
    if not isinstance(report["os_version"], str) or not re.fullmatch(r"[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}", report["os_version"]):
        raise ValueError("OS version")
    sid = report["snapshot_id"]
    if not isinstance(sid, str) or len(sid) != 36 or str(uuid.UUID(sid)).lower() != sid.lower():
        raise ValueError("snapshot")
    if "application_version" in report and (not isinstance(report["application_version"], str)
            or not re.fullmatch(r"[a-zA-Z0-9._-]{1,64}", report["application_version"])):
        raise ValueError("application version")
    if "eligible_windows" in report and not integer(report["eligible_windows"], 0, 4096):
        raise ValueError("window count")
    if "window_stable" in report and type(report["window_stable"]) is not bool:
        raise ValueError("window stability")
    instances, status, state = report["running_instances"], report["status"], report["capture_state"]
    gate = ("NOT_RUNNING" if instances == 0 else "AMBIGUOUS_INSTANCE" if instances > 1
            else "METADATA_ONLY" if not capture else "PERMISSION_REQUIRED" if not report["screen_capture_preflight"] else None)
    if gate:
        if (status != gate or state != "NOT_ATTEMPTED" or report["ocr_attempted"]
                or any(key in report for key in optional - {"application_version"})):
            raise ValueError("capture without prerequisites")
        return report
    if status in {"NOT_RUNNING", "AMBIGUOUS_INSTANCE", "METADATA_ONLY", "PERMISSION_REQUIRED"}:
        raise ValueError("contradictory gate")
    windows = report.get("eligible_windows")
    if status == "NO_ELIGIBLE_WINDOW" and windows != 0:
        raise ValueError("missing window count")
    if status == "AMBIGUOUS_WINDOW" and (windows is None or windows <= 1):
        raise ValueError("not ambiguous")
    if status in {"ACCESSIBILITY_REQUIRED", "NO_FOCUSED_WINDOW"} and (not focused or state != "NOT_ATTEMPTED"):
        raise ValueError("focused window gate")
    if "window_matching" in report:
        matching = report["window_matching"]
        keys = {"candidate_count", "origin_matches", "size_matches", "frame_matches"}
        if (not focused or not isinstance(matching, dict) or set(matching) != keys
                or not all(integer(matching[k], 0, 4096) for k in keys)
                or matching["frame_matches"] != windows
                or any(matching[k] > matching["candidate_count"] for k in keys - {"candidate_count"})
                or matching["frame_matches"] > min(matching["origin_matches"], matching["size_matches"])):
            raise ValueError("window matching summary")
    if state != "NOT_ATTEMPTED" and windows != 1:
        raise ValueError("capture must bind exactly one window")
    if status in {"NO_ELIGIBLE_WINDOW", "AMBIGUOUS_WINDOW", "INVALID_GEOMETRY", "RESOURCE_LIMIT", "ENUMERATION_FAILED"} and state != "NOT_ATTEMPTED":
        raise ValueError("capture after rejection")
    if state == "UNKNOWN" and status != "CAPTURE_FAILED":
        raise ValueError("unknown capture outcome")
    if status == "CAPTURE_FAILED" and state != "UNKNOWN":
        raise ValueError("lost capture result")
    if status == "CAPTURE_SIZE_MISMATCH" and state != "IMAGE_OBTAINED":
        raise ValueError("missing captured image")
    if "image" in report:
        image = report["image"]
        if (not isinstance(image, dict) or set(image) != {"width", "height", "downscaled"}
                or not integer(image["width"], 8, 4096) or not integer(image["height"], 8, 4096)
                or image["width"] * image["height"] > 8388608 or type(image["downscaled"]) is not bool
                or state != "IMAGE_OBTAINED"):
            raise ValueError("image metadata")
    if report["ocr_attempted"] and (state != "IMAGE_OBTAINED" or "image" not in report):
        raise ValueError("OCR without image")
    if status == "TARGET_CHANGED":
        if report.get("window_stable") is not False or "ocr" in report:
            raise ValueError("stale result")
    elif "window_stable" in report and status not in SUMMARY | CAPTURE_SUMMARY:
        raise ValueError("unsupported stability claim")
    if status == "CAPTURE_SUMMARY":
        if (state != "IMAGE_OBTAINED" or "image" not in report or report["ocr_attempted"]
                or "ocr" in report or report.get("window_stable") is not True or windows != 1
                or report["ocr_requested"] is not False):
            raise ValueError("missing capture-only evidence")
        return report
    if status not in SUMMARY:
        if "ocr" in report:
            raise ValueError("OCR payload on failure")
        if status in {"OCR_FAILED", "LANGUAGE_UNAVAILABLE"} and not report["ocr_attempted"]:
            raise ValueError("OCR was not attempted")
        return report
    if (state != "IMAGE_OBTAINED" or "image" not in report or not report["ocr_attempted"]
            or report.get("window_stable") is not True or windows != 1):
        raise ValueError("missing successful capture evidence")
    ocr = report.get("ocr")
    keys = {"line_count", "character_count", "low_confidence_lines", "partial_reasons",
            "complete_recognition", "engine", "request_revision", "language_correction"}
    if not isinstance(ocr, dict) or set(ocr) != keys:
        raise ValueError("OCR fields")
    if (not integer(ocr["line_count"], 0, 512) or not integer(ocr["character_count"], 0, 32768)
            or not integer(ocr["low_confidence_lines"], 0, ocr["line_count"])
            or ocr["character_count"] < ocr["line_count"] or (ocr["line_count"] == 0 and ocr["character_count"] != 0)
            or ocr["engine"] != "APPLE_VISION" or not integer(ocr["request_revision"], 3, 3)
            or ocr["language_correction"] is not False or type(ocr["complete_recognition"]) is not bool):
        raise ValueError("OCR invariants")
    reasons = ocr["partial_reasons"]
    if (not isinstance(reasons, list) or len(reasons) > len(REASONS)
            or any(not isinstance(reason, str) or reason not in REASONS for reason in reasons)
            or len(set(reasons)) != len(reasons) or ocr["complete_recognition"] != (not reasons)):
        raise ValueError("partial reasons")
    expected_status = "OCR_PARTIAL_SUMMARY" if reasons else "OCR_EMPTY" if ocr["line_count"] == 0 else "OCR_SUMMARY"
    if status != expected_status:
        raise ValueError("contradictory OCR status")
    return report


def failure_report(status: str, app: str, capture: bool, started: bool) -> dict:
    # A timeout/invalid report after process start is NOT proof that no image was captured.
    return {"status": status, "app": app, "read_only": True, "raw_text_included": False, "image_saved": False,
            "capture_state": "UNKNOWN" if capture and started else "NOT_ATTEMPTED",
            "ocr_state": "UNKNOWN" if capture and started else "NOT_ATTEMPTED"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", choices=["wechat", "qq"], required=True)
    parser.add_argument("--capture-and-ocr", action="store_true", help="capture exactly one verified target window and run local OCR; no whole-screen fallback")
    parser.add_argument("--capture-only", action="store_true", help="capture exactly one verified target window without running OCR")
    parser.add_argument("--focused-window", action="store_true", help="explicitly bind the application's existing AX focused standard window; never activate it")
    parser.add_argument("--timeout-seconds", type=int, choices=range(1, 31), default=15)
    args = parser.parse_args()
    if args.capture_and_ocr and args.capture_only:
        parser.error("choose only one capture mode")
    capture = args.capture_and_ocr or args.capture_only
    if args.focused_window and not capture:
        parser.error("--focused-window requires a capture mode")
    status, binary = locate_probe(candidates=BINARIES)
    data, started = b"", False
    if sys.platform != "darwin":
        status = "UNSUPPORTED_PLATFORM"
    elif status == "READY":
        command = [str(binary), "--app", args.app]
        if args.capture_and_ocr:
            command.append("--capture-and-ocr")
        elif args.capture_only:
            command.append("--capture-only")
        if args.focused_window:
            command.append("--focused-window")
        try:
            started = True
            status, data = supervise(command, timeout=args.timeout_seconds)
        except OSError:
            status = "PROCESS_FAILED"
    if status == "OK":
        try:
            report = safe_ocr_report(data, args.app, capture, args.focused_window, args.capture_and_ocr)
        except (ValueError, KeyError, TypeError, AttributeError, RecursionError, OverflowError):
            status = "INVALID_REPORT"
        else:
            print(json.dumps(report, ensure_ascii=False, indent=2))
            return 0  # A negative capability observation is valid, not an acceptance PASS.
    print(json.dumps(failure_report(status, args.app, capture, started)))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
