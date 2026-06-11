# Playwright setup on this machine

Matchy's capture layer (`packages/capture`) drives Chromium via Playwright. This
note records how Playwright is installed here and why it is isolated from the rest
of the system.

## TL;DR

- Matchy pins **Playwright 1.60.0** (`packages/capture/package.json`) → Chromium build
  **1223** (148.0.7778.x).
- Matchy's browsers live in a **repo-local cache**, `./.pw-browsers/`, selected via the
  `PLAYWRIGHT_BROWSERS_PATH` env var. They do **not** go in the shared
  `~/.cache/ms-playwright`.
- This keeps the checkout self-contained and guarantees we never disturb other tools
  that depend on the shared cache (notably **agent-browser**, which pins its own
  Chromium build there and launches it by absolute path via
  `AGENT_BROWSER_EXECUTABLE_PATH`).
- This is an **arm64 Linux** machine; Playwright auto-selected the arm64 builds.

## First-time / fresh-checkout install

```bash
# from repo root
cd packages/capture && npm install --no-audit --no-fund && cd -

# download Matchy's Chromium into the repo-local cache (NOT the shared one)
PLAYWRIGHT_BROWSERS_PATH=$PWD/.pw-browsers \
  npx --prefix packages/capture playwright install chromium
```

`--with-deps` is not needed here: the system libraries Chromium requires are already
present (agent-browser's Chromium runs fine on this box).

Verify:

```bash
PLAYWRIGHT_BROWSERS_PATH=$PWD/.pw-browsers ./target/release/matchy doctor
# expect: node.js OK, capture.cjs OK, playwright v1.60.0 OK, chromium build 148.x OK
```

## How the env var reaches the browser

`matchy` (Rust) spawns `node capture.cjs` with `Command::new("node")` and **inherits the
parent environment** (no `env_clear`). Playwright reads `PLAYWRIGHT_BROWSERS_PATH` at
launch time, so it is enough for the var to be set in the process that runs `matchy`.

It is wired in two places so you rarely set it by hand:

- **Makefile** exports `PLAYWRIGHT_BROWSERS_PATH := $(CURDIR)/.pw-browsers`, so every
  `make` target (`build`, `verify`, `fixture`, `testbed-*`) inherits it.
- **Testbed harnesses** (`testbed/check-fixture.py`, `testbed/check-m8.py`) do
  `os.environ.setdefault("PLAYWRIGHT_BROWSERS_PATH", REPO_DIR/".pw-browsers")`, so running
  them directly with `python3` also works. `setdefault` means an explicit override still wins.

To run the bare binary outside `make`, export it yourself:

```bash
export PLAYWRIGHT_BROWSERS_PATH=$PWD/.pw-browsers
./target/release/matchy --old <url> --new <url> --out <dir>
```

## Coexistence with agent-browser (do not break this)

`agent-browser` launches Chromium from the **shared** cache by absolute path:

```
AGENT_BROWSER_EXECUTABLE_PATH=/home/admin/.cache/ms-playwright/chromium-1217/chrome-linux/chrome
```

Because Matchy uses a *separate* `PLAYWRIGHT_BROWSERS_PATH`, nothing Matchy does
(`npm install`, `playwright install`, captures) writes to or garbage-collects the shared
cache. The shared cache still holds `chromium-1217` and its `.links` registry entry from
the global Playwright **1.59.1** install, untouched.

Rules of thumb:

- Never run `playwright uninstall` / `playwright install` **without** `PLAYWRIGHT_BROWSERS_PATH`
  set to `./.pw-browsers` — an unscoped install can trigger reference-counted GC on the shared
  cache.
- Never delete `~/.cache/ms-playwright/chromium-1217`.

## Testbed port range: 47000–47021

The testbed serves on **47000–47021** (golden 47000, variants 47001–47021). It was
migrated there from the original 3000–3021 (uniform `+44000` offset) because a separate
long-running `next-server` on this machine occupies port **3001** — which the testbed
needed for `v01-identical`, causing matchy to capture the foreign page and emit a large
spurious diff. The 47xxx range is quiet and avoids common dev-server ports. Migration is
reproducible via `scripts/migrate-ports.py`; details in `docs/golden-changelog.md`
(2026-06-11 port-migration entry).

## Remaining blocker for a fully-green `make verify`

Independent of ports, `make verify` is still gated by the pre-existing **srcset-404 flake**
documented in `docs/issue-v08-srcset-404-flake.md`: four unvendored `-p-NNN.webp` responsive
images 404 intermittently, yielding a spurious `network_error` on v01/v08/v09 and an unstable
`visual_region_changed` count on v04. The fix (vendor the four images) is in that doc. Until
then, individual fixtures pass on flake-free runs; the migration itself is fully verified
(20/21 goldens clean by substitution + v11 re-recorded and golden-audited).
