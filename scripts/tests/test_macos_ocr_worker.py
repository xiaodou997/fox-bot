import json
import sys
import time
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from macos_ocr_worker import WorkerProcess, safe_worker_reply


class OCRWorkerSupervisorTests(unittest.TestCase):
    def test_worker_process_reuses_one_child_for_multiple_requests(self):
        script = (
            "import json,sys\n"
            "for line in sys.stdin:\n"
            " d=json.loads(line); print(json.dumps({'id':d['id'],'status':'SHUTDOWN'}),flush=True)\n"
            " if d['command']=='shutdown': break\n"
        )
        worker = WorkerProcess([sys.executable, "-u", "-c", script])
        try:
            first = json.loads(worker.request({"id": "a", "command": "ping"}, 2))
            second = json.loads(worker.request({"id": "b", "command": "ping"}, 2))
            self.assertEqual(first["id"], "a")
            self.assertEqual(second["id"], "b")
            self.assertIsNone(worker.process.poll())
        finally:
            worker.shutdown()

    def test_timeout_kills_worker_and_does_not_leave_request_running(self):
        worker = WorkerProcess([sys.executable, "-u", "-c", "import time; time.sleep(10)"])
        with self.assertRaises(TimeoutError):
            worker.request({"id": "slow", "command": "warmup"}, 0.1)
        self.assertIsNotNone(worker.process.poll())
        worker.shutdown()

    def test_safe_warmup_reply_is_closed_and_bounded(self):
        raw = json.dumps({"id": "warmup", "status": "WARMED",
                          "warmup": {"elapsed_milliseconds": 1234, "line_count": 0, "succeeded": True}}).encode()
        self.assertEqual(safe_worker_reply(raw, "warmup", "warmup")["status"], "WARMED")
        changed = json.loads(raw); changed["raw_text"] = "PRIVATE"
        with self.assertRaises(ValueError):
            safe_worker_reply(json.dumps(changed).encode(), "warmup", "warmup")

    def test_request_id_mismatch_and_duplicate_json_keys_are_rejected(self):
        with self.assertRaises(ValueError):
            safe_worker_reply(b'{"id":"other","status":"SHUTDOWN"}', "expected", "shutdown")
        with self.assertRaises(ValueError):
            safe_worker_reply(b'{"id":"x","id":"x","status":"SHUTDOWN"}', "x", "shutdown")

    def test_worker_module_never_prints_child_stderr_or_accepts_large_commands(self):
        worker = WorkerProcess([sys.executable, "-u", "-c", "import sys,time; print('SECRET',file=sys.stderr); time.sleep(2)"])
        try:
            with self.assertRaises(ValueError):
                worker.request({"id": "x", "command": "x", "padding": "a" * 5000}, 1)
        finally:
            worker.shutdown()


if __name__ == "__main__":
    unittest.main()
