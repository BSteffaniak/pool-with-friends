#!/usr/bin/env python3
"""Exercise real Chromium readiness and failure collection against synthetic pages."""
import http.server
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time

browser = os.environ.get('CHROME_BIN') or shutil.which('google-chrome') or '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
root = Path(__file__).resolve().parent


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):
        if self.path == '/slow-navigation':
            time.sleep(2)
        script = {
            '/slow-navigation': 'document.querySelector("main").dataset.clientState="ready"',
            '/ready': 'setTimeout(()=>document.querySelector("main").dataset.clientState="ready",1500)',
            '/error': 'document.querySelector("main").dataset.clientState="error"',
            '/exception': 'setTimeout(()=>{throw new Error("synthetic failure")},500)',
        }.get(self.path, '')
        body = f'<main id="game-shell" data-client-state="loading"></main><script>{script}</script>'.encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/html')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
try:
    with tempfile.TemporaryDirectory(prefix='pwmtf-readiness-test-') as tmp:
        for route, expected in [('/ready', 0), ('/slow-navigation', 0), ('/error', 1), ('/exception', 1), ('/loading', 1)]:
            output = Path(tmp) / f'{route[1:]}.log'
            result = subprocess.run(['node', str(root / 'browser-smoke-ready.js'), browser,
                f'http://127.0.0.1:{server.server_port}{route}', str(output)],
                capture_output=True, text=True, timeout=240, check=False)
            artifacts = os.environ.get('PWMTF_TEST_ARTIFACT_DIR')
            if artifacts:
                destination = Path(artifacts) / 'readiness'
                destination.mkdir(parents=True, exist_ok=True)
                (destination / f'{route[1:]}-process.log').write_text(
                    f'exit={result.returncode}\n{result.stdout}\n{result.stderr}')
                if output.exists():
                    shutil.copyfile(output, destination / output.name)
            assert result.returncode == expected, (route, result.stderr)
            assert output.exists(), route
            if route == '/ready':
                assert 'data-client-state="ready"' in output.read_text()
            if route == '/loading':
                assert 'within 60 seconds' in result.stderr
            if route == '/exception':
                assert 'Uncaught browser application exception' in result.stderr
            if route == '/error':
                assert 'Application state: error' in result.stderr
        output = Path(tmp) / 'missing.log'
        result = subprocess.run(['node', str(root / 'browser-smoke-ready.js'), '/nonexistent/pwmtf-browser',
            f'http://127.0.0.1:{server.server_port}/ready', str(output)], capture_output=True, text=True, timeout=15)
        assert result.returncode != 0
        assert 'Browser could not start' in result.stderr
finally:
    server.shutdown()
    server.server_close()
    thread.join()
print('Browser readiness self-tests passed (delayed ready, error, exception, timeout, launch failure)')
