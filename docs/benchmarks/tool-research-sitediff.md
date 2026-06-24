# SiteDiff Research Briefing

**Tool:** SiteDiff — Ruby gem by Evolving Web
**Repo:** https://github.com/evolvingweb/sitediff
**Gem:** https://rubygems.org/gems/sitediff
**Latest release:** v1.2.11 (August 22, 2024)

---

## 1. Purpose and Intended Use

SiteDiff's primary purpose is to detect unexpected HTML regressions between two versions of a website. Its canonical use cases are:

- **Site migrations** — comparing the old site against a freshly migrated version to confirm content parity
- **CMS/platform upgrades** — e.g. Drupal major-version upgrades where markup may silently change
- **Pre/post-deployment QA** — diffing a staging site against production after a deploy

The tool's own framing ([evolvingweb.com](https://evolvingweb.com/blog/sitediff-compare-multiple-versions-website)): *"SiteDiff makes it easy to see how a website changes by comparing two similar sites or tracking changes to a single site over time."*

### CLI Workflow

| Command | Semantics |
|---|---|
| `sitediff init <url>` or `sitediff init <before-url> <after-url>` | Scaffolds `sitediff.yaml`; optionally crawls and caches a snapshot |
| `sitediff crawl` | Discovers all paths on the site; writes them to `paths.txt` |
| `sitediff diff` | Fetches pages from both before/after and reports markup diffs |
| `sitediff store` | Re-records the current state as the new baseline (replaces the "before" cache) |
| `sitediff serve` | Launches a local web server (port 13080) showing the HTML diff report |

**`--cached` flag semantics on `diff`:**
- `--cached=all` — skip re-fetching; compare previously stored HTML blobs directly
- `--cached=none` — always re-fetch both sides (no caching)
- Default — fetches "after" fresh, uses cached "before"

**Baseline / iterate-fix-rediff loop:**
The user runs `sitediff store` to record a snapshot of the "before" state. They then make changes to the site and run `sitediff diff` iteratively. When the diff output is clean (or all remaining diffs are accepted as intentional), they run `sitediff store` again to advance the baseline. This loop is described as taking several passes when differences are plentiful.

Sources: [GitHub README](https://github.com/evolvingweb/sitediff/blob/master/README.md), [evolvingweb.com blog](https://evolvingweb.com/blog/sitediff-compare-multiple-versions-website)

---

## 2. Mechanism

### Fetching

SiteDiff fetches pages over **HTTP using the `typhoeus` gem** (a libcurl wrapper), not a browser. There is no headless Chromium, Playwright, or Selenium involved. JavaScript is **not executed**; the tool receives raw server-sent HTML.

### Comparison

Comparison is **text-based HTML diff** using the `diffy` gem (which wraps the OS `diff` utility). It is **not a pixel diff**. The diff is performed on the raw/normalized HTML string.

No visual rendering occurs. A CSS-only change (e.g. a class rename from `btn-primary` to `btn-main`, or a stylesheet background-color change) will only be caught if it manifests in the HTML markup — a CSS-only change that leaves markup untouched is **invisible to SiteDiff**.

### Normalization before diff

Before diffing, SiteDiff applies a pipeline of normalization rules configured in `sitediff.yaml`:

1. **`selector:`** — Extract only the content within a CSS selector (e.g. `#main-content`) from both pages before comparing. Reduces noise from header/footer boilerplate.
2. **`sanitization:`** — Regex substitution rules applied to the HTML text. Each rule has a `title`, a `pattern` (regex), an optional `substitute` (default: empty string), and optional `selector`/`path` scoping. Use cases: stripping CSRF tokens, session-dependent values, random IDs, ad network parameters, timestamps.
3. **`dom_transform:`** — Structural HTML mutations applied before comparison: `remove` (delete matched elements), `unwrap` (replace an element with its children), `remove_class` (strip CSS class names). These are used when elements exist in markup for functional reasons but create false positives.

**`before:` / `after:` scoping:** Any of the above blocks can be nested under a `before:` or `after:` key so the rule applies only to one side. Top-level rules apply to both.

### Results presentation

`sitediff serve` launches a WEBrick-based HTTP server on port 13080. The UI shows:
- An overview page listing all crawled paths with pass (unchanged) / fail (changed) status
- Per-path colorized before/after diff view (line-by-line markup diff)
- A side-by-side view showing browser-rendered old vs new HTML in iframes

The report directory is written to `output/`. A file `output/failures.txt` lists all paths that differed.

Sources: [GitHub README](https://github.com/evolvingweb/sitediff), [RubyGems](https://rubygems.org/gems/sitediff), [example YAML](https://github.com/evolvingweb/sitediff/blob/master/config/sitediff.example.yaml)

---

## 3. Configuration Model

The canonical config file is `sitediff.yaml` (or `settings.yaml`). Key fields:

```yaml
before_url: http://old-site.example.com
after_url: http://new-site.example.com

# or single-site with cached baseline:
# url: http://mysite.example.com

paths:
  - /
  - /about
  - /contact

# or: discover via crawl (writes paths.txt, referenced automatically)

selector: "#main-content"   # compare only this subtree

sanitization:
  - title: "Remove CSRF tokens"
    pattern: 'name="csrf_token" value="[^"]+"'
    substitute: 'name="csrf_token" value=""'

  - title: "Remove Google Analytics params"
    pattern: '\?utm_[^"]*'
    substitute: ''

dom_transform:
  - type: remove
    selector: ".advertisement"
  - type: unwrap
    selector: "div.wrapper"
  - type: remove_class
    selector: "a"
    class: ["active", "selected"]

# Scoped rules — only apply to one side:
before:
  dom_transform:
    - type: remove
      selector: ".legacy-widget"

settings:
  depth: 3
  interval: 0
  concurrency: 3
  timeout: 30

includes:
  - config/drupal_rules.yaml
```

**Path discovery:** Either supply paths explicitly, or run `sitediff crawl` which follows links up to the configured `depth` and writes discovered paths to `paths.txt`. The `exclude` list (regex patterns) prunes URLs from the crawl.

Sources: [example YAML](https://github.com/evolvingweb/sitediff/blob/master/config/sitediff.example.yaml), [GitHub README](https://github.com/evolvingweb/sitediff/blob/master/README.md)

---

## 4. Strengths

- **Large-site coverage at scale.** Crawl-driven discovery means sitediff can cover hundreds or thousands of pages automatically. Concurrency is configurable.
- **Noise suppression via rules.** The sanitization + dom_transform pipeline handles real-world false positives (session tokens, ad IDs, timestamps) well once rules are tuned.
- **Drupal ecosystem polish.** Ships with preset rule files for Drupal-specific markup patterns; deeply integrated with the Drupal community.
- **Iterative loop UX.** The store → diff → fix → rediff loop is explicitly designed and well-documented; the HTML report gives immediate per-path drill-down.
- **No browser required.** Pure HTTP fetch means it works anywhere curl works — lightweight, fast, no headless browser overhead.
- **ddev integration** — a [companion plugin](https://github.com/evolvingweb/ddev-sitediff) exists for ddev-based Drupal development environments.
- **Real maintenance.** v1.2.11 released August 2024 — the project is not abandoned.

---

## 5. Weaknesses and Limitations

### Critical: No visual / pixel comparison

SiteDiff cannot catch any change that does not alter HTML markup. This includes:

- **CSS-only regressions** — a changed `background-color`, `font-size`, `margin`, or `z-index` leaves no HTML trace
- **Layout shifts** — if a grid change makes content overlap or reflow, the markup may be identical
- **Image rendering differences** — an `<img src="logo.png">` pointing to a re-exported image with different dimensions still looks identical in markup
- **Font loading failures** — a broken WOFF2 reference is invisible in markup

A GitHub issue ([#191, opened June 2024](https://github.com/evolvingweb/sitediff/issues/191)) explicitly asks whether sitediff can compare CSS styles or visual UI — it remains open with no response, confirming this is a known gap.

### No HTTP-level checks

SiteDiff does not inspect:
- HTTP response status codes (a 301/302/404/500 does not trigger a "diff")
- Redirect chains
- Response headers (Content-Type, Cache-Control, security headers)
- Broken links (unless the broken page returns markup sitediff can diff)

### No JavaScript execution

Pages are fetched with curl — JavaScript is not executed. Single-page-app content that requires JS to render will produce incomplete HTML. Dynamic content driven by JS (popups, modals, lazy-loaded sections) is invisible.

### No accessibility or console checks

Accessibility (WCAG), ARIA attributes, and browser console errors are out of scope.

### Regex fragility

The sanitization rule system uses raw regexes over HTML, which is inherently brittle. The documentation warns that greedy patterns (`.*`, `.+`) can over-match. Poorly written rules can suppress real regressions.

### Ruby dependency overhead

Requires Ruby >= 3.1.2 plus native extensions (nokogiri → libxml2/libxslt, typhoeus → libcurl). This is non-trivial to install on aarch64 Ubuntu (see §6).

### Modest adoption

23,515 total gem downloads as of late 2024. Niche tool with primary audience in Drupal shops.

---

## 6. Install and Run Requirements on Linux (Ubuntu/Debian aarch64)

### Native system dependencies

```bash
sudo apt-get install -y ruby-dev libz-dev gcc patch make \
  libxml2-dev libxslt-dev libcurl4-openssl-dev
```

(Note: some Ubuntu versions ship `libcurl3` only; `libcurl4-openssl-dev` is the current equivalent.)

### Ruby version

Ruby >= 3.1.2 is required. Ubuntu 22.04 ships Ruby 3.0; use `rbenv` or `rvm` to install 3.1.x:

```bash
# via rbenv
curl -fsSL https://github.com/rbenv/rbenv-installer/raw/HEAD/bin/rbenv-installer | bash
rbenv install 3.1.4
rbenv global 3.1.4
```

### Install nokogiri with system libraries (avoids compiling bundled libxml2)

```bash
gem install nokogiri --no-document -- --use-system-libraries
```

### Install SiteDiff

```bash
gem install sitediff
```

### Minimal command sequence to diff one path between two base URLs

```bash
# 1. Scaffold config pointing at before and after URLs
sitediff init http://old.example.com http://new.example.com

# 2. Edit sitediff.yaml to restrict to one path (optional for quick test)
# paths:
#   - /some/page

# 3. Run diff
sitediff diff

# 4. View HTML report
sitediff serve
# Then open http://localhost:13080 in a browser
```

### Docker alternative (avoids Ruby setup)

```bash
docker run -p 13080:13080 -t -d --name sitediff evolvingweb/sitediff:latest
```

Sources: [INSTALLATION.md](https://github.com/evolvingweb/sitediff/blob/master/INSTALLATION.md), [RubyGems sitediff](https://rubygems.org/gems/sitediff)

---

## 7. Output Format

### During `sitediff diff`

- Prints per-path status lines to stdout: `[PASS]` or `[FAIL path]`
- Writes the full HTML report to the `output/` directory relative to cwd
- `output/failures.txt` — newline-delimited list of paths that differed
- `output/<encoded-path>.html` — per-path diff HTML file
- `output/report.html` — overview index page

### Exit behavior

- Exit 0 — all paths matched (no diffs found)
- Exit non-zero — at least one path differed (or an error occurred)

This makes the tool CI-friendly: `sitediff diff && echo "clean"` is a valid gate.

### `sitediff serve` report UI

- Overview page at `http://localhost:13080` listing all paths with colored PASS/FAIL indicators
- Clicking a failed path shows a colorized line-level markup diff (green = added, red = removed)
- A side-by-side tab renders both versions in iframes for visual inspection

### Export

`sitediff diff --export` produces a gzipped tar archive of the full report for offline sharing.

Sources: [GitHub README](https://github.com/evolvingweb/sitediff/blob/master/README.md), [evolvingweb.com blog](https://evolvingweb.com/blog/sitediff-compare-multiple-versions-website)

---

## Summary Comparison vs matchy

| Dimension | SiteDiff | matchy (page-pair-diff) |
|---|---|---|
| Fetch mechanism | HTTP/curl (no browser) | Playwright (full browser render) |
| Comparison type | HTML text diff | Visual pixel + DOM structure |
| CSS-only changes | **Not detected** | Detected (pixel diff) |
| JS-rendered content | **Not detected** | Detected (Playwright executes JS) |
| HTTP status/redirects | **Not checked** | In scope |
| Normalization | Regex + dom_transform YAML rules | Confidence-band signal system |
| Report format | HTML diff UI (per-path) | JSON DiffResult contract |
| Maintenance | v1.2.11, Aug 2024, active | In development |
| Install footprint | Ruby 3.1+, libxml2, libcurl | Node + Rust + Playwright |
