#!/usr/bin/env python3
"""
v15-locale-underscore serve script — port 3015.
Maps URL prefix /es_MX/products/connect/branded-call (no trailing slash) to site/ directory.
GET /es_MX/products/connect/branded-call          -> site/index.html (200 directly, no redirect)
GET /es_MX/products/connect/branded-call/assets/... -> site/assets/...
GET /assets/...                                    -> site/assets/... (fallback)

The underscore in es_MX (instead of hyphen es-MX) is the violation under test.
No trailing-slash redirect is involved for the page URL itself.
"""
import http.server
import socketserver
import os
import urllib.parse

PORT = 3015
SITE_DIR = os.path.join(os.path.dirname(__file__), "site")
PAGE_PREFIX = "/es_MX/products/connect/branded-call"
PAGE_PREFIX_SLASH = PAGE_PREFIX + "/"


class PrefixHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path

        # Serve the page directly at the no-trailing-slash URL (200, no redirect)
        if path == PAGE_PREFIX or path == PAGE_PREFIX + "/index.html":
            file_path = os.path.join(SITE_DIR, "index.html")
        elif path.startswith(PAGE_PREFIX_SLASH):
            rel = path[len(PAGE_PREFIX_SLASH):]
            if rel == "" or rel == "index.html":
                file_path = os.path.join(SITE_DIR, "index.html")
            else:
                rel_decoded = urllib.parse.unquote(rel)
                file_path = os.path.join(SITE_DIR, rel_decoded)
        elif path.startswith("/assets/"):
            # Fallback: bare /assets/... -> site/assets/...
            rel = path[1:]
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
        print(f"v15-locale-underscore serving {SITE_DIR} on port {PORT}")
        httpd.serve_forever()
