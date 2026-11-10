#!/usr/bin/env python3
"""python3 serve.py [port]  -> builds the preview and serves it on http://localhost:8099/ (static files only)."""
import functools, http.server, pathlib, socketserver, subprocess, sys
here = pathlib.Path(__file__).resolve().parent
subprocess.run([sys.executable, str(here / 'build.py')], check=True)
port = int(sys.argv[1]) if len(sys.argv) > 1 else 8099
class H(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a): pass
socketserver.TCPServer.allow_reuse_address = True
with socketserver.TCPServer(('127.0.0.1', port), functools.partial(H, directory=str(here))) as s:
    print(f'OSNT mission-control preview: http://localhost:{port}/   (Ctrl+C to stop)')
    s.serve_forever()
