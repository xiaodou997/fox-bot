#!/usr/bin/env python3
"""Supervise the fixed read-only macOS probe. No build, permission prompt or raw-text output."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
BINARIES = [ROOT / "target/macos-probe/out/Products/Debug/foxbot-macos-probe",
            ROOT / "target/macos-probe/debug/foxbot-macos-probe"]
STATES = {"NOT_READ", "UNAVAILABLE", "EMPTY", "NONEMPTY", "PROTECTED", "AMBIGUOUS"}
STATUSES = {"METADATA_ONLY", "NOT_RUNNING", "AMBIGUOUS_INSTANCE", "PERMISSION_REQUIRED",
            "NO_READABLE_FOCUSED_WINDOW", "WINDOW_CHANGED", "AX_SUMMARY", "AX_PARTIAL_SUMMARY"}
COUNTS = {"visited_nodes", "static_text_nodes", "readable_static_text_nodes", "protected_nodes",
          "message_candidates", "readable_message_candidates", "editor_candidates", "read_errors"}
REASONS = {"DEADLINE", "NODE_LIMIT", "CHILD_LIMIT", "DEPTH_LIMIT", "REPEATED_NODE", "READ_ERROR"}


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate JSON key")
        value[key] = item
    return value


def safe_report(data: bytes, app: str, allow_read: bool) -> dict:
    """Closed schema prevents a future accidental native text field leaking to diagnostics."""
    if len(data) > 16384:
        raise ValueError("report size")
    report = json.loads(data, object_pairs_hook=unique_object)
    fields = {"schema_version", "app", "bundle_id", "os_version", "application_version", "snapshot_id",
              "read_only", "raw_text_included", "screenshot_taken", "network_requests", "ax_read_requested",
              "accessibility_trusted", "screen_capture_preflight", "running_instances", "status",
              "focused_window_title_state", "window_stable", "tree", "account_identity",
              "conversation_identity", "write_capability"}
    optional = {"application_version", "window_stable", "tree"}
    if not isinstance(report, dict) or not fields - optional <= report.keys() or report.keys() - fields:
        raise ValueError("schema")
    expected_bundle = {"qq": "com.tencent.qq", "wechat": "com.tencent.xinWeChat"}[app]
    if (report["schema_version"] != "foxbot.macos-probe.v1" or report["app"] != app
            or report["bundle_id"] != expected_bundle or report["read_only"] is not True
            or report["raw_text_included"] is not False or report["screenshot_taken"] is not False
            or type(report["network_requests"]) is not int or report["network_requests"] != 0
            or report["ax_read_requested"] is not allow_read
            or report["account_identity"] != "UNVERIFIED" or report["conversation_identity"] != "UNVERIFIED"
            or report["write_capability"] != "NOT_IMPLEMENTED" or report["status"] not in STATUSES
            or report["focused_window_title_state"] not in STATES):
        raise ValueError("invariant")
    for flag in ("accessibility_trusted", "screen_capture_preflight"):
        if type(report[flag]) is not bool:
            raise ValueError("flag")
    if not re.fullmatch(r"[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}", report["os_version"]):
        raise ValueError("version")
    if len(report["snapshot_id"]) != 36 or str(uuid.UUID(report["snapshot_id"])).upper() != report["snapshot_id"].upper():
        raise ValueError("snapshot")
    if type(report["running_instances"]) is not int or not 0 <= report["running_instances"] <= 1024:
        raise ValueError("count")
    if "application_version" in report and not re.fullmatch(r"[a-zA-Z0-9._-]{1,64}", report["application_version"]):
        raise ValueError("application version")
    if "window_stable" in report and type(report["window_stable"]) is not bool:
        raise ValueError("window flag")
    if "tree" in report:
        tree = report["tree"]
        allowed = COUNTS | {"editor_state", "partial_reasons", "complete_traversal", "account_identity",
                            "conversation_identity", "send_capability", "screenshot_taken", "raw_text_included"}
        if not allow_read or not isinstance(tree, dict) or set(tree) != allowed:
            raise ValueError("tree schema")
        if any(type(tree[k]) is not int or not 0 <= tree[k] <= 8192 for k in COUNTS):
            raise ValueError("tree count")
        if (tree["editor_state"] not in STATES or tree["account_identity"] != "UNVERIFIED"
                or tree["conversation_identity"] != "UNVERIFIED" or tree["send_capability"] != "NOT_IMPLEMENTED"
                or tree["screenshot_taken"] is not False or tree["raw_text_included"] is not False
                or type(tree["complete_traversal"]) is not bool or not isinstance(tree["partial_reasons"], list)
                or len(tree["partial_reasons"]) > len(REASONS)
                or any(reason not in REASONS for reason in tree["partial_reasons"])):
            raise ValueError("tree invariant")
    status = report["status"]
    instances = report["running_instances"]
    has_tree = "tree" in report
    if status == "NOT_RUNNING":
        valid_gate = instances == 0
    elif status == "AMBIGUOUS_INSTANCE":
        valid_gate = instances > 1
    elif status == "METADATA_ONLY":
        valid_gate = instances == 1 and not allow_read
    elif status == "PERMISSION_REQUIRED":
        valid_gate = instances == 1 and allow_read and not report["accessibility_trusted"]
    else:
        valid_gate = instances == 1 and allow_read and report["accessibility_trusted"]
    if not valid_gate:
        raise ValueError("contradictory gate")
    if status in {"AX_SUMMARY", "AX_PARTIAL_SUMMARY"}:
        if not has_tree or report.get("window_stable") is not True:
            raise ValueError("missing stable summary")
        tree = report["tree"]
        reasons = tree["partial_reasons"]
        visited = tree["visited_nodes"]
        if (visited > 512 or len(set(reasons)) != len(reasons)
                or tree["complete_traversal"] != (len(reasons) == 0)
                or tree["complete_traversal"] != (status == "AX_SUMMARY")
                or tree["readable_static_text_nodes"] > tree["static_text_nodes"]
                or tree["readable_message_candidates"] > tree["message_candidates"]
                or any(tree[k] > visited for k in ("static_text_nodes", "protected_nodes", "message_candidates", "editor_candidates"))
                or tree["read_errors"] > visited * 2
                or (tree["read_errors"] > 0) != ("READ_ERROR" in reasons)):
            raise ValueError("contradictory summary")
        editors = tree["editor_candidates"]
        editor = tree["editor_state"]
        if (editors == 0 and editor != "NOT_READ") or (editors > 1 and editor != "AMBIGUOUS"):
            raise ValueError("contradictory editor")
        if reasons and editor in {"EMPTY", "NONEMPTY"}:
            raise ValueError("partial editor")
    elif status == "WINDOW_CHANGED":
        if has_tree or report.get("window_stable") is not False:
            raise ValueError("stale summary")
    elif has_tree or "window_stable" in report or report["focused_window_title_state"] != "NOT_READ":
        raise ValueError("unexpected AX read")
    return report


def supervise(command: list[str], timeout: float = 8.0, max_bytes: int = 16384) -> tuple[str, bytes]:
    """Own one process group; bound stdout and kill+wait before returning on failure."""
    child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL, start_new_session=True)
    data = bytearray()
    deadline = time.monotonic() + timeout
    eof = False
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ)
            eof = False
            while not eof:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return "TIMEOUT", b""
                for key, _ in selector.select(min(remaining, 0.1)):
                    chunk = os.read(key.fileobj.fileno(), min(4096, max_bytes + 1 - len(data)))
                    if not chunk:
                        eof = True
                        break
                    data.extend(chunk)
                    if len(data) > max_bytes:
                        return "OUTPUT_LIMIT", b""
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                return "TIMEOUT", b""
            try:
                code = child.wait(timeout=remaining)
            except subprocess.TimeoutExpired:
                return "TIMEOUT", b""
            return ("OK", bytes(data)) if code == 0 else ("PROCESS_FAILED", b"")
    finally:
        if child.poll() is None or not eof:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        child.wait()
        child.stdout.close()


def locate_probe(candidates=None, scratch=None):
    candidates = BINARIES if candidates is None else candidates
    scratch = (ROOT / "target/macos-probe" if scratch is None else scratch).resolve()
    found = set()
    for path in candidates:
        if path.is_file():
            resolved = path.resolve()
            try:
                resolved.relative_to(scratch)
            except ValueError:
                return "UNSAFE_BUILD_PATH", None
            found.add(resolved)
    if len(found) != 1:
        return ("BUILD_REQUIRED" if not found else "AMBIGUOUS_BUILD"), None
    return "READY", found.pop()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", choices=["qq", "wechat"], required=True)
    parser.add_argument("--allow-ax-read", action="store_true")
    parser.add_argument("--timeout-seconds", type=int, choices=range(1, 21), default=8)
    args = parser.parse_args()
    status, binary = locate_probe()
    data = b""
    if sys.platform != "darwin":
        status = "UNSUPPORTED_PLATFORM"
    elif status == "READY":
        command = [str(binary), "--app", args.app]
        if args.allow_ax_read:
            command.append("--allow-ax-read")
        try:
            status, data = supervise(command, args.timeout_seconds)
        except OSError:
            status, data = "PROCESS_FAILED", b""
    if status == "OK":
        try:
            report = safe_report(data, args.app, args.allow_ax_read)
        except (ValueError, TypeError, KeyError, AttributeError, RecursionError):
            status = "INVALID_REPORT"
        else:
            print(json.dumps(report, ensure_ascii=False, indent=2))
            return 0
    print(json.dumps({"status": status, "app": args.app, "read_only": True,
                      "raw_text_included": False, "screenshot_taken": False}))
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
