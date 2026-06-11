"""
v20-console-error serve script — port 47020, serves site/, SO_REUSEADDR, no caching variance.
"""
import http.server
import socketserver
import os

PORT = 47020
SITE_DIR = os.path.join(os.path.dirname(__file__), "site")

class NoCacheHandler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=SITE_DIR, **kwargs)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, fmt, *args):
        pass  # silence request logging for determinism

class ReusableTCPServer(socketserver.TCPServer):
    allow_reuse_address = True

if __name__ == "__main__":
    with ReusableTCPServer(("", PORT), NoCacheHandler) as httpd:
        print(f"v20-console-error serving {SITE_DIR} on port {PORT}")
        httpd.serve_forever()
