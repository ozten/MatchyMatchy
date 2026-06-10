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
  external: ["playwright", "playwright-core"],
  // Ensure node built-ins are not bundled
  packages: "external",
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
