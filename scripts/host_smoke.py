#!/usr/bin/env python3
"""Synthetic continuous-host smoke over loopback. No model/keychain/native chat access.
Build: cargo build --locked -p foxbot-host; run: python3 scripts/host_smoke.py
"""
from __future__ import annotations
import copy
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    binary = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else root / 'target/debug/foxbot-host'
    if not binary.is_file():
        raise SystemExit('Build first: cargo build --locked -p foxbot-host')
    lock = threading.Lock()
    generations: dict[str, dict] = {}
    receipts: dict[str, dict] = {}
    controls = {'delay': 0.0, 'fail_receipt': 0, 'generate_requests': 0}
    started = time.monotonic()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass
        def do_POST(self):
            size = int(self.headers.get('Content-Length', '0'))
            if not 0 < size <= 1048576:
                self.send_error(400)
                return
            body = json.loads(self.rfile.read(size))
            status, delay = 200, 0.0
            with lock:
                if self.path == '/generate':
                    controls['generate_requests'] += 1
                    delay = controls['delay']
                    assert 'system_prompt' not in body and 'context' not in body
                    response = generations.setdefault(body['request_id'], {
                        'schema_version': '0.1', 'request_id': body['request_id'],
                        'conversation_ref': body['conversation_ref'],
                        'in_reply_to': [e['event_id'] for e in body['input_events']], 'complete': True,
                        'outcome': {'result': 'reply', 'text': 'Synthetic local-host reply only'},
                    })
                elif self.path == '/feedback':
                    if controls['fail_receipt']:
                        controls['fail_receipt'] -= 1
                        status = 503
                    elif body['revision'] > receipts.get(body['request_id'], {}).get('revision', 0):
                        receipts[body['request_id']] = body
                    response = {'schema_version': '0.1', 'receipt_id': body['receipt_id'],
                                'revision': body['revision'], 'accepted': True}
                else:
                    status, response = 404, {}
            time.sleep(delay)
            payload = json.dumps(response).encode()
            try:
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
            except (BrokenPipeError, ConnectionResetError):
                pass  # Expected when a cancelled generation closes its socket.

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    children: list[subprocess.Popen] = []
    readers: list[threading.Thread] = []
    try:
        with tempfile.TemporaryDirectory(prefix='foxbot-host-smoke-') as temporary:
            directory = Path(temporary)
            config = json.loads((root / 'examples/host-synthetic.json').read_text())
            origin = f'http://127.0.0.1:{server.server_address[1]}'
            config['http']['endpoint'] = origin + '/generate'
            config['http']['receipt_endpoint'] = origin + '/feedback'
            config['bindings'].append(copy.deepcopy(config['bindings'][0]))
            config['bindings'][1]['key']['conversation'] = 'synthetic-second-conversation'
            path = directory / 'config.json'
            path.write_text(json.dumps(config))
            env = dict(os.environ)
            env.pop('FOXBOT_HTTP_TOKEN', None)

            def launch():
                process = subprocess.Popen([str(binary), 'run', str(path), str(directory / 'ledger'),
                    '--allow-network', '--allow-plaintext-synthetic'], stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env, bufsize=1)
                children.append(process)
                output = queue.Queue()
                def read():
                    for line in process.stdout:
                        output.put(json.loads(line))
                reader = threading.Thread(target=read)
                readers.append(reader)
                reader.start()
                assert output.get(timeout=5)['event'] == 'started_paused'
                return process, output

            def command(process, output, name, **fields):
                process.stdin.write(json.dumps({'command': name, **fields}) + '\n')
                process.stdin.flush()
                value = output.get(timeout=5)
                assert value['event'] != 'command_rejected', value['event']
                return value['status']

            def wait_status(process, output, predicate):
                deadline = time.monotonic() + 8
                while time.monotonic() < deadline:
                    value = command(process, output, 'status')
                    if predicate(value):
                        return value
                    time.sleep(0.02)
                raise AssertionError('continuous-host condition did not complete')

            def finish(process, output):
                value = command(process, output, 'stop')
                # Deliberately leave stdin open until after exit; stop must be sufficient.
                assert process.wait(timeout=5) == 0, process.stderr.read()
                assert value['active_jobs'] == value['active_feedback'] == 0
                process.stdin.close()
                return value

            try:
                process, output = launch()
                command(process, output, 'message', session=0, id='history', text='Synthetic baseline')
                assert command(process, output, 'status')['provider_jobs_started'] == 0
                command(process, output, 'resume')
                command(process, output, 'message', session=0, id='one', text='Synthetic question 1')
                wait_status(process, output, lambda s: s['observed_outgoing_this_run'] == 1 and s['feedback_acked_this_run'] >= 1)
                command(process, output, 'message', session=0, id='one', text='Synthetic question 1')
                with lock:
                    controls['delay'] = 0.4
                    before = controls['generate_requests']
                command(process, output, 'message', session=1, id='cancelled', text='Synthetic cancellation')
                deadline = time.monotonic() + 5
                while True:
                    with lock:
                        arrived = controls['generate_requests'] > before
                    if arrived:
                        break
                    assert time.monotonic() < deadline
                    time.sleep(0.01)
                assert command(process, output, 'pause')['paused']
                command(process, output, 'message', session=1, id='paused', text='Synthetic paused baseline')
                wait_status(process, output, lambda s: s['active_jobs'] == 0 and s['feedback_acked_this_run'] >= 2)
                with lock:
                    controls['delay'], controls['fail_receipt'] = 0.0, 1
                command(process, output, 'resume')
                command(process, output, 'message', session=1, id='two', text='Synthetic question 2')
                first = wait_status(process, output, lambda s: s['observed_outgoing_this_run'] == 2 and s['feedback_acked_this_run'] >= 3)
                finish(process, output)
                replay, replay_output = launch()
                command(replay, replay_output, 'resume')
                command(replay, replay_output, 'message', session=1, id='two', text='Synthetic question 2')
                time.sleep(0.08)
                second = command(replay, replay_output, 'status')
                assert second['provider_jobs_started'] == second['observed_outgoing_this_run'] == 0
                finish(replay, replay_output)
                report = {'loopback_only': True, 'synthetic_only': True, 'native_chat_operations': 0,
                    'first_run_jobs': first['provider_jobs_started'], 'first_run_mock_sends': first['observed_outgoing_this_run'],
                    'feedback_acked': first['feedback_acked_this_run'], 'replay_jobs': second['provider_jobs_started'],
                    'replay_mock_sends': second['observed_outgoing_this_run'], 'elapsed_seconds': round(time.monotonic()-started, 3)}
                print(json.dumps(report, indent=2))
            finally:
                # Confirm every process is dead BEFORE TemporaryDirectory removes its ledger.
                for process in children:
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=5)
                    for pipe in (process.stdin, process.stdout, process.stderr):
                        if pipe and not pipe.closed:
                            pipe.close()
                for reader in readers:
                    reader.join(timeout=5)
                    assert not reader.is_alive()
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        assert not thread.is_alive()

if __name__ == '__main__':
    main()
