#!/usr/bin/env python3
"""Exercise the actual settings server with temporary config and a loopback model fixture.

No chat reads/sends, Keychain operations, external requests or real API keys.
--preview-seconds opens the same test page for a bounded manual visual inspection on macOS.
"""
from __future__ import annotations
import argparse
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
from urllib.parse import urlsplit


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preview-seconds', type=int, default=0)
    args = parser.parse_args()
    if not 0 <= args.preview_seconds <= 180:
        raise SystemExit('preview must be between 0 and 180 seconds')
    root = Path(__file__).resolve().parents[1]
    binary = root / 'target/debug/foxbot-host'
    requests = []
    class Model(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass
        def do_POST(self):
            n = int(self.headers.get('Content-Length', '0'))
            body = json.loads(self.rfile.read(n))
            requests.append(body)
            code = 401 if self.path == '/denied' else 200
            payload = json.dumps({'choices': [{'index': 0, 'finish_reason': 'stop',
                'message': {'role': 'assistant', 'content': 'OK'}}]}).encode()
            self.send_response(code)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
    model = ThreadingHTTPServer(('127.0.0.1', 0), Model)
    thread = threading.Thread(target=model.serve_forever)
    thread.start()
    children = []
    try:
        with tempfile.TemporaryDirectory(prefix='foxbot-settings-') as temp:
            path = Path(temp) / 'app/config.json'
            origin = f'http://127.0.0.1:{model.server_address[1]}'
            def start():
                process = subprocess.Popen([str(binary), 'settings', str(path), '--no-open'],
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                children.append(process)
                lines = queue.Queue()
                def first_line():
                    lines.put(process.stdout.readline())
                reader = threading.Thread(target=first_line)
                reader.start()
                ready = json.loads(lines.get(timeout=10))
                reader.join(timeout=2)
                assert not reader.is_alive()
                parsed = urlsplit(ready['url'])
                def call(route, data=None, *, authenticated=True, host=None):
                    conn = http.client.HTTPConnection(parsed.hostname, parsed.port, timeout=20)
                    headers = {'Host': host or parsed.netloc, 'Origin': f'http://{parsed.netloc}'}
                    if authenticated:
                        headers['X-FoxBot-Session'] = parsed.fragment
                    if data is not None:
                        headers['Content-Type'] = 'application/json'
                    conn.request('GET' if data is None else 'POST', route,
                        body=None if data is None else json.dumps(data).encode(), headers=headers)
                    response = conn.getresponse()
                    body = response.read()
                    status = response.status
                    conn.close()
                    return status, body
                return process, call, ready['url']
            process, call, url = start()
            assert call('/api/config', authenticated=False)[0] == 403
            assert call('/api/config', host='evil.example')[0] == 403
            assert b'AI' in call('/')[1]
            assert b'inputConnection' in call('/app.js')[1]
            def view():
                code, raw = call('/api/config')
                assert code == 200
                assert b'synthetic-secret' not in raw
                return json.loads(raw)
            def edit(action):
                current = view()
                code, raw = call('/api/edit', {'revision': current['revision'], 'edit': action})
                assert code == 200, raw
                return json.loads(raw)
            a = {'id': 'primary', 'name': '常用接口（测试）', 'protocol': 'chat_completions',
                'endpoint': origin + '/chat/completions', 'api_key': 'synthetic-secret-primary', 'model': 'demo-model'}
            b = dict(a, id='backup', name='本地模型（测试）', api_key='synthetic-secret-backup')
            edit({'action': 'save', 'connection': a})
            edit({'action': 'save', 'connection': b})
            assert len(requests) == 0  # Save is offline.
            saved = json.loads(path.read_text())
            assert saved['connections'][0]['api_key'] == 'synthetic-secret-primary'
            keep = dict(a, name='常用接口（测试）')
            keep.pop('api_key')
            edit({'action': 'save', 'connection': keep})
            assert json.loads(path.read_text())['connections'][0]['api_key'] == 'synthetic-secret-primary'
            edit({'action': 'duplicate', 'id': 'backup'})
            duplicate = view()['config']['connections'][-1]['id']
            edit({'action': 'delete', 'id': duplicate})
            edit({'action': 'set_default', 'id': 'backup'})
            assert view()['config']['default_connection'] == 'backup'
            code, raw = call('/api/test', {'connection': keep, 'confirm_billable': True})
            assert code == 200 and json.loads(raw)['ok'] is True
            denied = dict(b, endpoint=origin + '/denied')
            code, raw = call('/api/test', {'connection': denied, 'confirm_billable': True})
            assert code == 200 and json.loads(raw)['ok'] is False
            assert len(requests) == 2
            assert all('连接测试' in json.dumps(q, ensure_ascii=False) for q in requests)
            exported = call('/api/export')[1]
            assert b'api_key' not in exported and b'synthetic-secret' not in exported
            assert json.loads(path.read_text())['connections'][1]['endpoint'] == b['endpoint']
            assert call('/api/shutdown', {})[0] == 200
            assert process.wait(timeout=10) == 0
            process, call, url = start()
            assert len(view()['config']['connections']) == 2
            assert view()['config']['default_connection'] == 'backup'
            if args.preview_seconds:
                if sys.platform == 'darwin':
                    subprocess.run(['/usr/bin/open', url], check=True, timeout=10)
                print(json.dumps({'preview_ready': True, 'seconds': args.preview_seconds,
                    'fixture_config_only': True}), flush=True)
                time.sleep(args.preview_seconds)
            assert call('/api/shutdown', {})[0] == 200
            assert process.wait(timeout=10) == 0
            print(json.dumps({'settings_http_smoke': 'PASS', 'saved_connections': 2,
                'inline_key_roundtrip': True, 'masked_ui_and_export': True, 'reopen_persisted': True,
                'loopback_generation_requests': len(requests), 'external_model_requests': 0,
                'native_chat_operations': 0, 'keychain_operations': 0}), flush=True)
    finally:
        for process in children:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)
            for stream in (process.stdout, process.stderr):
                if stream:
                    stream.close()
        model.shutdown()
        model.server_close()
        thread.join(timeout=5)
        assert not thread.is_alive()


if __name__ == '__main__':
    main()
