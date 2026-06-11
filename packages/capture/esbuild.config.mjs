import { build } from "esbuild";
import { mkdirSync } from "fs";

mkdirSync("dist", { recursive: true });

await build({
  entryPoints: ["src/capture.ts"],
  bundle: true,
  platform: "node",
  target: "node20",
  format: "cjs",
  outfile: "dist/capture.cjs",
  // playwright/playwright-core are host deps (resolved at runtime, cannot be bundled).
  // Node built-ins are also external. axe-core is NOT listed here so it gets bundled.
  external: [
    "playwright",
    "playwright-core",
    // Node built-ins
    "fs", "path", "readline", "os", "url", "util", "stream", "buffer",
    "crypto", "http", "https", "net", "dns", "tls", "child_process",
    "events", "assert", "zlib", "string_decoder", "querystring",
    "process", "module", "vm",
  ],
  // Source map for debugging
  sourcemap: false,
  // Minify in production
  minify: false,
  // Needed for proper CJS output
  define: {
    "import.meta.url": '"file://__bundled__"',
  },
});

console.log("Built dist/capture.cjs");
