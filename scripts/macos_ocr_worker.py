#!/usr/bin/env python3
"""Supervise one persistent local Vision worker. Replies never contain OCR text or image bytes."""
from __future__ import annotations
import argparse
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import time
from macos_probe import locate_probe, unique_object
from macos_ocr import BINARIES, safe_ocr_report

MAX_COMMAND_BYTES = 4096
MAX_REPLY_BYTES = 16384


class WorkerProcess:
    def __init__(self, command: list[str]):
        self.process = subprocess.Popen(
            command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            start_new_session=True
        )
        self.buffer = bytearray()

    def _terminate(self):
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except PermissionError:
                # Some supervised/test environments deny process-group signalling even
                # though the direct child is still ours. Reap the child rather than leak it.
                try:
                    self.process.kill()
                except ProcessLookupError:
                    pass
        self.process.wait()

    def request(self, payload: dict, timeout: float) -> bytes:
        if self.process.poll() is not None:
            raise RuntimeError("worker exited")
        data = json.dumps(payload, ensure_ascii=True, separators=(",", ":")).encode()
        if len(data) > MAX_COMMAND_BYTES:
            raise ValueError("command too large")
        assert self.process.stdin is not None and self.process.stdout is not None
        self.process.stdin.write(data + b"\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + timeout
        with selectors.DefaultSelector() as selector:
            selector.register(self.process.stdout, selectors.EVENT_READ)
            while True:
                newline = self.buffer.find(b"\n")
                if newline >= 0:
                    result = bytes(self.buffer[:newline])
                    del self.buffer[:newline + 1]
                    return result
                if len(self.buffer) > MAX_REPLY_BYTES:
                    self._terminate()
                    raise ValueError("reply too large")
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    self._terminate()
                    raise TimeoutError("worker request timed out")
                events = selector.select(min(remaining, 0.1))
                if not events:
                    if self.process.poll() is not None:
                        raise RuntimeError("worker exited")
                    continue
                chunk = os.read(self.process.stdout.fileno(), 4096)
                if not chunk:
                    raise RuntimeError("worker closed stdout")
                self.buffer.extend(chunk)

    def shutdown(self):
        if self.process.poll() is None:
            try:
                self.request({"id": "shutdown", "command": "shutdown"}, 2)
            except Exception:
                self._terminate()
        if self.process.poll() is None:
            self._terminate()
        if self.process.stdin:
            self.process.stdin.close()
        if self.process.stdout:
            self.process.stdout.close()


def safe_worker_reply(data: bytes, request_id: str, action: str,
                      app: str | None = None, focused: bool = False) -> dict:
    if len(data) > MAX_REPLY_BYTES:
        raise ValueError("worker reply size")
    reply = json.loads(data, object_pairs_hook=unique_object)
    if not isinstance(reply, dict) or set(reply) - {"id", "status", "warmup", "report"}:
        raise ValueError("worker fields")
    if reply.get("id") != request_id or reply.get("status") not in {
        "WARMED", "WARMUP_FAILED", "NOT_WARMED", "INVALID_REQUEST", "REPORT", "SHUTDOWN"
    }:
        raise ValueError("worker identity")
    if reply["status"] in {"WARMED", "WARMUP_FAILED"}:
        warmup = reply.get("warmup")
        if (set(reply) != {"id", "status", "warmup"} or not isinstance(warmup, dict)
                or set(warmup) != {"elapsed_milliseconds", "line_count", "succeeded"}
                or type(warmup["elapsed_milliseconds"]) is not int or not 0 <= warmup["elapsed_milliseconds"] <= 300_000
                or type(warmup["line_count"]) is not int or not 0 <= warmup["line_count"] <= 512
                or type(warmup["succeeded"]) is not bool
                or warmup["succeeded"] != (reply["status"] == "WARMED")):
            raise ValueError("warmup reply")
    elif reply["status"] == "REPORT":
        if set(reply) != {"id", "status", "report"} or app is None:
            raise ValueError("report envelope")
        safe_ocr_report(json.dumps(reply["report"]).encode(), app, True, focused,
                        action == "capture_ocr")
    elif set(reply) != {"id", "status"}:
        raise ValueError("unexpected worker payload")
    return reply


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", choices=["wechat", "qq"], required=True)
    parser.add_argument("--focused-window", action="store_true")
    parser.add_argument("--repeat", type=int, choices=range(1, 6), default=2)
    parser.add_argument("--warmup-timeout-seconds", type=int, choices=range(5, 91), default=60)
    parser.add_argument("--request-timeout-seconds", type=int, choices=range(2, 31), default=15)
    args = parser.parse_args()
    if sys.platform != "darwin":
        print(json.dumps({"status": "UNSUPPORTED_PLATFORM", "raw_text_included": False}))
        return 2
    status, binary = locate_probe(candidates=BINARIES)
    if status != "READY":
        print(json.dumps({"status": status, "raw_text_included": False}))
        return 2

    worker = WorkerProcess([str(binary), "--worker"])
    runs = []
    request_elapsed = []
    try:
        warm_raw = worker.request({"id": "warmup", "command": "warmup"}, args.warmup_timeout_seconds)
        warm = safe_worker_reply(warm_raw, "warmup", "warmup")
        if warm["status"] != "WARMED":
            print(json.dumps({"status": warm["status"], "warmup": warm.get("warmup"),
                              "raw_text_included": False, "image_saved": False}))
            return 2
        for index in range(args.repeat):
            request_id = f"capture_{index + 1}"
            payload = {"id": request_id, "command": "capture_ocr",
                       "app": args.app, "focused_window": args.focused_window}
            started = time.monotonic()
            raw = worker.request(payload, args.request_timeout_seconds)
            request_elapsed.append(max(0, int(round((time.monotonic() - started) * 1000))))
            reply = safe_worker_reply(raw, request_id, "capture_ocr", args.app, args.focused_window)
            if reply["status"] != "REPORT":
                raise ValueError("worker did not return report")
            runs.append(reply["report"])
        result = {
            "schema_version": "foxbot.ocr-worker.v1",
            "status": "OK",
            "worker_reused": args.repeat > 1,
            "warmup": warm["warmup"],
            "runs": runs,
            "request_elapsed_milliseconds": request_elapsed,
            "raw_text_included": False,
            "image_saved": False,
            "network_requests": 0
        }
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0
    except TimeoutError:
        print(json.dumps({"status": "TIMEOUT", "raw_text_included": False,
                          "image_saved": False, "capture_state": "UNKNOWN"}))
        return 2
    except (OSError, RuntimeError, ValueError, TypeError, KeyError, json.JSONDecodeError):
        print(json.dumps({"status": "WORKER_FAILED", "raw_text_included": False,
                          "image_saved": False, "capture_state": "UNKNOWN"}))
        return 2
    finally:
        worker.shutdown()


if __name__ == "__main__":
    raise SystemExit(main())
