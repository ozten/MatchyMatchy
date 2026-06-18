# M6 Real-Pair Calibration Note

Spec §12 M6 gate. Design: `docs/design/M6.md`. Executed 2026-06-10/11 on the machine whose
environment fingerprint appears in every bundle (Linux, Chromium 148.0.7778.0, Playwright
1.60.0, dsf 1 — fingerprints matched across all pairs, so no pixel-confidence downgrades).

## 1. Pairs run

The project's real target page — the page the entire testbed golden was captured from — is
`https://www.hiya.com/products/connect/branded-call`. It changed substantially over the past
year (title, sections, footer, restyle), giving a genuine old/new pair. Three pairs, one
calibration axis each (M6.md §2):

| pair | old | new | axis |
|---|---|---|---|
| R1 | Wayback `20250603143211if_` snapshot (June 2025 build) | live page (June 2026) | real drift: TP detection + matcher stress across a year of edits |
| R2 | live page | same live page (same run) | noise floor: any issue = false positive |
| R3 | `http://localhost:3000/` (frozen, third-party-stripped golden) | live page | staging-vs-prod analog: third-party + origin noise |

Desktop viewport only (`desktop=1440x1000`), per the scoping decision in M6.md §2; mobile is
calibrated at first production use. No hide/mask/click selectors were needed — first runs were
raw, and no volatile-region masking proved necessary (R2 came back clean without any).

Captures happened once; all tuning re-analysis ran `matchy analyze` against the archived
bundles. Bundles + screenshots stay local under `calibration/.capture/` (gitignored — live
pages are unreproducible anyway); the committed record is `calibration/<pair>/diff-result.json`
(final code) plus these SHA-256s of the analyzed bundles:

```
r1-archive-vs-live/old.bundle.json: 2343d263e32a8d0bade1aef0bc996a342d73357067f3d40cdce1b8765fc8a28d
r1-archive-vs-live/new.bundle.json: 669b1bb2334afa2b182942538ced5e445280344700a5ac19b5cc5015ef07f451
r2-live-vs-live/old.bundle.json:    593223edf3f40a1cdf1540746e43ea2d4f40c9926f8257653afdf98ad2af7bc7
r2-live-vs-live/new.bundle.json:    d11324f3ed16b49dbdc0355fd4fddc14ce5f5456639023978d588371debdbd85
r3-golden-vs-live/old.bundle.json:  0bfce3726afbd63b0c58d7b849b9ed0a53a8b53addfcdb2fa25245ca01d39691
r3-golden-vs-live/new.bundle.json:  13c923ab46b58e893e499f4567be4d4efff89493697aad1d711fc141c8b65bb0
```

All determinism steps recorded `ran` in every bundle (no retries, no time-freeze fallback).
Determinism at real-page scale: double-analyze of the R1 bundles → byte-identical DiffResult
modulo `runId` (197→211-node streams, ~an order of magnitude beyond the unit fixtures).

## 2. Results and triage

Buckets per M6.md §3: TP-defect / TP-drift / FP-tool / FP-noise / archive-artifact.

### R2 — noise floor: **0 issues, status `pass`** (initial run AND final code)

Two independent captures of a live production page, minutes apart, with GTM, Weglot,
reCAPTCHA, HubSpot all running. Zero issues of any category. This is the strongest available
evidence that the frozen thresholds sit above real capture noise: `VISUAL_THRESHOLD`,
`MIN_REGION_AREA`, the matcher floors, and the style diff all emitted nothing.

### R1 — drift pair: 491 issues, status `fail` (initial code: 517)

Every issue traces to a bucket; **zero unexplained missing/added** (the gate criterion):

| type (count) | bucket | basis |
|---|---|---|
| changed_title (1), changed_meta_description (1) | TP-drift | verified against raw HTML of both sides |
| missing_text (15), missing_link (10), missing_image (5), missing_alt_text (1) | TP-drift | each anchor verified present-in-old / visibly-absent-in-new (footer+social+app-badge block removed, partner logos removed, "Limited Time Offer" banner removed, "Secure branding" section removed, nav entries hidden into menus) |
| changed_text (12) | TP-drift | real copy edits ("State of the Call 2024"→"2026") plus text-regrouping prefix deltas (finding F7) |
| changed_link_target (8) | TP-drift | real destination changes (work.hiya.com landing pages → same-site paths; logo link gained `?r=0`); includes 2 uncertain-band pairings (finding F5) |
| changed_link_text (8), changed_image_dimensions (12) | TP-drift | copy edits; assets re-exported at new resolutions |
| component_reordered (7), component_swapped (1) | TP-drift | all confidence 1.0, identity-stage pairs; real restructure (logo band moved up, top promo bar added, nav/footer reordered) |
| style_changed (356) | TP-drift | sitewide restyle cascades — top groups: text color `rgb(0,0,0)`→`rgb(80,93,111)`, font-size 13→14px, line-height bumps, `inline`→`inline-block`, alignment changes; ~104 distinct (property, from, to) groups ≈ a dozen root causes (M8 clustering will compress) |
| visual_region_changed (52), page_height_changed (1) | TP-drift | corroborating pixel evidence of the above; height 6679→6977 |
| url_trailing_slash (1) | TP | live page links `/newsroom/` against the `never` policy |

The 26-issue delta from the initial run (517→491) is exactly the four false-positive classes
fixed during the gate (§3): 6 duplicate-label `missing_text` (F1), ~12 sub-pixel style deltas
(F2), 8 asset-host `url()` deltas (F3).

Wayback note: `if_` frames include wombat.js, which shims `el.href` back to original-host
URLs, so the old bundle carries clean `https://www.hiya.com/...` hrefs and link parity needed
no wayback-specific handling. The only archive residue observed was the 8 `url()` style values
pointing at `web.archive.org` asset hosts — absorbed by F3's fix rather than special-cased.

### R3 — staging-vs-prod analog: 13 issues, status `fail` (initial code: 59)

| type (count) | bucket | basis |
|---|---|---|
| changed_link_target (6) | fixture-vendoring artifact | the testbed golden rewrote same-site links to local files (`pricing.html` vs live `/products/connect/pricing`); genuinely different hrefs, correctly reported, unique to the vendored fixture — not a real-migration pattern |
| visual_region_changed (6, all `info`) | TP-drift / noise | live top promo banner + small nav/hero/footer deltas vs. the day-old frozen capture; `info` severity is the §11-correct corroborating role |
| changed_link_text (1) | TP-drift | live copy fix `Cookies Settings`→`Cookie Settings` (OneTrust) landed within a day of the golden capture — real CMS drift, caught |

The 46-issue delta from the initial run (59→13): 30 `changed_link_target` were cross-origin
href aliasing noise (F4), 16 `style_changed` were asset-host `url()` noise (F3).

## 3. Findings and actions

**F1 — duplicate-label double-count (fixed: C1).** Webflow nests a label `<div>` inside
`<a>`/`<button>`; capture emits both a link/button node and a text node with identical text
inside the link's bbox. An element removal double-counted (v08: `missing_link` + `missing_text`
for one CTA); a one-sided nesting change false-flagged surviving elements (R1: "Get started",
"Log in" nav CTAs — link paired, orphan label read as missing). Fix: `semantic_diff` computes
the dup-label id set (normalized-text equality + bbox containment, `DUP_LABEL_BBOX_TOLERANCE_PX
= 2.0`) and suppresses **only `missing_text` emission** for those nodes. A first design that
filtered the matcher input streams was **rejected by the testbed**: v05's deliberate
`.button_content` change lives on the label node itself (Webflow styles the inner div), and
stream filtering silenced all 15 legitimate `style_changed` — the audited intent files caught
the over-suppression exactly as designed. Emission-side suppression keeps matching, style,
sequence, and visual behavior intact (18/19 goldens byte-identical; only v08 changed, by the
intended −1 issue). Carry-forward (auditor): the suppression is old-stream/`missing_text` only;
when `added_*` emission lands post-v1, the symmetric new-stream suppression must ship with it.

**F2 — sub-pixel style jitter (fixed: C2).** Live pages emit metric noise like `19.6px` vs
`19.5776px`, `14px` vs `13.984px`. New comparison rule: values equal when token structure is
identical and every numeric token (same unit) differs by `< STYLE_NUMERIC_EPSILON = 0.1`.
A real 13px→14px change still reports.

**F3 — asset-host `url()` noise (fixed: C3).** Migrations serve identical assets from
different hosts/paths (CDN vs staging origin vs archive host). `url()` tokens now compare by
filename tail when the two sides' URL hosts differ (or one side is relative after own-origin
normalization). Same-host path changes (e.g. `/v1/hero.svg`→`/v2/hero.svg`) and both-relative
changes still report — those are author-controlled.

**F4 — cross-origin href flood (fixed: C4).** The primary migration scenario compares the same
page on two origins; links to *either* input origin are same-site by definition. `hrefSim` and
the `changed_link_target` gate now normalize hrefs whose origin matches either page's origin to
path+query form. Evidence keeps raw values; genuinely third-party target changes (work.hiya.com)
still report (R1's 8 survived intact).

**F5 — uncertain band validated (no change).** Two anonymous icon-link ambiguities
(old-Twitter↔new-Facebook; a partners link) landed at combined scores 0.529/0.526 — inside the
`NO_MATCH_CEIL`(0.45)–`MATCH_FLOOR`(0.70) band — and were emitted at confidence 0.459 for
review instead of being coin-flipped, which is precisely the §3.3 design. Observation recorded:
empty-vs-empty accName/text similarity counts as 1.0, which inflates identity for anonymous
icon links (it contributed 0.45 of that identity score); the band absorbed it here. Post-v1
candidates (not taken now): empty-string similarity = 0, partial hrefSim credit for same-host,
severity cap for uncertain-band issues. Fix-value ordering already discounts via confidence.

**F6 — heading demotion reads as `missing_text` (documented sharp edge).** The live footer
demoted a `Company` heading to plain text; §6.2 kind-blocking (heading vs text) prevents the
pair, so the old heading reports missing while the replacement (an "added" node) is silent
because v1 has no `added_*` types. Factually defensible, but agents should know a
heading-level rewrite presents this way.

**F7 — text regrouping produces prefix-delta `changed_text` (documented sharp edge).** When a
combined label+body block is split (or merged) across builds, the pair still matches and
reports `changed_text` whose delta is the label prefix. Correct pairing, slightly noisy
evidence; acceptable.

**F8 — vendored-fixture link rewrites (testbed-specific).** R3's six surviving
`changed_link_target` come from the golden's local-file link rewriting; any future
golden-vs-live run should expect them or pin a baseline (M8) over their stable ids.

## 4. Frozen constants (spec §12 M6: "defaults frozen only after this gate")

All matcher and visual constants survived calibration **unchanged** — the evidence (R2 zero
FPs, R1 fully-explained triage, F5 band behavior) supports the M3/M4/M5 defaults. Frozen, with
the annotation block in `config.rs`:

| constant | value | calibration evidence |
|---|---|---|
| `IDENTITY_FLOOR` | 0.85 | all R1 cross-year true pairs cleared it; no false identity locks observed |
| `TIE_MARGIN` | 0.05 | duplicate "Read more"-class links resolved via stage 2 without mispairs |
| `MATCH_FLOOR` | 0.70 | real pairs scored ≥0.82; ambiguities fell below into the band |
| `NO_MATCH_CEIL` | 0.45 | genuine removals scored below; nothing real was force-paired |
| `UNCERTAIN_MULTIPLIER` | 0.6 | F5 issues surfaced at 0.459 — visible but discounted |
| per-kind identity weights | M3 table | href/text/alt dominance produced zero observed mispairs at identity stage |
| `STAGE2_*`, `TIEBREAK_*` | 0.7/0.3, 0.5/0.3/0.2 | stage-2 pairings consistent with visual reality |
| `VISUAL_THRESHOLD` | 0.005 | R2: zero visual issues at the noise floor |
| `MIN_REGION_AREA` | 2500 px² | R1: 52 regions on a full redesign — informative, not noisy |
| `SEQ_MIN_DISPLACEMENT` | 2 | R1: 8 structure issues, all real moves, conf 1.0; zero jitter reorders |
| **new** `DUP_LABEL_BBOX_TOLERANCE_PX` | 2.0 | F1 |
| **new** `STYLE_NUMERIC_EPSILON` | 0.1 | F2 (suppresses ≤0.0224px jitter; keeps 1px changes) |

## 5. Recommended real-migration run configuration

From observed behavior on this page family:

- Both default viewports for production runs; this gate calibrated desktop.
- No hide/mask was required even with GTM/Weglot/reCAPTCHA/HubSpot live (R2 = 0). Add
  `--mask` only for regions that prove volatile across re-runs.
- Set the trailing-slash policy explicitly (`--trailing never` matched this site's convention).
- Expect `style_changed` volume on intentional restyles to be large but cluster-compressible
  (M8); the issue ordering already front-loads high-fix-value items.
- Probe latency against archive.org was acceptable; no probe-timeout false `broken_link`
  appeared in any run.

## 6. DoD statement

- Written calibration note: this file.
- Triaged DiffResults: `calibration/{r1,r2,r3}-*/diff-result.json` (final code), triage tables
  above; **no unexplained missing/added** on the real pair.
- Defaults frozen: §4 table + `config.rs` annotation; the four normalization fixes (C1–C4) are
  code corrections found by the gate, each with unit tests and testbed-verified blast radius.
- "The team agrees it reflects reality": the triage above is the orchestrator's reading of the
  evidence; team sign-off is the review of this note and the committed DiffResults — the one
  DoD clause an autonomous run cannot self-certify.

## 7. Progressive-disclosure budget (2026-06-18)

Calibrated for the agent-first progressive-disclosure report (compact `report.md`
/ `report.html` default + `matchy show` drill-down). The compact view inlines
"Issues by section" branches in fix-value order until a cumulative rendered-size
(character) proxy is spent, then collapses the rest to one-line drill pointers.
Two bands govern the fold:

- **High watermark:** saturated ARIA regions (the existing `regions[]` rollups)
  and any single section whose inline size exceeds `DISCLOSURE_SECTION_CEILING`
  always collapse, independent of the budget.
- **Low watermark:** a section set whose total inline size fits
  `DISCLOSURE_BUDGET` inlines wholesale (R4 — a near-clean page shows everything).

### Method

Calibrated offline against the **frozen** `testbed/pairs/p01-hiya-number-registration`
desktop bundle via `matchy analyze` **replay** — never a live capture (p1-03: real
captures are not byte-stable, counts swung 116–155; only analyze over a frozen
bundle is deterministic). p01 desktop replays to 272 issues, 1 saturated region
(`contentinfo`, 88 members, saturation 0.86), 20 clusters.

### Measured per-section compact inline sizes (chars)

Caps were temporarily lifted to inline every section and measure each block:

| cluster | sections (chars) |
|---|---|
| content / defect | 385, 539, 539, 927, 937, 1048, **1137** (the `broken_link` section) |
| flood (18/27/30-issue style+visual dumps) | 1965, 2107, 3071 |

A wide natural gap (1137 → 1965 = **828 chars**) separates real-defect/content
sections from the style-flood sections.

### Frozen values and margins

| Constant | Value | Margin evidence |
|---|---|---|
| `DISCLOSURE_SECTION_CEILING` | **1500** | Mid-gap. The `broken_link` section (1137) stays under it with **363 chars (24%)** headroom and inlines (R13); the three flood sections collapse with **≥465 chars (31%)** margin. The earlier 1200 left only a fragile **69-char** margin on the defect section — message-length variance could have flipped it and buried the `broken_link`. |
| `DISCLOSURE_BUDGET` | **3000** | Inlines the top-3 content sections (cumulative **2570**, **430** headroom). The next section by fix-value (937) would reach **3507** — a **507-char** overshoot. The inline/collapse boundary therefore sits inside a **937-char gap**, so a ±1-char jitter cannot flip a branch (R3). |

### Observed p01 outcome (the gate)

- `contentinfo` collapses to **one** region pointer (the ~83 footer rows are gone),
  drillable via `matchy show --region contentinfo`.
- The standalone `broken_link` (Error) surfaces as the **first inlined item** of the
  lead section — never swallowed (R13).
- **3 sections inlined / 7 collapsed**; `report.md` is **6.9 KB** vs **19.9 KB** in
  `--full` mode (~65% smaller); `diff-result.json` is **byte-unchanged** at 539 KB
  (R8 — disclosure is a render/CLI projection, it never touches `json.rs`).
- AE4 round-trip verified: `matchy show --region contentinfo` returns exactly the 88
  members, hermetically (static-file read; no browser/network/re-capture).

### Per-section proxy spread / second-page note

The character proxy tracks message-length variance directly (it is the literal
rendered byte count of each inlined section). The content-vs-flood gap on p01 is
wide (828 chars), so the ceiling is well-centered; but the absolute sizes are
message-text dependent, so a reviewer should re-measure the per-section spread on
a **second flooded page** (home / branded-call) before treating 1500/3000 as
cross-page defaults. Second-page generalization is **deferred validation**, not a
pre-freeze blocker — mirroring how the `0.6`/`10` saturation constants were frozen
on p01 first.
