# Golden Capture Notes — hiya.com/products/connect/branded-call

Captured: 2026-06-10  
Source URL: https://www.hiya.com/products/connect/branded-call  
Capture tool: wget --page-requisites + manual asset harvest via wget/curl  
Cleaned with: BeautifulSoup/lxml Python scripts (two-pass)

---

## STRIPS AND EDITS (every removal recorded)

### Scripts removed

1. **Google Tag Manager** — `<script>` block (lines 2–7 of original): the GTM loader
   `(function(w,d,s,l,i){…})(window,document,'script','dataLayer','GTM-K66MSHV')` removed entirely.
2. **reCAPTCHA v2** — `<script src="https://www.google.com/recaptcha/api.js">` removed.
3. **reCAPTCHA v3** — `<script src="https://www.google.com/recaptcha/api.js?render=6LcMQeorAAAAAJbxvVrMeVjDOF49C2TMmVAq_rWH">` removed.
4. **WebFont.js loader (CDN)** — `<script src="https://ajax.googleapis.com/ajax/libs/webfont/1.6.26/webfont.js">` removed.
5. **WebFont.load inline call** — inline `<script>` containing `WebFont.load({ google: { families: ["Catamaran:…","Nunito Sans:…"] } })` removed (Google Fonts JS loader).
6. **Weglot translation SDK** — `<script src="https://cdn.weglot.com/weglot.min.js">` removed.
7. **Weglot initialize inline** — inline `<script>Weglot.initialize({ api_key: 'wg_…' })` removed (3 occurrences).
8. **Language-picker config inline** — inline `<script>` setting `window.__LANG_PICKER_ROOT_MODE` and `window.__LANG_MAIN_HOST` removed.
9. **GTM noscript comment** — `<!-- Google Tag Manager noscript removed while consent gating is required -->` was already a stub; left as inert comment (no iframe present).

### Link/preconnect elements removed

10. **preconnect to cdn.prod.website-files.com** — `<link rel="preconnect" href="https://cdn.prod.website-files.com">` removed.
11. **preconnect to fonts.googleapis.com** — `<link rel="preconnect" href="https://fonts.googleapis.com">` removed.
12. **preconnect to fonts.gstatic.com** — `<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>` removed.

### Comment removed

13. **Last Published timestamp** — HTML comment `<!-- Last Published: Wed Jun 10 2026 00:31:39 GMT+0000 (Coordinated Universal Time) -->` at top of document removed (nondeterministic on re-capture).

### Dynamic content pinned

14. **Copyright year** — JavaScript `const currentYear = new Date().getFullYear()` replaced with `const currentYear = 2026` to pin the copyright year statically. The `<span data-year="current">` element is populated by this script and now always renders "Copyright Hiya 2026."

### External resources rewritten to local paths

15. **CDN CSS** — `https://cdn.prod.website-files.com/…/css/hiya-com-temp.shared.26cf8468f.min.css` → `assets/css/hiya-shared.min.css`. `integrity` and `crossorigin` attributes removed (hashes were for CDN delivery).
16. **CDN JS (3 files)** — main bundle and two chunk files rewritten to `assets/js/main.js`, `assets/js/chunk1.js`, `assets/js/chunk2.js`. Integrity/crossorigin removed.
17. **jQuery (Webflow CDN)** — `https://d3e54v103j8qbb.cloudfront.net/js/jquery-3.5.1.min.dc5e7f18c8.js` → `assets/js/jquery-3.5.1.min.js`. Integrity/crossorigin removed.
18. **64 CDN image/SVG/avif/webp assets** — all `src` and `srcset` attributes referencing `cdn.prod.website-files.com` rewritten to `assets/images/<filename>`.
19. **Favicon + apple-touch-icon** — CDN `href` attributes on `<link rel="shortcut icon">` and `<link rel="apple-touch-icon">` rewritten to `assets/images/…`.
20. **og:image / twitter:image meta** — two `<meta content="https://cdn.prod.website-files.com/…">` tags rewritten to `assets/images/…` (not fetched at render time but kept consistent).
21. **45 CDN url() references in hiya-shared.min.css** — all `url(https://cdn.prod.website-files.com/…)` occurrences rewritten to `url(../images/<filename>)` (or `url(../fonts/<filename>)` for Eina01 woff2 files).

### Fonts vendored locally

22. **Google Fonts — Catamaran** (weights 300/400/500/600/700) — woff2 files downloaded from `fonts.gstatic.com` to `assets/fonts/catamaran-{latin,latin-ext,tamil}.woff2`. Local `@font-face` CSS written to `assets/fonts/catamaran-local.css` and linked from `index.html`.
23. **Google Fonts — Nunito Sans** (weights 300/400/500/600/700) — woff2 files downloaded to `assets/fonts/nunito-sans-{latin,latin-ext,cyrillic,cyrillic-ext,vietnamese}.woff2`. Local CSS at `assets/fonts/nunito-sans-local.css`.
24. **Eina01 (Webflow CDN)** — regular/semibold/bold woff2 files referenced in `hiya-shared.min.css` downloaded to `assets/fonts/5f1e0dc2…_Eina01-{regular,semibold,bold}.woff2`. CSS rewritten to `url(../fonts/…)`.

### Unavailable asset noted

25. **680ba660f7ce0092827ba934_Vector%20(17)** — referenced in `hiya-shared.min.css` via an escaped CSS url. HTTP 403 from CDN; a zero-byte placeholder file `assets/images/680ba660f7ce0092827ba934_Vector-17.svg` was created and the CSS url() rewritten to it. This asset was not visible in any rendered section of the page.

### Removed wget artifacts

26. **www.hiya.com/ directory** — wget created `site/www.hiya.com/` during crawl (containing `robots.txt` and sibling page HTML). Deleted entirely; `index.html` contains no file references into that directory.

---

## MATERIAL INVENTORY

### H1 Text

> Reach more customers with Hiya's Branded Call

### Representative Paragraph

> 80% of unidentified calls go unanswered. Read the benchmark report for what is happening in voice today, and what you can do to drive business.

### Forms

No `<form>` elements are present in the static HTML. The page loads a HubSpot form dynamically via JavaScript (`hbspt.forms.create`) into a target `<div>` container. At golden-capture time (static HTML only) no form fields are present.

- **HubSpot form target container**: inline script block around line 1009 handles phone-number formatting and form submission for a contact/demo form in the CTA section. Selector for the container div: `.js.w-embed.w-script` (the wrapping div hosting the phone-format script). The form itself injects into the DOM at runtime and is not capturable in static HTML.

### Gradient / Distinctive Background Elements

All values are in `assets/css/hiya-shared.min.css` unless noted.

| Selector | background-image value |
|---|---|
| `.notification-bar_component` | `linear-gradient(90deg, var(--primary--brand-700), var(--primary--brand))` |
| `.background-gradient-01` | `linear-gradient(180deg, var(--primary--brand-100), white 90%)` |
| `.background-gradient-02` | `linear-gradient(180deg, var(--primary--brand-100), var(--neutral--gray-100) 90%)` |
| `.background-gradient-03` | `linear-gradient(180deg, var(--neutral--gray-100), var(--base--white))` |
| `.text-gradient-primary` | `linear-gradient(93deg, var(--primary--brand-450), var(--accent--blue-400))` (used as text-fill gradient) |
| `.section_dark-grid` | `url(../images/6700244ca2c4310b28f5a5bd_bg-grid.svg), linear-gradient(180deg, var(--primary--brand), …)` |
| `[data-rt-color-scheme="dark"] strong` | `linear-gradient(90deg, #9e73f7, #1ca6cc)` (inline style block in HTML) |
| `.hs-form select` | `linear-gradient(45deg, transparent 50%, gray 50%), linear-gradient(135deg, gray 50%, transparent 50%)` (dropdown arrow; inline style block) |

### Main Page Sections (in document order)

Sections live in two `<div class="slot-placeholder">` sibling groups.

**Group 1** (8 direct `<section>` siblings — all swappable with each other):

| # | Heading / Label |
|---|---|
| 0 | Hero — "Reach more customers with Hiya's Branded Call" |
| 1 | Logo bar (trusted-by logos, no heading) |
| 2 | Problem — "When prospects and customers don't know who's calling, they don't pick up" |
| 3 | Solution overview — "Stand out, build trust, and get more calls answered with Hiya's Branded Call" |
| 4 | Feature: Display — "Display your company's name, logo, and call reason on outbound calls" |
| 5 | Feature: Analytics — "Branded Call performance analytics" |
| 6 | Feature: Easy to use — "Easy to use — no integration required" |
| 7 | Feature: Secure Branding — "Secure Branding: Your Brand, Only on Verified Calls" |

**Sibling-swappable pairs (recommended for variants):**
- Sections 4 and 5 (`data-wf--split-content-section` siblings, same class, visually equivalent weight — swap reorders two feature blocks).
- Sections 2 and 3 (`section[2]` problem block and `section[3]` solution overview — both `section-zero` siblings, swapping inverts problem→solution narrative order).

**Group 2** (5 direct `<section>` siblings):

| # | Heading / Label |
|---|---|
| 8 | "Who we work for" (industry/use-case tabs) |
| 9 | Case study / testimonial block (no heading) |
| 10 | G2 awards block (no heading) |
| 11 | CTA banner — "Our sales team does not cold call…" (mid-banner) |
| 12 | FAQs — "FAQs" (accordion) |

### Links

Total `<a>` elements: **161**

Example hrefs:
- `https://www.hiya.com/lp/see-branded-call-demo`
- `https://www.hiya.com/free-call-inspection`
- `pricing.html` (relative — Hiya Connect pricing page)

### Images

Total `<img>` elements: **103**

Examples:
- `assets/images/5f1a08b8f263c3ef6e879a5b_hiya%20logo.svg` — Hiya logo (navigation)
- `assets/images/68654b6ec13627bdfc5c9100_home_main-hero_us.avif` — hero background image
- `assets/images/673b4d11b8fb561f6d7d8ccd_bc_performance-analysis_us.png` — analytics dashboard screenshot
