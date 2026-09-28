#!/usr/bin/env python3
"""Run the built HTTP CLI against a loopback-only synthetic service; no external model.

Usage: python3 scripts/http_smoke.py [path/to/foxbot-http]
The server and temporary state are stopped/removed before returning.
"""
from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    binary = Path(sys.argv[1]) if len(sys.argv) == 2 else root / "target/debug/foxbot-http"
    if not binary.is_file():
        raise SystemExit("Build first: cargo build --locked -p foxbot-http")
    lock = threading.Lock()
    generations: dict[str, dict] = {}
    feedback: dict[str, dict] = {}
    counters = {"generate_requests": 0, "feedback_requests": 0, "fail_next_feedback": False}

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args) -> None:
            pass

        def do_POST(self) -> None:
            size = int(self.headers.get("Content-Length", "0"))
            if size < 1 or size > 1_048_576:
                self.send_error(400)
                return
            body = json.loads(self.rfile.read(size))
            status = 200
            with lock:
                if self.path == "/generate":
                    counters["generate_requests"] += 1
                    request_id = body["request_id"]
                    assert self.headers.get("Idempotency-Key") == request_id
                    assert "system_prompt" not in body and "context" not in body
                    response = generations.setdefault(request_id, {
                        "schema_version": "0.1", "request_id": request_id,
                        "conversation_ref": body["conversation_ref"],
                        "in_reply_to": [e["event_id"] for e in body["input_events"]],
                        "complete": True,
                        "outcome": {"result": "reply", "text": "仅供合成测试的本地 HTTP 回复"},
                    })
                elif self.path == "/feedback":
                    counters["feedback_requests"] += 1
                    assert self.headers.get("Idempotency-Key") == body["receipt_id"]
                    if counters["fail_next_feedback"]:
                        counters["fail_next_feedback"] = False
                        status = 503
                    else:
                        previous = feedback.get(body["request_id"], {}).get("revision", 0)
                        if body["revision"] > previous:
                            feedback[body["request_id"]] = body
                    response = {"schema_version": "0.1", "receipt_id": body["receipt_id"],
                                "revision": body["revision"], "accepted": True}
                else:
                    status, response = 404, {}
            payload = json.dumps(response, ensure_ascii=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        with tempfile.TemporaryDirectory(prefix="foxbot-http-smoke-") as temporary:
            directory = Path(temporary)
            origin = f"http://127.0.0.1:{server.server_address[1]}"
            config = directory / "provider.json"
            config.write_text(json.dumps({
                "protocol": "business_v1", "endpoint": origin + "/generate", "model": None,
                "context_mode": "service_managed", "receipt_endpoint": origin + "/feedback",
                "idempotency_supported": True, "staging_contract": True,
                "allow_loopback_http": True, "attempt_timeout_ms": 1000, "total_timeout_ms": 3000,
                "max_attempts": 2, "max_response_bytes": 131072, "max_in_flight": 2,
            }), encoding="utf-8")
            environment = dict(os.environ)
            environment.pop("FOXBOT_HTTP_TOKEN", None)

            def run(command: str, state: str, expected_code: int = 0) -> dict:
                args = [str(binary.resolve()), command, str(config), str(directory / state)]
                if command != "inspect":
                    args.append("--allow-network")
                result = subprocess.run(args, env=environment, capture_output=True, text=True, timeout=15)
                if result.returncode != expected_code:
                    raise AssertionError(f"unexpected CLI exit {result.returncode}: {result.stderr}")
                return json.loads(result.stdout) if result.stdout else {}

            first = run("synthetic-run", "happy")
            second = run("synthetic-run", "happy")
            assert first["provider_jobs_this_run"] == first["mock_send_calls_this_run"] == 1
            assert second["provider_jobs_this_run"] == second["mock_send_calls_this_run"] == 0
            with lock:
                counters["fail_next_feedback"] = True
            run("synthetic-run", "compensation", expected_code=1)
            before = run("inspect", "compensation")
            assert before["feedback_queue"]["pending"] == 1
            assert before["ledger"]["actions"][0][1] == "VERIFIED_OUTGOING"
            time.sleep(0.3)  # First durable retry delay is 250ms.
            repaired = run("feedback", "compensation")
            assert repaired["mock_send_calls_this_run"] == 0
            assert repaired["feedback_queue"]["acked"] == 1
            with lock:
                assert len(generations) == len(feedback) == 2
                assert all(r["disposition"] == "observed_outgoing" for r in feedback.values())
                report = {"loopback_only": True, "synthetic_data_only": True,
                          "first_run": [first["provider_jobs_this_run"], first["mock_send_calls_this_run"]],
                          "replay": [second["provider_jobs_this_run"], second["mock_send_calls_this_run"]],
                          "compensation_mock_send_calls": repaired["mock_send_calls_this_run"],
                          "feedback_acked": len(feedback), "generate_requests": counters["generate_requests"],
                          "feedback_requests": counters["feedback_requests"]}
            print(json.dumps(report, ensure_ascii=False, indent=2))
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)
        if worker.is_alive():
            raise RuntimeError("local fixture did not stop")


if __name__ == "__main__":
    main()
