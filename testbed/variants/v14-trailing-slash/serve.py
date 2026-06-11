#!/usr/bin/env python3
"""
v14-trailing-slash serve script — port 47014.
Maps URL prefix /products/connect/branded-call/ to site/ directory.
GET /products/connect/branded-call/          -> site/index.html
GET /products/connect/branded-call/assets/... -> site/assets/...
GET /products/connect/branded-call           (no trailing slash) -> 301 redirect to trailing-slash form
GET /assets/...                              -> site/assets/... (fallback for asset references)
"""
import http.server
import socketserver
import os
import urllib.parse

PORT = 47014
SITE_DIR = os.path.join(os.path.dirname(__file__), "site")
PAGE_PREFIX = "/products/connect/branded-call"
PAGE_PREFIX_SLASH = PAGE_PREFIX + "/"


class PrefixHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path

        # 301 redirect: bare prefix without trailing slash -> trailing-slash form
        if path == PAGE_PREFIX:
            self.send_response(301)
            self.send_header("Location", PAGE_PREFIX_SLASH)
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            return

        # Map prefix path to filesystem path
        if path.startswith(PAGE_PREFIX_SLASH):
            rel = path[len(PAGE_PREFIX_SLASH):]  # strip leading prefix
            if rel == "" or rel == "index.html":
                file_path = os.path.join(SITE_DIR, "index.html")
            else:
                # Decode percent-encoding for filesystem lookup
                rel_decoded = urllib.parse.unquote(rel)
                file_path = os.path.join(SITE_DIR, rel_decoded)
        elif path.startswith("/assets/"):
            # Fallback: bare /assets/... -> site/assets/...
            rel = path[1:]  # strip leading /
            rel_decoded = urllib.parse.unquote(rel)
            file_path = os.path.join(SITE_DIR, rel_decoded)
        else:
            self.send_response(404)
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            return

        # Serve the file
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
    with ReusableTCPServer(("", PORT), PrefixHandler) as httpd:
        print(f"v14-trailing-slash serving {SITE_DIR} on port {PORT}")
        httpd.serve_forever()
