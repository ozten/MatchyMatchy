#!/usr/bin/env python3
"""
v03-font-size serve script — port 3003, serves site/, SO_REUSEADDR, no caching variance.
"""
import http.server
import socketserver
import os

PORT = 3003
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
        print(f"v03-font-size serving {SITE_DIR} on port {PORT}")
        httpd.serve_forever()
