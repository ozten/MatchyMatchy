#!/usr/bin/env python3
"""
v18-status-mismatch serve script — port 47018.
GET /products/connect/branded-call -> 404 with self-contained rendered error page.
Every other path also returns 404 with the same error page.
Content-Type: text/html for all responses.

The 404 response at the URL that golden serves with 200 is the violation under test
(spec §10.1 status_code_mismatch short-circuit).
"""
import http.server
import socketserver
import os

PORT = 47018

ERROR_PAGE = b"""<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>404 - Page not found</title>
  <style>
    *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
    body {
      display: flex;
      align-items: center;
      justify-content: center;
      min-height: 100vh;
      background: #f5f5f5;
      font-family: system-ui, sans-serif;
      color: #333;
    }
    .card {
      background: #fff;
      border: 1px solid #ddd;
      border-radius: 8px;
      padding: 48px 64px;
      text-align: center;
      max-width: 480px;
    }
    .code { font-size: 72px; font-weight: 700; color: #c00; line-height: 1; }
    .message { font-size: 20px; margin-top: 16px; }
  </style>
</head>
<body>
  <div class="card">
    <div class="code">404</div>
    <div class="message">Page not found</div>
  </div>
</body>
</html>
"""


class StatusMismatchHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(404)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(ERROR_PAGE)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(ERROR_PAGE)

    def log_message(self, fmt, *args):
        pass  # silence request logging for determinism


class ReusableTCPServer(socketserver.TCPServer):
    allow_reuse_address = True


if __name__ == "__main__":
    with ReusableTCPServer(("", PORT), StatusMismatchHandler) as httpd:
        print(f"v18-status-mismatch serving on port {PORT}")
        httpd.serve_forever()
