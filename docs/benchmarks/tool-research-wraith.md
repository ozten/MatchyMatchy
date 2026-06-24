# Wraith — Responsive Screenshot Comparison Tool: Research Briefing

**Sources:** [BBC/wraith GitHub](https://github.com/bbc/wraith) · [Official docs](http://bbc.github.io/wraith/) · [RubyGems](https://rubygems.org/gems/wraith) · [Axelerant blog](https://www.axelerant.com/blog/visual-regression-testing-using-wraith) · [Config docs](https://bbc.github.io/wraith/configs.html)

---

## 1. Intended Purpose

Wraith is a **responsive visual regression testing** tool created by developers at BBC News. Its core use case is pixel-level screenshot comparison across two environments (live vs. staging) or across time (the same environment before and after a change). It automates:

- **Two-domain compare** (`wraith capture`): Screenshot the same paths on two different base URLs and diff them.
- **Time-based compare** (`wraith history` / `wraith latest`): Capture a baseline of one domain, then re-capture later and diff.
- **Spidering** (`wraith spider`): Auto-crawl a domain to generate a list of paths rather than specifying them manually.
- **Multi-config** (`wraith multi_capture`): Run multiple config files in one invocation.
- **Setup** (`wraith setup`): Scaffold a starter directory with template configs and scripts.

The tool was designed to catch unintended visual regressions — e.g., a CSS change breaking layout at a specific breakpoint — by flagging any path where pixel difference exceeds a configurable threshold.

---

## 2. Mechanism

### Screenshot Engine

Wraith shells out to a headless browser to capture screenshots at each specified viewport width. Supported engines as of v4.2.4 (the final release):

| Engine | Notes |
|---|---|
| `phantomjs` | WebKit-based; the original primary engine |
| `casperjs` | JavaScript automation layer on top of PhantomJS/SlimerJS; supports CSS selector targeting |
| `slimerjs` | Gecko-based; limited/broken support in later versions |
| `chrome` | **Added in a later version** via Selenium WebDriver + Chromedriver; supports CSS selector targeting |

The `browser:` key in the config YAML selects the engine. The Chrome test config uses:

```yaml
browser:
  phantomjs: "chrome"
```

This unusual nesting (using the `phantomjs:` sub-key set to `"chrome"`) is an artifact of how Chrome support was bolted on — it routes through a code path that dispatches to a Selenium/Chromedriver runner.

### Chrome/Headless Chrome Support

Chrome **is** officially supported in the v4.x series via `selenium-webdriver` + `chromedriver-helper`. However:

- `chromedriver-helper` was **deprecated and archived on 2019-03-31** ([announcement](https://github.com/flavorjones/chromedriver-helper/issues/83)), replaced by the `webdrivers` gem. Wraith's last release (June 2019) still lists `chromedriver-helper` as a runtime dependency and was never updated to `webdrivers`. This means installing Wraith today and attempting Chrome mode requires manually working around the broken/deprecated dependency.
- There is no native Puppeteer or `chrome-headless-shell` path — it goes through Selenium WebDriver only.
- The `settle:` config key (seconds to wait for page stabilization) is Chrome-only.

### Image Differencing

After both sets of screenshots are captured, Wraith uses **ImageMagick `compare`** (via the `mini_magick` Ruby gem) to:

1. Compute the percentage of pixels that differ between the two screenshots.
2. Generate a `diff.png` highlighting changed regions (default color: blue).
3. Write the percentage to a `data.txt` file alongside each diff image.

The `fuzz` parameter (e.g., `'20%'`) tells ImageMagick to treat colors within that color-space distance as identical — this suppresses false positives from anti-aliasing and minor font rendering differences. The `threshold` parameter (an integer, e.g., `5`) sets the maximum acceptable percentage before the comparison is flagged as a failure.

**Multiple breakpoints:** For each path in `paths:`, Wraith captures one screenshot per entry in `screen_widths:`. Width×height notation (e.g., `768x1500`) is supported to pin viewport height.

---

## 3. Config Model

The YAML config (e.g., `configs/capture.yaml`) controls the full run. Full annotated example:

```yaml
# Which headless browser engine to use
browser: "phantomjs"

# PhantomJS CLI flags (optional)
phantomjs_options: '--ignore-ssl-errors=true --ssl-protocol=tlsv1'

# Output directory for screenshots and diffs
directory: 'shots'

# Two base URLs to compare (keys become labels in the gallery)
domains:
  live: "https://www.example.com"
  staging: "https://staging.example.com"

# URL paths to capture on each domain
paths:
  home: /
  about: /about
  # Optional: capture only a specific CSS selector
  nav_menu:
    path: /
    selector: "#main-nav"

# Viewport widths (pixels); can also be WxH e.g. 768x1500
screen_widths:
  - 320
  - 768
  - 1024
  - 1280

# Optional JS to execute before capture (PhantomJS/CasperJS)
before_capture: 'javascript/disable_javascript--phantom.js'

# ImageMagick fuzz factor — suppress anti-aliasing false positives
fuzz: '20%'

# Max % pixel difference before flagging failure (default: 0)
threshold: 5

# How to sort/filter the gallery
# Options: alphanumeric | diffs_first | diffs_only
mode: diffs_first

# Gallery HTML report config
gallery:
  template: 'slideshow_template'   # or 'basic_template'
  thumb_width: 200
  thumb_height: 200

# Whether to resize or reload the browser between widths
resize_or_reload: 'resize'
```

The gallery is a static HTML file (`gallery.html`) generated by ERB templates bundled with the gem. It shows thumbnail grids of before/after/diff images, sorted per `mode:`.

For history mode, `history_dir:` sets where the baseline screenshots are stored.

---

## 4. Strengths

- **Pixel-accurate cross-breakpoint regression.** Wraith is genuinely reliable at catching any visual change — layout shifts, color changes, content reflow — across multiple viewport widths simultaneously.
- **Two-environment comparison.** Clean workflow for comparing live vs. staging at arbitrary paths, with no need for a test framework.
- **Low cognitive overhead.** A single YAML file drives the entire comparison run; no JavaScript test code required.
- **CSS selector targeting.** Via CasperJS or Chrome, you can scope a screenshot to a specific component rather than the full page.
- **Automatic spidering.** `wraith spider` can auto-discover paths from a sitemap or crawl, reducing manual path maintenance.
- **Gallery output.** The HTML gallery provides a fast human-review workflow — thumbnails sorted by most-changed first.

---

## 5. Weaknesses and Limitations

### No semantic understanding
Wraith outputs only: "X% of pixels changed at this path and breakpoint." It does not know *what* changed — it cannot distinguish a button color change from a broken layout, a missing image from a font change, or a z-index bug from an invisible change in hidden content. A 0.1% diff in a critical CTA and a 0.1% diff in a footer copyright year are treated identically.

### No HTTP-level checks
Wraith does not check HTTP status codes, follow redirects, validate links, detect console errors, check accessibility, or report on network requests. It is purely a pixel tool.

### False positives from dynamic content
Any content that differs between domains for non-visual-regression reasons (timestamps, ad slots, A/B test variants, user-specific content, animated elements) will trigger a diff. The `fuzz` parameter helps with sub-pixel rendering but not with content changes.

### PhantomJS is abandoned
PhantomJS's last release was **2.1.1 in January 2018**. The project is suspended ([official announcement](https://github.com/ariya/phantomjs/issues/15344)). It uses an old WebKit that does not support modern CSS (Grid, custom properties, many ES6+ features). Wraith's primary engine is a dead project.

### PhantomJS has NO official aarch64/ARM64 binary
The official PhantomJS project never produced a prebuilt binary for `linux/arm64` (aarch64). This is a firm blocker: `gem install wraith` succeeds, but `wraith capture` fails immediately on aarch64 Linux because `phantomjs` cannot be installed via npm/the standard PATH. Community forks exist (`fg2it/phantomjs-on-raspberry`, `clarity-tech/phantomjs-on-arm`) with unofficial aarch64 binaries, but they are also based on PhantomJS 2.0–2.1 and unmaintained.

### Chrome path is broken by dependency rot
The `chromedriver-helper` gem Wraith depends on for Chrome support was deprecated in March 2019 and archived. The gem's install-time post-install message warns users to migrate to `webdrivers`. Wraith's last release predates this and was never updated. Attempting `gem install wraith` today on a modern Ruby will pull in a `chromedriver-helper` that either fails or prints prominent deprecation errors.

### Maintenance: archived January 2026
The BBC/wraith repository was officially archived on **January 16, 2026**. It is read-only; no issues, PRs, or security fixes will be accepted. The last gem release was **v4.2.4 on June 26, 2019** — nearly 7 years before archiving.

### Ruby version issues
The Dockerfile used `ruby:2.1.2`. The gem's `required_ruby_version` is loose (`>= 0`), but gem dependencies may conflict with modern Ruby (3.x). The `anemone` gem (used for spidering) has not been updated since 2012.

---

## 6. Install/Run Requirements on Linux

### Minimal dependencies
- Ruby (2.x recommended; 3.x may have gem compatibility issues)
- ImageMagick (`apt install imagemagick` or `brew install imagemagick`)
- At least one screenshot engine (see below)

### Screenshot engine options on aarch64 Linux

| Engine | aarch64 Status | Notes |
|---|---|---|
| PhantomJS | **BLOCKED** — no official binary | Unofficial builds from `fg2it/phantomjs-on-raspberry` exist but are old/unmaintained |
| CasperJS | **BLOCKED** — depends on PhantomJS | Same blocker |
| SlimerJS | Theoretically possible (Gecko/Firefox based) but Wraith's SlimerJS support is described as broken/limited | Requires Firefox ESR |
| Chrome (Selenium) | **Potentially possible** — Chromium/Chrome has official aarch64 builds | But blocked by `chromedriver-helper` deprecation; would require manual workaround |

### Can Wraith run on aarch64 Linux in 2024+?

**Verdict: No, not without significant manual workarounds.**

The cleanest theoretical path is Chrome mode with manual dependency surgery:
1. Manually replace `chromedriver-helper` with `webdrivers` in the gem's vendored dependencies (or use `gem 'webdrivers'` in a Gemfile that pins wraith).
2. Install Chromium for aarch64 (`apt install chromium-browser`) and a matching `chromedriver`.
3. Point `CHROME_BIN` and `CHROMEDRIVER_BIN` env vars appropriately.

But this is not "install wraith and run" — it requires patching the gem's dependency graph. The official Docker image (`bbcnews/wraith`) is based on `ruby:2.1.2` + Debian Jessie and uses PhantomJS; it is x86_64-only and long unmaintained.

### Minimal commands IF running on x86_64 Linux with PhantomJS

```bash
# Install system deps
sudo apt-get install -y imagemagick nodejs npm

# Install PhantomJS via npm (x86_64 only)
npm install -g phantomjs-prebuilt casperjs

# Install wraith gem
gem install wraith

# Scaffold a project
wraith setup

# Edit configs/capture.yaml (set domains, paths, screen_widths)
# Then capture + compare:
wraith capture configs/capture.yaml
```

This produces screenshots in `shots/` and a `gallery.html`. The diff images land at `shots/<path>/<width>_diff.png` with accompanying `data.txt`.

---

## 7. Exact Output Format

After a `wraith capture` run, the output directory (set by `directory:` in config) contains:

```
shots/
  home/
    320_live.png          # screenshot from domain "live" at 320px
    320_staging.png       # screenshot from domain "staging" at 320px
    320_diff.png          # ImageMagick diff image, changed pixels highlighted blue
    320_data.txt          # contains the percentage difference, e.g. "1.23"
    1024_live.png
    1024_staging.png
    1024_diff.png
    1024_data.txt
  about/
    ...
  gallery.html            # HTML gallery report
```

The `data.txt` file contains a single float (the percentage of pixels that differ). The gallery HTML is generated from ERB templates bundled in the gem — it renders thumbnails linking to full-size images, sorted by `mode:` (e.g., `diffs_first` puts highest-% diffs at the top).

**Pass/fail:** Wraith exits non-zero if any path's diff percentage exceeds the `threshold:` value. No structured JSON output is produced — only the gallery and per-path PNG/TXT files.

There is no bounding-box output of changed regions beyond the visual diff image itself — ImageMagick's `compare` command highlights pixels but Wraith does not parse or expose the bounding-box coordinates programmatically.

---

## Summary

Wraith is a pioneering but now-defunct visual regression tool. It is **archived (January 2026), last released June 2019, and depends on PhantomJS (abandoned 2018)**. On aarch64/ARM64 Linux it is not runnable via any supported path without significant manual dependency surgery. On x86_64 it still installs and runs but relies on a dead browser engine that cannot render modern web pages accurately.

Its core mechanism — screenshot two URLs at N breakpoints → ImageMagick `compare` → % diff + highlighted diff PNG — is simple and replicable, but it produces only pixel-level signal with no semantic understanding of *what* changed or *why* it matters.
