"""Serve crates/ui/tests/fonts on 127.0.0.1 for the web_fonts browser test.

    python3 crates/ui/tests/web/serve_fonts.py [port]   # default 8137

/fonts/<file> serves the file. /flaky/<file> answers 503 the first time and serves the file after.
Other paths answer 404. /log lists the paths requested so far, as JSON.
"""

import http.server
import json
import pathlib
import sys

FONTS = pathlib.Path(__file__).resolve().parent.parent / "fonts"
requested = []
failed_once = set()


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/log":
            return self.reply(200, json.dumps(requested).encode(), "application/json")
        requested.append(self.path)
        kind, _, name = self.path.lstrip("/").partition("/")
        file = FONTS / name
        if kind == "flaky" and self.path not in failed_once:
            failed_once.add(self.path)
            return self.reply(503, b"", "text/plain")
        if kind in ("fonts", "flaky") and "/" not in name and file.is_file():
            return self.reply(200, file.read_bytes(), "font/ttf")
        self.reply(404, b"", "text/plain")

    def reply(self, status, body, content_type):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


port = int(sys.argv[1]) if len(sys.argv) > 1 else 8137
http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
