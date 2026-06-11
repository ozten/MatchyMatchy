#!/usr/bin/env python3
"""
v16-locale-lowercase serve script — port 47016.
Maps URL prefix /es-mx/products/connect/branded-call (lowercase region, hyphen separator) to site/.
GET /es-mx/products/connect/branded-call          -> site/index.html (200 directly, no redirect)
GET /es-mx/products/connect/<anything>            -> site/<anything> (assets resolve via parent dir)
GET /es-mx/products/connect/branded-call/<anything> -> site/<anything> (sub-path form, also handled)

The browser resolves relative asset URLs (e.g. assets/css/x.css) against the parent of the page
URL (/es-mx/products/connect/), not the page URL itself, so asset requests arrive as
/es-mx/products/connect/assets/css/x.css. The PARENT_PREFIX branch handles these.

The lowercase region subtag in es-mx (instead of es-MX) is the violation under test (spec §10.2).
No trailing-slash redirect is involved for the page URL itself.
"""
import http.server
import socketserver
import os
import urllib.parse

PORT = 47016
SITE_DIR = os.path.join(os.path.dirname(__file__), "site")
PAGE_PREFIX = "/es-mx/products/connect/branded-call"
PAGE_PREFIX_SLASH = PAGE_PREFIX + "/"
PARENT_PREFIX = "/es-mx/products/connect/"


class PrefixHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        path = parsed.path

        # Serve the page directly at the no-trailing-slash URL (200, no redirect)
        if path == PAGE_PREFIX or path == PAGE_PREFIX + "/index.html":
            file_path = os.path.join(SITE_DIR, "index.html")
        elif path.startswith(PAGE_PREFIX_SLASH):
            # Sub-path under branded-call/ (e.g. branded-call/assets/..., though unusual)
            rel = path[len(PAGE_PREFIX_SLASH):]
            if rel == "" or rel == "index.html":
                file_path = os.path.join(SITE_DIR, "index.html")
            else:
                rel_decoded = urllib.parse.unquote(rel)
                file_path = os.path.join(SITE_DIR, rel_decoded)
        elif path.startswith(PARENT_PREFIX):
            # Assets resolve via parent dir: /es-mx/products/connect/assets/css/x.css
            # -> site/assets/css/x.css
            rel = path[len(PARENT_PREFIX):]
            rel_decoded = urllib.parse.unquote(rel)
            file_path = os.path.join(SITE_DIR, rel_decoded)
        else:
            self._send_404()
            return

        # Serve the file
        if not os.path.isfile(file_path):
            self._send_404()
            return

        self._serve_file(file_path)

    def _send_404(self):
        body = b"404 Not Found"
        self.send_response(404)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def _serve_file(self, file_path):
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
        print(f"v16-locale-lowercase serving {SITE_DIR} on port {PORT}")
        httpd.serve_forever()
