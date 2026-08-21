/**
 * Minimal local HTTP server for feeding fixture HTML to `stabilize()`'s
 * `page.goto(url)` call (stabilize navigates itself — it does not accept an
 * already-loaded `Page` — so tests need a real, network-idle-safe URL rather
 * than `page.setContent()`). Bound to 127.0.0.1 on an ephemeral port; always
 * responds 200 text/html with the fixed body regardless of path.
 */
import * as http from "http";
import type { Server } from "http";
import type { AddressInfo } from "net";

export interface ServedHtml {
  url: string;
  close: () => Promise<void>;
}

export async function serveHtml(html: string): Promise<ServedHtml> {
  const server: Server = http.createServer((_req, res) => {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(html);
  });

  await new Promise<void>((resolve) => {
    server.listen(0, "127.0.0.1", resolve);
  });

  const { port } = server.address() as AddressInfo;
  const url = `http://127.0.0.1:${port}/`;

  return {
    url,
    close: () =>
      new Promise<void>((resolve, reject) => {
        server.close((err) => (err ? reject(err) : resolve()));
      }),
  };
}
