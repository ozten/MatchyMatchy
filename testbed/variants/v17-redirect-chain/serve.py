#!/usr/bin/env python3
"""
v17-redirect-chain serve script — port 3017.
GET /start  -> 301 Location /mid
GET /mid    -> 301 Location /
GET /       -> 200 site/index.html
GET /*      -> serves site/ relative path (relative assets resolve naturally from root)

Two-hop 301 chain /start -> /mid -> / is the violation under test (spec §10.1: url_redirect_chain
when redirectChain.length > 1).
"""
import http.server
import socketserver
import os
import urllib.parse

PORT = 3017
SITE_DIR = os.path.join(os.path.dirname(__file__), "site")


class RedirectChainHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path

        # Two-hop deterministic 301 chain
        if path == "/start":
            self.send_response(301)
            self.send_header("Location", "/mid")
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            return

        if path == "/mid":
            self.send_response(301)
            self.send_header("Location", "/")
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            return

        # All other paths: serve from site/ directly (root maps to index.html)
        if path == "/" or path == "":
            rel = "index.html"
        else:
            # Strip leading slash for filesystem path
            rel = path.lstrip("/")

        rel_decoded = urllib.parse.unquote(rel)
        file_path = os.path.join(SITE_DIR, rel_decoded)

        if not os.path.isfile(file_path):
            self.send_response(404)
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            return

        # Determine content type
        ext = os.path.splitext(file_path)[1].lower()
        content_types = {
            ".html": "text/html; charset=utf-8",
            ".css": "text/css; charset=utf-8",
            ".js": "application/javascript; charset=utf-8",
            ".json": "application/json",
            ".png": "image/png",
            ".jpg": "image/jpeg",
            ".jpeg": "image/jpeg",
            ".gif": "image/gif",
            ".svg": "image/svg+xml",
            ".ico": "image/x-icon",
            ".woff": "font/woff",
            ".woff2": "font/woff2",
            ".ttf": "font/ttf",
            ".avif": "image/avif",
            ".webp": "image/webp",
        }
        ct = content_types.get(ext, "application/octet-stream")

        with open(file_path, "rb") as f:
            data = f.read()

        self.send_response(200)
        self.send_header("Content-Type", ct)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):
        pass  # silence request logging for determinism


class ReusableTCPServer(socketserver.TCPServer):
    allow_reuse_address = True


if __name__ == "__main__":
    with ReusableTCPServer(("", PORT), RedirectChainHandler) as httpd:
        print(f"v17-redirect-chain serving {SITE_DIR} on port {PORT}")
        httpd.serve_forever()
