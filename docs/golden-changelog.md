# Golden / expected-output changelog

Every change to `expected-issues.json` files or recorded goldens requires an entry here with a
spec justification and a pasted `golden-auditor` verdict (see CLAUDE.md, "Golden discipline").

---

## 2026-06-10 — Initial authorship of all 15 `expected-issues.json` files (testbed build)

**What:** First versions of `testbed/variants/v01…v15/expected-issues.json`, authored by the
orchestrator against spec §7.3 (taxonomy), §10 (hygiene), §11 (visual emission), §13.1/§13.2
(fixture matrix), and the M4/M5 DoDs, using each variant's `manifest.json` and
`testbed/golden/CAPTURE-NOTES.md` as factual ground truth.

**Notable authoring decisions:**
- **v08 adaptation:** spec §13.1.5 calls for a missing *form*, but the golden page has no static
  `<form>` (HubSpot injects forms at runtime — CAPTURE-NOTES.md). v08 covers the G2 missing-CTA
  case instead (hero secondary CTA removed); `missing_form` coverage requires a future synthetic
  fixture pair. Documented in v08's notes.
- **v02:** v1 taxonomy has no `added_*` content types, so the added banner is expected via
  `visual_region_changed` (§11 region→node linking); status deliberately not pinned beyond
  "not error".
- **v13/v01:** zero-issue `status: pass` controls with `maxIssues: 0`, v13 scoped to the default
  profile (§13.2 info-note carve-out applies only when implementation comparison is enabled).

**Initial audit (golden-auditor, 2026-06-10): 13/15 APPROVE, 2 REJECT.**

Rejected and corrected before any implementation exists (no code was accommodated):

1. **v03-font-size** — REJECT: second required matcher demanded `style_changed(font-size)` on the
   "Spam, robocalls, and scams" paragraph, but that paragraph is sized by the untouched
   `--_typography---paragraph-md--*` variable chain (the variant edits only h1/h2/paragraph-BODY
   variables), making the requirement unsatisfiable against a correct tool. **Fix (auditor option
   a):** matcher dropped; H1 matcher carries G1; paragraph-level G1 coverage rides on v04 (whose
   body→secondary-font inheritance chain the auditor verified). Manifest knock-on wording
   corrected (it overstated "every body paragraph").
2. **v10-paragraph-removed** — REJECT: `nearestHeadingContains` used ASCII apostrophes but the
   real heading contains U+2019 (smart-quote folding is *optional* per §11), making the substring
   unsatisfiable. **Fix:** re-anchored to the apostrophe-free substring "prospects and customers".

**Standing conditions from the audit:**
- The fixture runner must pin `--trailing never` for v14 (its `url_trailing_slash` requirement
  presumes the "never" policy, §10.1).
- At M6 (calibration gate), revisit the `["warn","fail"]` status bands on v14/v15 — if the §9
  default profile maps hygiene to fail, "warn" may be tightened out (would itself require an
  entry + audit here).

**Re-audit verdicts (corrected v03 + v10), pasted verbatim from golden-auditor:**

```
VERDICT: APPROVE
EXPECTATION(S): /home/admin/MatchyMatchy/testbed/variants/v03-font-size/expected-issues.json
REASONING: The unsatisfiable paragraph matcher is gone; the surviving required matcher
(style_changed/font-size on the H1, evidence pinned to property only) was already verified
satisfiable at audit — the h1 element computes font-size from the edited
--_typography---h1--font-size-* chain at the default desktop viewport, with the hardcoded
.heading-rich-text-fluid-medium h1 overrides confined to <=991px media queries. G1 traceability
(§1, §13.1 fixture 9, M4 DoD style_changed-with-property-delta) is still exercised; the forbidden
entries (missing_h1/missing_text/changed_h1) remain intact, so no detection power was lost beyond
removal of a requirement that was factually wrong. The notes honestly record the removal, the
REJECT that forced it, and where paragraph-level G1 coverage now lives (v04, whose
body→paragraph-body--font inheritance chain I verified in the initial audit). manifest.json
knockOnEffects now correctly distinguishes body-tier text from the untouched paragraph-md chain —
manifest and expectation are consistent.
CONDITIONS: Create docs/golden-changelog.md and record this change (what was removed, why the old
expectation was wrong — paragraph-md vs paragraph-body variable chain — and this APPROVE verdict).
The in-file notes reference the audit but the changelog required by the golden discipline does not
exist yet.
```

```
VERDICT: APPROVE
EXPECTATION(S): /home/admin/MatchyMatchy/testbed/variants/v10-paragraph-removed/expected-issues.json
REASONING: The required matcher is now satisfiable: "prospects and customers" is verified as an
exact ASCII substring of the actual h3 in golden/site/index.html, avoiding both U+2019 apostrophes
that made the previous string unmatchable under §11's optional-only smart-quote folding. Conjoined
with textContains "Spam, robocalls, and scams" (verified ASCII in the page), the matcher still
pins the deleted paragraph to the correct Problem-section anchor, so §13.1 fixture 6 / G2 is fully
exercised with no loss of specificity. The forbidden changed_text entry on the same text — the
trap for mis-pairing the deleted node — is unchanged, and nothing else in the file moved.
CONDITIONS: Include this re-anchoring in the same docs/golden-changelog.md entry (old string's
character-form defect, new string, this verdict). Carry the character-form rule forward: any
future matcher touching headings containing don't/who's/company's must use the curly form or an
apostrophe-free substring.
```

(The changelog the conditions ask for is this file/entry — it was created concurrently with the
re-audit, so the "does not exist yet" note was already discharged when the verdict landed.)

**Carry-forward authoring rule (from the v10 re-audit):** any future matcher touching headings
containing don't / who's / company's must use the curly (U+2019) form or an apostrophe-free
substring.

---

## 2026-06-10 — M3: v02 byte-golden re-recorded after approved capture-extraction changes (D13/D14); first recording of v08–v12 goldens

**What:** `testbed/goldens/v02-banner-added.diffresult.json` re-recorded from the M3 run.
No `expected-issues.json` changed; v02's intent check passes before and after. Also recorded
(first-time promotions after intent-check pass, per the milestone loop): goldens for
v08-cta-removed, v09-h1-changed, v10-paragraph-removed, v11-broken-link, v12-image-404, and the
`make verify` gate extended to v08–v12 plus a v08 determinism spot-check.

**Why the old golden was superseded:** two deliberate, spec-aligned capture-extraction behavior
changes made during M3, documented as **D13 and D14 in `docs/design/M3.md` §8**:
- **D13** — broken images (`complete && naturalWidth == 0`, not CSS-hidden) are kept in the
  SemanticNode stream despite zero rendered area. Required by goal G7 / spec §13.1 fixture 12:
  without it the v12 broken image vanished from the node stream and the tool emitted the
  factually wrong (and forbidden) `missing_image`. A narrow, documented exception to §4.3's
  non-empty-bbox visibility rule.
- **D14** — `nearestHeading` prefers the enclosing `<section>`'s own first heading when the
  document-order-preceding heading lies outside that section. Spec §5 defines the anchor set as
  the element's *locator*; the section's own title is the strictly more local locator (v12's
  image precedes its section heading in DOM order).

**Complete delta on v02 (per audit):** identical issue count (54), identical type multiset
(53 `visual_region_changed` + 1 `page_height_changed`), identical severities, confidences, and
pixel evidence. Exactly 4 issues changed anchors — 3 stat-region issues whose `nearestHeading`
moved from the hero h1 to the Problem section's own h3 (D14), and the footer ISO-27001 image
issue's `ordinalInLandmark` 4→5 (D13 surfaced a pre-existing broken footer image on both sides).
Additionally 4 issues had `seqIndexNew` shift +1 (three with only that change) — the direct
consequence of D13 adding one node to the new-side stream; `seqIndexNew` is a tool-internal
locator field explicitly excluded from the issue-id hash (spec §7.1, §5). The id changes follow
mechanically from anchors being hash inputs (§7.1) and the reordering from the §7.2 fix-value
tie-breaks.

**Known fixture quirk (recorded per audit condition 2, do NOT silently "fix"):** the golden
testbed itself contains a broken footer image on BOTH sides — the asset is vendored with a
literal percent-encoded filename (`5f1a08b8f263c3ef6e879a5b_hiya%20logo.svg`), so the decoded
request 404s. Renaming that file later would shift footer image ordinals again and would itself
require a golden change with audit.

**Audit:** golden-auditor verdict pasted verbatim below.

> VERDICT: APPROVE
>
> EXPECTATION(S): testbed/goldens/v02-banner-added.diffresult.json (re-recorded from
> testbed/.runs/v02-banner-added/diff-result.json); no change to any expected-issues.json.
>
> REASONING: This falls under approval ground 3 (approved, changelogged behavior change; golden
> re-recorded to match), with the behavior change itself verified sound. I independently diffed
> golden vs fresh: identical issue counts (54), identical type multiset (53 visual_region_changed
> + 1 page_height_changed), identical severities, confidences, and pixel evidence. The substantive
> deltas are exactly 4 anchor changes — 3 stat-region issues whose nearestHeading moved from the
> hero h1 to the Problem section's own h3 (I confirmed in testbed/golden/site/index.html that the
> "86%" stat precedes its section's heading inside <section class="section-zero">, so D14 yields
> the strictly more local locator that spec §5 defines anchors to be), and 1 footer ISO-27001
> image ordinalInLandmark 4→5 (confirmed: the first footer image is broken on BOTH sides — the
> asset is vendored with a literal percent-encoded filename 5f1a08b8f263c3ef6e879a5b_hiya%20logo.svg,
> so the decoded request 404s — and D13 now keeps it in the node stream). The id and artifact-path
> changes follow mechanically from anchors being hash inputs per spec §7.1, and the reordering from
> §7.2 tie-breaks. D13 is a narrow, documented exception to §4.3's non-empty-bbox rule required by
> G7 (spec §1) and by v12's intent (required broken_image/network_error, forbidden missing_image —
> testbed/variants/v12-image-404/expected-issues.json); I verified the fresh v12 run emits
> broken_image (error, 0.95) anchored to "Branded Call performance analytics" with no missing_image.
> No detection was weakened: v02's intent file is untouched, its required "20% off" anchor is
> present in both old and new goldens, and its forbidden assertions remain exercisable (hero nodes
> following the hero h1 keep that heading anchor under D14). Drift spot-check: fresh runs for v01,
> v13, v14, v15, v16, v17, v18 all match their recorded goldens modulo runId; only v02 differs.
>
> CONDITIONS: (1) changelog must cite D13/D14 and give the complete delta incl. seqIndexNew
> shifts — satisfied above; (2) record the broken-footer-image fixture quirk — satisfied above.

## 2026-06-11 — M7: v12 re-recorded (network_error added); v20/v21 first goldens (a11y + console differs)

**What changed:** `testbed/goldens/v12-image-404.diffresult.json` **re-recorded** from the verified
`testbed/.runs/v12-image-404/diff-result.json`; **first recording** of
`testbed/goldens/v20-console-error.diffresult.json` and `testbed/goldens/v21-a11y-lang.diffresult.json`
(two new M7 variants). No `expected-issues.json` changed for any of the three; all 21 intent checks
pass before and after, and the other 18 goldens are byte-identical.

**Why (designed M7 behavior changes, `docs/design/M7.md`):** M7 introduced three pure differs and the
capture signals they consume — `network_diff.rs` (`network_error`, `console_error`, category technical,
§11/§7.3) and `a11y_diff.rs` (`accessibility_regression`/`accessibility_improved`, category accessibility,
§11/§7.3/§9), plus axe-core in capture, a pre-navigation console listener, and a synchronous network-status
listener.

- **v12 (re-record, no expectation change):** the deleted asset
  `673b4d11b8fb561f6d7d8ccd_bc_performance-analysis_us.png` is 200 on golden (:3000) and 404 on new
  (:3012) → one new-only `network_error` (error, confidence 0.95, goal G7), anchored to the image node
  (role img, nearestHeading "Branded Call performance analytics", landmark main). v12's intent file
  **already anticipated** this (`required[0].anyOfTypes: ["broken_image","network_error"]`); the
  expectation is untouched — this is a byte-golden re-record after a designed behavior change. `network_error`
  co-detects with `broken_image` (the IMG element's load status, M3) without double-counting — distinct
  taxonomy strings for distinct facts (asset request status vs. element render), like v11's
  `broken_link`+`changed_link_target`. The differ correctly **suppresses ~7 other asset 404s that fail on
  BOTH sides** (spec §11 "failures on both are noted but not scored"); the cross-base-path correlation keys
  each request relative to its own page's URL directory (the fix that cleared the v14/v15/v16 prefix-mount
  false positives — see below). **Complete delta:** exactly +1 issue (`issue_f4fbff68cf48` network_error);
  `byType.network_error` 0→1; `fixableNow` 1→2; `scores.technical` 1.0→0.5; `topFixes` shifted one position
  (mechanical §7.2 fix-value re-sort — the new error/0.95/medium-anchor issue legitimately ranks #2);
  issues 36→37. All 36 pre-existing issues (incl. `broken_image` `issue_16e7cfbe3ecf`) are byte-identical;
  `missing_image` stays absent; status stays `fail`.
- **v20-console-error (first golden):** one new-only load-time `console.error(...)` → one `console_error`
  (warning, confidence 0.9, status `warn`); byType `{console_error:1}`, no side-effect issues (the inline
  script renders nothing). Symmetric "Failed to load resource… 404" console lines (present on both sides)
  are correctly **excluded** (routed to the network differ per M7.md §2.2, and suppressed by the new-only
  rule) — no double-count. Honest null anchor (§5); message lives in `evidence.new` + `remediation.findBy.grep`.
- **v21-a11y-lang (first golden):** new page drops `<html lang>` → axe rule set gains exactly
  `html-has-lang` (OLD 9 rules → NEW 9+html-has-lang) → one `accessibility_regression` (warning, goal G8,
  status `warn`); byType `{accessibility_regression:1}`, page-level null anchors (honest). Rule-level
  set-diff (M7.md §3); `accessibility` score 1.0→0.5.

**Blast radius (a11y differ design property, empirically confirmed by the auditor):** across all v01–v19
`.runs`, `accessibility_*`/`console_error`/`network_error` issue counts are **zero** (network_error only on
v12). Because the golden page already trips 9 axe rules, restyle variants (v03/v04/v05/v06/v19) add no new
rule, so rule-level diffing emits no spurious regression. The v14/v15/v16 URL-hygiene variants — which
serve the same site under a URL **path prefix** — initially leaked symmetric dangling-asset 404s as false
new-only `network_error`; this was a tool correlation bug (origin-root keying can't survive a base-path
change, which is exactly what a URL migration is) and was fixed by keying each request relative to its own
page's URL directory. Post-fix, v14/v15/v16 emit `network_error=0` and remain byte-identical to their
pre-M7 goldens.

**Determinism:** two-run byte-identity confirmed on v12, v20, v21 (`testbed/determinism-check.py`).

**Audit:** golden-auditor **APPROVE × 3** (2026-06-11), pasted verbatim.

```
VERDICT: APPROVE
ITEM: v12-image-404 (re-record)
REASONING: Independently diffed testbed/goldens/v12-image-404.diffresult.json against
testbed/.runs/v12-image-404/diff-result.json. The ID set difference is exactly ONE: the new
issue_f4fbff68cf48 (network_error); zero pre-existing IDs were dropped. Deep canonical diff of all 36
shared issues (sorted by id) is byte-identical — no pre-existing id, anchor, severity, confidence,
evidence, or remediation changed. The pre-existing broken_image (issue_16e7cfbe3ecf, error, G7, anchored
"Branded Call performance analytics") is unchanged and missing_image stays absent (forbidden assertion
intact). The new network_error is spec-legitimate: capture bundles confirm the asset is status 200 on old
(:3000) and 404 on new (:3012), genuinely new-only; the differ correctly SUPPRESSES the ~7 other 404s that
fail on BOTH sides (spec §11). It is NOT double-counting broken_image: broken_image = the IMG element load
status (M3 semantic), network_error = the asset request status (M7, §7.3 distinct taxonomy strings) —
co-detection like v11. The only deltas match the claim exactly: technical 1.0→0.5, fixableNow 1→2, byType
+network_error:1, issues 36→37, topFixes shifted by one (mechanical fix-value re-sort per §7.2). Intent
check PASSES; determinism-check PASSES (byte-identical two-run). v12's expected-issues.json is unchanged
and already anticipated this via anyOfTypes ["broken_image","network_error"].
CONDITIONS: Before overwriting the golden, add an M7-dated entry stating: byte-golden RE-RECORD only (no
expectation change) following the M7 network differ now emitting the new-only asset 404 the fixture's
anyOfTypes already permitted; spec §11 + §7.3 network_error + goal G7; deltas = exactly +1 network_error
(issue_f4fbff68cf48), technical 1.0→0.5, fixableNow 1→2; all 36 pre-existing issues byte-identical. Paste
this APPROVE verbatim. [satisfied by this entry]

VERDICT: APPROVE
ITEM: v20-console-error (first)
REASONING: First recording — nothing pre-existing changed. Verified the variant is a single render-nothing
edit: manifest + diff confirm only site/index.html differs (+97 bytes, an inline <script>console.error(...)
</script>), and the proposed output has exactly ONE issue, byType {console_error:1}, all other scores 1.0
except technical 0.5 — no visual/content/structure/style side-effects. The capture bundles confirm the
message "MatchyMatchy M7 seeded: newsletter widget failed to initialize" is genuinely new-only and
load-time. Critically, the symmetric "Failed to load resource… 404" console lines appear on BOTH sides and
are correctly NOT emitted as console_errors (excluded per M7.md §2.2; suppressed by the new-only rule) — so
the differ neither double-counts nor over-emits. console_error is spec-legitimate (§11; §7.3), severity
warning → status warn (proportionate, §9). The honest null anchor is correct (§5). expected-issues.json
intent is honest (required newContains the message text; forbidden accessibility_regression + network_error
— both correctly absent). Intent check PASSES; determinism-check PASSES.
CONDITIONS: Add a "new fixture" note (M7 entry) recording the first golden for v20-console-error, citing
§11/§7.3 and the v01-v19 blast-radius check (zero spurious console_error elsewhere). [satisfied by this entry]

VERDICT: APPROVE
ITEM: v21-a11y-lang (first)
REASONING: First recording — nothing pre-existing changed. Manifest + diff confirm the only change is the
removed lang="en-US" on <html> (-13 bytes, site/index.html only); bundles confirm page.lang "en-US"→"" with
zero other effects. Proposed output has exactly ONE issue, byType {accessibility_regression:1}, all scores
1.0 except accessibility 0.5. The regression is spec-legitimate (§11 "diff violation SETS →
accessibility_regression"; rule-level set diff per M7.md §3; §7.3). I verified the rule diff directly from
the bundles: OLD axe rule set = 9 rules, NEW = those same 9 PLUS html-has-lang — the new-minus-old set
difference is EXACTLY {html-has-lang}, so precisely one regression is correct. evidence carries ruleId at
both evidence.ruleId and evidence.new.ruleId, helpUrl, target ["html"]; page-level null anchors are honest.
Severity warning (§9 a11y=warn) → status warn. The blast-radius claim underpinning the whole a11y differ is
empirically confirmed: across all v01-v19 .runs, accessibility issues = ZERO everywhere; a11y appears ONLY
on v21. intent is honest (required accessibility_regression G8 newContains html-has-lang; forbidden
accessibility_improved, correctly absent). Intent check PASSES; determinism-check PASSES.
CONDITIONS: Add a "new fixture" note (M7 entry) recording the first golden for v21-a11y-lang, citing
§11/§7.3/§9 and the v01-v19 a11y blast-radius spot-check (a11y present only on v21). [satisfied by this entry]
```

## 2026-06-10 — M4: v04/v06 `fromContains`/`toContains` corrected to fix-direction semantics

**What changed:** In `testbed/variants/v04-font-family/expected-issues.json` (required[0].evidence)
`fromContains`/`toContains` swapped from `"Nunito Sans"`/`"Georgia"` to `"Georgia"`/`"Nunito Sans"`.
In `testbed/variants/v06-gradient-removed/expected-issues.json` (required[0].evidence) swapped from
`"linear-gradient"`/`"none"` to `"none"`/`"linear-gradient"`, and the required[0].note reworded so
the vars-resolved-to-rgb caveat refers to `toContains` (the old-page gradient now lives in `to`).
No other matcher fields, no forbidden assertions, and no other files changed.

**Why the old expectation was wrong:** The matchers encoded change-direction (old→new), but the
spec defines `remediation.from`/`to` in **fix direction**: `from` = current/new-page value,
`to` = desired/old-page value. `check-fixture.py` maps `fromContains`/`toContains` directly onto
`remediation.from`/`to`, so the matchers as authored contradicted the spec and were unsatisfiable
by a spec-conforming implementation. The correction was made before any style-remediation code
existed; both matchers still pin exactly the same two values with the same anchors, types, and
goals, so no detection strength was lost.

**Spec justification:** §7.1's canonical example records `evidence.old` = gradient,
`evidence.new` = `"none"`, yet remediation reads `"from": "none", "to": "linear-gradient(...)"`
under `restore_css_property`; §10.2 reads `from: "/es_MX/about", to: "/es-MX/about"` (from =
wrong/current, to = corrected). The shipped M3 code (`semantic_diff.rs`, e.g. changed_title:
`"from": nt, "to": ot`) already follows fix direction.

**Audit:** golden-auditor verdict pasted verbatim below.

> VERDICT: APPROVE
>
> EXPECTATION(S):
>   /home/admin/MatchyMatchy/testbed/variants/v04-font-family/expected-issues.json (required[0].evidence)
>   /home/admin/MatchyMatchy/testbed/variants/v06-gradient-removed/expected-issues.json (required[0].evidence + note rewording)
>
> REASONING: The spec unambiguously defines remediation from/to in fix direction: §7.1's canonical
> issue example records evidence.old = gradient, evidence.new = "none" yet remediation reads
> `"from": "none", "to": "linear-gradient(...)"` under action restore_css_property, and §10.2 reads
> `from: "/es_MX/about", to: "/es-MX/about"` (from = wrong/current, to = corrected). check-fixture.py
> (_issue_matches, lines 133-141) maps fromContains/toContains directly onto remediation.from/to, and
> the shipped M3 code (packages/analyze/src/semantic_diff.rs, e.g. changed_title at lines 212-213:
> `"from": nt, "to": ot`) already follows fix direction. Per the v04/v06 manifests the old page has
> Nunito Sans / the gradient and the new page has Georgia / none, so the current matchers encode
> change-direction (old→new) and contradict §7.1/§10.2 — approval criterion 1. This is not a
> weakening: both matchers still pin exactly the same two values with the same anchors, types, and
> goals (G1/G4 still exercised), no forbidden assertion is touched, no v04/v06 byte goldens exist
> yet, and no style-remediation code exists to "teach to" — the correction precedes implementation,
> as docs/design/M4.md §1 (lines 24-28) records.
>
> CONDITIONS: (1) docs/golden-changelog.md entry must state the old matchers encoded change-direction
> (old→new) while spec §7.1/§10.2 define remediation in fix-direction (from = current/new value,
> to = desired/old value), citing both sections, and must note no forbidden assertions or other
> matcher fields changed. (2) The v06 required[0].note must be reworded so the "computed value will
> have vars resolved to rgb stops, hence only pins 'linear-gradient'" caveat refers to toContains
> (the old-page gradient now lives in `to`). (3) M4 implementation must emit style remediation in the
> same fix direction; any deviation discovered later is a code bug, not grounds to re-flip these
> matchers.

## 2026-06-10 — M4: initial authorship of `v19-container-gap` expectation (new variant)

**What changed:** New testbed variant `v19-container-gap` (port 3019) added with its
`manifest.json` and `expected-issues.json`. One deliberate change vs golden:
`.g2-badge-feature-list { grid-column-gap: 2rem -> 0.5rem }` in `assets/css/hiya-shared.min.css`.
Required: `style_changed` with `evidence.property = "gap"`, `fromContains "32px 8px"` /
`toContains "32px"` (fix-direction per spec §7.1: from = current/new value, to = desired/old value),
anchored by rendered-uppercase `nearestHeadingContains "TRUSTED BY 1,000+ BUSINESSES"`.

**Why:** Spec §12 M4 DoD requires "a container `flex-direction`/`gap` change is detected" and no
existing variant covered a container layout property; the affected element is a grid wrapper (not a
SemanticNode leaf), exercising the M4 ancestor-channel style diff (docs/design/M4.md §4).
Authoring notes: Chromium serializes computed `gap` collapsed to `"32px"` when row == column;
the heading anchor is pinned in rendered (CSS `text-transform: uppercase`) form because capture
extracts rendered text and the fixture checker is case-sensitive.

**Audit:** golden-auditor APPROVE (initial authorship). Verdict highlights, pasted from the audit:

> VERDICT: APPROVE
> REASONING: Independently verified: the variant differs from golden by exactly one file, and a
> rule-level diff shows a single declaration change — `.g2-badge-feature-list{grid-column-gap:2rem→0.5rem}`
> — exactly as the manifest declares; the class appears once in the variant's `index.html` on the
> badge grid `div`, and the only other rules touching it live inside `max-width:991px`/`767px` media
> blocks, so no desktop-width override masks the edit. The required matcher is genuine and correctly
> directional per spec §7.1 (`fromContains "32px 8px"` would fail if the tool ever reversed fix
> direction), and the advisory run confirms `evidence.old.gap = "32px"` (Chromium collapses the
> shorthand), `evidence.new.gap = "32px 8px"`, and rendered-uppercase `nearestHeading` matching the
> case-sensitive `_substring` in `testbed/check-fixture.py`. The variant squarely exercises spec §12
> M4 DoD under G1; the forbidden `missing_*`/`changed_text` assertions are correct for a CSS-only
> edit, and the sibling `style_changed(grid-template-columns)` knock-on is a real consequence (1fr
> tracks re-resolve from 109.7px to 130.3px when 6×24px of gap is freed inside the max-width grid).
> Nothing in this expectation weakens detection or encodes "the code currently produces X".
> CONDITIONS: (1) fix manifest `edit` field — old computed gap serializes as `"32px"`, not
> `"32px 32px"`; (2) manifest `knockOnEffects` must declare the `style_changed(grid-template-columns)`
> knock-on; (3) correct the "confined to the G2 badges section" claim — the ~+23px reflow shifts
> regions below the grid; (4) changelog entry cites spec §12 M4 DoD and §7.1 fix-direction semantics.

All four conditions applied: manifest `edit` corrected to computed `"32px"`, `knockOnEffects` now
declares the `grid-template-columns` sibling issue and the below-grid reflow regions, and this entry
cites §12 / §7.1 as required.

## 2026-06-10 — M4: first recording of v03/v04/v05/v06/v19 byte goldens

**What changed:** First recording of `testbed/goldens/{v03-font-size,v04-font-family,v05-cta-style,
v06-gradient-removed,v19-container-gap}.diffresult.json`, captured from fresh runs immediately after
each variant passed its (audited) `expected-issues.json` intent check and the full `make verify`
gate went green (all 18 fixtures, all 13 pre-existing goldens byte-identical, determinism checks on
v02/v08/v06). No pre-existing golden was modified. Recorded per spec §13.3 (end-to-end goldens,
float tolerances, runId/timestamps excluded) and the /implement-milestone promotion step.

## 2026-06-10 — M5: first recording of v07-sections-swapped byte golden

**What changed:** First recording of `testbed/goldens/v07-sections-swapped.diffresult.json`,
captured from a fresh run immediately after v07 passed its (unchanged, hand-authored)
`expected-issues.json` intent check under the new M5 sequence differ, plus a two-run determinism
check (byte-identical) and a fully green `make verify` (19-variant fixture gate, all 18
pre-existing goldens byte-identical, determinism checks on v02/v08/v06). No pre-existing golden
or expectation was modified. Recorded per spec §13.3 (end-to-end goldens, float tolerances,
runId/capturedAt excluded) and the /implement-milestone promotion step.

**Why correct:** Spec §12 M5 DoD — swapped-sections fixture yields a single `component_swapped`,
not missing+added. The golden contains exactly one `component_swapped` (goal G3, structure/error,
confidence 1.0, identity-stage evidence for both blocks, remediation `reorder_components` with
`expectedOrder`), zero `missing_*`/`component_reordered`, status `fail`, structure score 0.5 per
the 1/(1+n) rule (docs/design/M5.md §2). The five info-severity `visual_region_changed` knock-ons
match the manifest's declared knock-on effects.

**Auditor verdict (golden-auditor): APPROVE.** "This is a first recording, not a weakening — no
existing expectation is touched, and the hand-authored intent file remains the authoritative gate.
The recorded output satisfies the intent and the M5 DoD (spec §12): exactly one component_swapped
(goal G3, category structure, severity error), zero missing_* for either swapped section, zero
component_reordered, status 'fail' ∈ {warn, fail}. […] No nondeterminism risk: runId and capturedAt
are the only timestamps and compare-golden.py excludes them; URLs are the fixed testbed ports,
artifact paths are relative, and the two-run byte-identity check passed." Non-blocking follow-ups
noted for M6: (a) unify `locale` stamping across emitters (`component_swapped` emits null while
visual issues emit "en-US"); (b) suffix-aware crop artifact naming for collision-suffixed issue ids.

## 2026-06-10 — M6: hygiene `statusIn` tightened `["warn","fail"]` → `["fail"]` on v14–v17

**What changed:** `statusIn` in `testbed/variants/{v14-trailing-slash,v15-locale-underscore,
v16-locale-lowercase,v17-redirect-chain}/expected-issues.json` tightened from `["warn","fail"]`
to `["fail"]`. No required matcher, forbidden assertion, anchor, or evidence field changed; no
byte golden was re-recorded (the recorded goldens already read `status: "fail"`).

**Why the old expectation was wrong (over-loose):** Spec §9 maps the Hygiene category to **fail**
under both parity profiles, including the default `content-structure` the fixture runner uses.
The implementation conforms: `scoring.rs::severity_for` returns `Error` for hygiene
unconditionally, and `compute_status` yields `warn` only when the worst post-profile severity is
`warning` — so a fixture whose single required issue is hygiene/error can never legitimately
produce `warn`. The `"warn"` band was provisional looseness from initial authorship (the §9
mapping was unimplemented then); it is unreachable for any §9-conforming implementation, so the
tightened band cannot reject a correct future tool. This discharges the standing condition from
the testbed-build entry above ("At M6 (calibration gate), revisit the ["warn","fail"] status
bands on v14/v15") per docs/design/M6.md §7. v16/v17 are included beyond the condition's literal
v14/v15 scope because they assert the identical band for the identical category-severity mapping;
leaving them loose would be the real inconsistency. (v18 was authored `["fail"]` already.)

**Closing caveat (auditor condition 2):** the tightening is valid only under the default profile
the fixture runner uses. If a future config ever applies an explicit per-type severity override
(§9: "explicit per-type severity config overrides them") to these fixtures, the band must be
revisited via this same discipline.

**Audit:** golden-auditor VERDICT: APPROVE (2026-06-10). Key verification quoted: "the band being
removed is unreachable for any §9-conforming implementation, meaning the tightened band cannot
reject a correct future tool. […] The justification is spec §9, not 'the code currently produces
fail' — the goldens merely corroborate." Both conditions are satisfied by this entry.

## 2026-06-10 — M6: v02/v03/v04/v07/v12/v19 re-recorded after suffix-aware crop naming (WP-A) and sequence-diff locale stamping (WP-B)

**What changed:** `testbed/goldens/{v02-banner-added,v03-font-size,v04-font-family,
v07-sections-swapped,v12-image-404,v19-container-gap}.diffresult.json` re-recorded from the
verified `testbed/.runs/<variant>/diff-result.json` outputs (the exact files the auditor
field-walked). No `expected-issues.json` changed; all 19 intent checks pass before and after.

**Why the old goldens were superseded:** two deliberate behavior changes, designed in
`docs/design/M6.md` §5–§6, discharging follow-ups (a) and (b) that the golden-auditor recorded
in the M5 entry above:

- **WP-A — suffix-aware crop artifacts (M6.md §5):** crop PNGs were previously written and
  `evidence.artifacts` paths stamped using the content-addressed id *before*
  `resolve_id_collisions` ran, so collision-suffixed issues (e.g. v03's `issue_a609a79ce17b-2`
  …`-19`) all referenced and successively overwrote the SAME three crop files — n−1 of n
  distinct region crops were destroyed. **The prior goldens therefore referenced
  destroyed/aliased crop files; their artifact paths were never valid evidence** (auditor
  condition 3). Crops are now written after collision resolution using each issue's FINAL id
  (lib.rs deferred `pending_crops` pass; unit test `test_deferred_crop_suffix_aware`); the v03
  run archive contains 150 per-suffix crop files with distinct bytes (auditor md5-verified).
- **WP-B — locale stamping (M6.md §6):** `sequence_diff.rs` was the only emitter leaving
  `issue.locale = null`; it now stamps the new page's `<html lang>` like every other emitter,
  per the spec §7.1 Issue example (which shows `locale` stamped). Affects exactly v07's
  `component_swapped`: null → `"en-US"`.

**Spec grounding:** §7.1 hashes only type+viewport+anchors+styleProperty and explicitly
excludes artifact paths (and locale) from the id hash — hence the auditor-confirmed invariance
of all ids, counts, type multisets, severities, confidences, anchors, and ordering across all
six files. Divergences were **field-verified as confined to** `evidence.artifacts` path strings
on collision-suffixed issues (210 path diffs, 0 pattern violations: each new path embeds the
owning issue's final suffixed id) **plus v07's single `locale` field**.

**Determinism:** two-run byte-identity check on v03 (highest collision count, 758 issues)
passed before promotion (auditor condition 2).

**Audit:** golden-auditor VERDICT (pasted verbatim, key passage): "This satisfies approval
ground 3 — re-recording after sound, designed behavior changes. I field-walked all six
golden/run pairs myself: divergences are exactly (a) evidence.artifacts path strings on
collision-suffixed issues, where I programmatically verified the old path embeds the base id
and the new path embeds the owning issue's final suffixed id (0 pattern violations across 210
artifact-path diffs), and (b) one field, v07 component_swapped.locale null → 'en-US', matching
the variant page's <html lang=\"en-US\"> and the spec §7.1 Issue example which stamps locale.
[…] The old goldens encoded a genuine defect: in v03, issue_a609a79ce17b and its 17
collision-suffixed siblings all pointed at the same three crop files, destroying n−1 distinct
region crops — the run archive now contains 150 per-suffix crop files with distinct bytes
(md5-verified), so the new goldens are strictly more truthful evidence, not weaker detection.
[…] v07's intent file asserts nothing about locale or artifacts, so no required/forbidden
assertion is affected. VERDICT: APPROVE."

## 2026-06-11 — M6: v08 re-recorded after dup-label missing_text suppression (calibration fix C1)

**What changed:** `testbed/goldens/v08-cta-removed.diffresult.json` re-recorded from the
verified `testbed/.runs/v08-cta-removed/diff-result.json`. No `expected-issues.json` changed;
all 19 intent checks pass before and after; the other 18 goldens are byte-identical.

**Defect class:** duplicate-label double-count. Webflow-style markup nests a label `<div>`
inside `<a>`/`<button>`; capture emits both a link/button node and a text node with identical
text inside the link's bbox. v08's golden encoded TWO issues for ONE removed CTA
(`missing_link` + `missing_text`, both 'Get a Demo'); on the M6 real calibration pair the same
mechanism produced false `missing_text` for elements that survived a nesting change
('Get started', 'Log in' nav CTAs). Fix (C1, emission-side): `semantic_diff::dup_label_ids`
(BTreeMap/BTreeSet, normalized-text equality + bbox containment within
`DUP_LABEL_BBOX_TOLERANCE_PX = 2.0`) gates ONLY `missing_text` emission in the missing-old
loop. An earlier stream-filtering design was rejected when the testbed caught it suppressing
v05's legitimate `style_changed` issues (the label div carries the button's computed styles) —
full account in `docs/calibration-note.md` §3 F1, which also freezes the tolerance constant
(auditor condition 2).

**Spec grounds:** §6.1 (the link node carries the label text as its own identity signal),
§6.2/§8 item 4 (one defect must not double-count), §12 M6 (tune against observed false
positives), §13.1.16 (zero-false-missing noise floor).

**Exact delta (auditor-verified):** issues 2→1 (`missing_text` `issue_2a38225dffb3` removed;
surviving `missing_link` `issue_289f0009bd4c` byte-identical in every field), content score
1/3→1/2 (mechanical `1/(1+n)`), `fixableNow`/`topFixes` 2→1, status stays `fail`. The dropped
issue's remediation was a strict subset of the surviving issue's — zero agent work lost.

**Carry-forward (auditor condition 4, non-blocking):** the suppression is old-stream /
`missing_text` only. When `added_*` emission lands post-v1, the symmetric new-stream dup-label
suppression must be implemented with its own fixture, or this false-positive class returns on
the added side.

**Audit:** golden-auditor VERDICT: APPROVE (2026-06-11), pasted verbatim (key passage): "This
is a legitimate re-record after a sound behavior change (approval ground 3), not teaching to
the test: the old golden encoded a double-count defect — two issues for one removed element —
contradicting the spec's one-defect-one-issue economy (§6.2 line 373, §8.4) and the §13.1.16
zero-false-missing noise floor that §12 M6 exists to enforce. […] I independently rebuilt from
source, reproduced the proposed output byte-for-byte, confirmed two-run determinism
(BTreeSet/BTreeMap implementation, emission-scoped to the missing-old loop only), and confirmed
the delta is exactly as claimed with the surviving issue unchanged in every field." Conditions:
(1) this entry; (2) calibration note — satisfied by `docs/calibration-note.md`; (3) full
`make verify` re-confirmation — run after this promotion, output in the M6 report; (4) the
carry-forward above.

---

## 2026-06-11 — M8: clustering populated; 7 goldens re-recorded; new `clusters` intent on v04

**What changed.** M8 (Reporters, profiles, migration loop) makes the analyze layer populate three
`DiffResult` fields that were hardcoded empty through M1–M7:
- `clusters` (was `[]`) — deterministic issue clusters per spec §7.4;
- `agentSummary.clusterCount` (was `0`);
- `agentSummary.topFixes` — now a cluster-aware work queue (a clustered group is represented by its
  one `cluster_…` id instead of N member issue ids), per §7.2/§7.4 ("one work item, not hundreds").

Three change classes in this entry:
1. **Byte-golden re-records (7 variants):** `v02-banner-added`, `v03-font-size`, `v04-font-family`,
   `v05-cta-style`, `v07-sections-swapped`, `v12-image-404`, `v19-container-gap`. These are the only
   variants where at least one group reaches `clusterMin = 3`. The other 14 goldens were **not**
   re-recorded — M8 produces zero change for them (no group clusters), and `runId`/`capturedAt` are
   excluded from comparison, so they keep passing untouched.
2. **Intent-tier addition on `v04-font-family/expected-issues.json`:** a new `clusters.required`
   assertion pinning exactly one cluster with `sharedProperty: "font-family"`,
   `memberType: "style_changed"`, `minMembers: 3`, `exactlyOne: true`. This is a **new** assertion
   for a new feature (the file already flagged it as pending: "one cluster expected once M8
   clustering lands"), not a correction of a prior expectation.
3. **Testbed schema extension** (`testbed/schemas/expected-issues.schema.json`): added a `clusters`
   property + `clusterMatcher` `$def` so the intent file above validates. Strictly additive;
   `additionalProperties: false` preserved on every matcher; no existing constraint removed.

**Why the old goldens are superseded.** This is a spec-mandated M8 behavior addition, not the tool
being taught to a test. Spec §7.4 (deterministic clustering: group by `type`+changed-style-property
**or** `type`+landmark when group size ≥ `clusterMin` default 3), §7.2 (issue/cluster ordering by
fix value), §12 M8 DoD ("a seeded global-style defect produces **one cluster**"), §13.1 fixture 15
("Global stylesheet defect → one cluster referencing all member issues"). Design + the
property-takes-precedence-over-landmark partition rule: `docs/design/M8.md` §2.

**Confined diff (independently auditor-verified).** For all 7 re-recorded goldens, a JSON-aware diff
(after stripping `runId`/`capturedAt`) shows `issues`, `status`, `scores`, `suppressed`, `viewports`,
`determinism`, and all of `agentSummary` **except** `clusterCount`/`topFixes` are **byte-identical**
to the prior goldens. The only deltas are within `clusters`, `agentSummary.clusterCount`, and
`agentSummary.topFixes`. v04 spot-check: all 454 `style_changed`/`font-family` issues land in one
`sharedProperty:"font-family"` cluster (no landmark-cluster duplication of those members) — the
literal "one cluster referencing all member issues" of §13.1 fixture 15.

**Correction to M8.md §8.** That section originally asserted "no golden-auditor verdict is required"
for this milestone on the grounds that only byte goldens were re-recorded. That was wrong: this set
**also adds a new intent assertion to `v04-font-family/expected-issues.json`**, which is an
intent-tier edit and therefore requires both this changelog entry and a pasted golden-auditor APPROVE
(CLAUDE.md "Golden discipline"). M8.md §8 has been corrected accordingly.

**Audit (golden-auditor, 2026-06-11).** First pass returned REJECT on a **purely procedural** ground
(this changelog entry was missing and M8.md §8 denied the audit gate applied); it verified all four
substantive dimensions clean and stated the work is approvable once recorded. Key passages pasted
verbatim:

```
(A) For every one of the 7 re-recorded goldens, a JSON-aware diff (after stripping runId/
capturedAt) shows issues, status, scores, suppressed, viewports, determinism, and all of
agentSummary except clusterCount/topFixes are byte-identical to HEAD; the only deltas are
confined to clusters, agentSummary.clusterCount, and agentSummary.topFixes, exactly as claimed.
(B) Every new cluster is internally consistent with spec §7.4: content-addressed id
(cluster_+12 hex), member count ≥ clusterMin=3, exactly one shared key, sorted member ids, the
clusters form a partition (no member reused), property clusters hold only same-type/same-property
style-category members, landmark clusters only same-type/same-landmark members, clusterCount==
array length, array ordered (count desc, id asc), and the §7.4/M8.md §2.2 property-precedence
rule is observable. The v04 spot-check is exact: all 454 style_changed/font-family issues fall in
a single sharedProperty:"font-family" cluster with no landmark-cluster duplication.
(C) The v04 clusters.required edit is purely additive (+11 −0), strengthens coverage, pins only
the spec-relevant invariants (shared key, member type, exactlyOne, minMembers=3) and not brittle
counts/scores, and my adversarial negative tests confirm it is non-vacuous (it fails on split-
cluster, removed-cluster, below-minMembers, and memberType mismatch).
(D) The schema extension is strictly additive — no removed paths, issueMatcher/statusValue/
top-level required unchanged, additionalProperties:false preserved on both the new clusters block
and the clusterMatcher def.
… Once the changelog entry (conditions 1–2) is added, the substantive work is approvable — A
through D all hold and the assertion is a genuine, non-vacuous strengthening of fixture-15
coverage.
```

Conditions 1–3 from that verdict are satisfied by this entry (changelog record + pasted verdict)
and the M8.md §8 correction above. Substantive verdict: APPROVE (A–D all hold); the sole blocker was
this paperwork.

---

## 2026-06-11 — Testbed port-range migration 3000–3021 → 47000–47021; v11 golden re-recorded

**What changed.** The testbed's fixed port range was migrated by a uniform `+44000` offset
(golden `3000→47000`, variants `3001..3021 → 47001..47021`) to escape a collision with an
unrelated long-running `next-server` occupying port 3001 on the build machine. User-directed
("migrate the port range to a less used range. It is okay to change the code or config"). The
move touched only testbed/build wiring — `serve.py` (`PORT`), `manifest.json` (`port` +
`urlUnderTest`), `run-all.py` (`GOLDEN_PORT`), `check-fixture.py`/`check-m8.py` (`GOLDEN_URL`),
`manifest.schema.json` (port `minimum`/`maximum` 3001/3099 → 47001/47099 and the `urlUnderTest`
pattern `[0-9]{4}` → `[0-9]{5}`) — plus a `localhost:30NN → localhost:47NN` substitution inside
the goldens and the v14/v17 `expected-issues.json`. No site content was touched (site assets
contain zero `localhost:30NN` refs; bare 30xx numbers in floats/issue-ids were deliberately left
alone — the substitution is scoped to the unambiguous `localhost:port` form). Transform encoded in
`scripts/migrate-ports.py`; 163 substitutions.

**Goldens: substitution sufficed for 20 of 21.** After the `localhost:port` substitution, fresh
new-port captures match the recorded goldens for 20 variants (the 16 that matched immediately, plus
v01/v08/v09 which match on any flake-free run — see the pre-existing srcset note below). **Only
`v11-broken-link` required a fresh re-record**, because its issue `id`s are content-addressed
SHA-256 hashes over `anchors.href` (which embeds the port — `packages/analyze/src/issue.rs::compute_issue_id`),
so substituting the URL strings left the derived hashes stale. The re-record changed exactly the
three ids `4b9058597867→97d291f9bb0d`, `8f3cdee4603f→eb914dfb7592`, `4e86118dfdea→bb5b728ea168`
(and the topFixes/viewports ordering keyed off them). Issue count (3), types (`broken_link`,
`changed_link_target`, `url_protocol_downgrade`), severities, evidence, anchors, scores, and
`status:fail` are unchanged. v11 is deterministic (two-run byte-identity) and the re-recorded golden
matches a fresh run.

**Spec justification.** CLAUDE.md "Golden discipline" re-record clause (an approved, user-directed
config change genuinely alters output → golden re-recorded to match) and spec §15 determinism
(content-addressed ids are stable under identical inputs and *must* change under changed inputs —
the resolved URL is a changed input). No expectation was weakened; v11's intent file
(`required: broken_link`, `forbidden: missing_link`) and manifest goals G2+G7 remain fully exercised.

**Pre-existing issues surfaced but NOT addressed here (out of migration scope).** The full
`make verify` is currently blocked by the **already-documented srcset-404 flake**
(`docs/issue-v08-srcset-404-flake.md`): four unvendored `-p-NNN.webp` images 404 intermittently,
producing a spurious `network_error` on v01/v08/v09 (and, with the related run-to-run `srcset`
visual variance, an unstable `visual_region_changed` count on v04). This is port-independent and
pre-existing — proven by clean runs matching the substituted goldens. Per that doc, the network_error
must **not** be blessed into a golden; the fix is to vendor the four images. No golden was re-recorded
to accommodate the flake.

**Audit (golden-auditor, 2026-06-11): APPROVE**, pasted verbatim:

```
VERDICT: APPROVE
EXPECTATION(S):
- testbed/goldens/v11-broken-link.diffresult.json (re-recorded golden, the change under review)
- Compared against /tmp/v11-golden-HEAD.json (pre-migration committed golden)
REASONING: This is the legitimate re-record case under CLAUDE.md "Golden discipline" / spec §15: an
approved, user-directed config change (port migration) genuinely alters output, and the golden is
re-recorded to match. I verified, not trusted, every difference. After reversing the port substitution
(localhost:3011->47011, localhost:3000->47000), the three claimed id substitutions, and blanking runId,
the normalized new file is byte-identical to the original. The raw unified diff shows every changed line
falls into exactly the three claimed buckets: the oldUrl/newUrl/runId header, the three content-addressed
ids (in agentSummary.topFixes, viewports[].issues[], and issues[].id — the only ordering keyed off them),
and port substrings inside URL fields (message, anchors.href, evidence URLs, remediation findBy.grep/from/
to). I independently reproduced all six ids from packages/analyze/src/issue.rs compute_issue_id (SHA-256
over a U+001F-joined canonical string that includes anchors.href), using the real anchor values; all six
matched exactly, proving the id deltas are necessary consequences of the port change and not values pasted
to force a pass. There is zero semantic drift: issue count (3), types (broken_link/changed_link_target/
url_protocol_downgrade), severities, confidences, scores, status: fail, anchor text/role/landmark/heading/
ordinal, css selectors, bboxes, seqIndexes, match sub-scores (0.945; accName/href/text), determinism, and
remediation actions are all unchanged. The v11 intent tier (expected-issues.json required broken_link,
forbidden missing_link) and manifest goals G2+G7 remain fully exercised, so no real defect detection was
weakened.
CONDITIONS: Add a docs/golden-changelog.md entry (dated 2026-06-11) recording: (1) what changed — testbed
port migration 3000-3021 -> 47000-47021 (+44000) to avoid a port-3001 collision, user-directed; 20/21
goldens updated by pure localhost:30NN->localhost:47NN string substitution; (2) why v11 alone required a
fresh re-record — its issue ids are content-addressed SHA-256 hashes over anchors.href (which embeds the
port), so substituting URL strings left the derived id hashes stale; the three ids changed
4b9058597867->97d291f9bb0d, 8f3cdee4603f->eb914dfb7592, 4e86118dfdea->bb5b728ea168; (3) spec justification
— CLAUDE.md "Golden discipline" re-record clause and spec §15 determinism; (4) paste this APPROVE verdict.
No code fix is required; the id-recompute behavior is correct as designed.
```

---

## 2026-06-11 — srcset-404 testbed defect fixed; v04-font-family golden re-recorded

**What changed.** Executed the fix in `docs/issue-v08-srcset-404-flake.md`: vendored the four
unvendored responsive-image `srcset` variants (`67caf62e…_A-LIGN_ISO-27001-p-500.webp` and
`691ca7f9…_Case Studies_BCLC_hero image-p-{500,800,1080}.webp`) into all 22 site dirs as
byte-for-byte copies of their base images — 88 files. The three BCLC variants are written with
**real spaces** in their filenames (the on-disk base has literal `%20`, but the server URL-decodes
the requested path, so only real-space files actually serve 200). All four candidates now return
200 on every variant server, so the flaky new-only `network_error` can no longer fire. No existing
site file was modified or renamed; no code changed. Then **`testbed/goldens/v04-font-family.diffresult.json`
was re-recorded** from a verified, deterministic fresh run.

**Why the old v04 golden was superseded.** It was recorded while the BCLC hero `<img>` was broken
(404). A broken image renders its **alt text**, and that alt text was subject to v04's global
`font-family` swap (Nunito Sans → Georgia), contaminating the diff with artifacts that vanish once
the image renders. The repaired-hero output is strictly more truthful (the page renders as intended).
Spec/discipline grounds: CLAUDE.md "Golden discipline" re-record clause (an approved testbed-defect
fix genuinely alters output → golden re-recorded) and spec §3.3/§15 determinism; the fix is the
issue-doc's own fix-plan step 1.

**Exact delta (independently auditor-verified).** Net 515 → 486 issues. `style_changed` 454 → 454
(count identical) — all 454 are `font-family` Nunito Sans → Georgia in both goldens; 6 differ only in
`bboxNew` (downstream y-shift from the now-rendered hero), so their content-addressed ids rotated on
geometry alone with identical text/heading/landmark/role/evidence/remediation. Removed:
`visual_region_changed` 60 → 32 (−28 alt-text-vs-image artifacts) and `page_height_changed` 1 → 0
(−1 alt-text reflow artifact). `status` stays `warn`; `suppressed` unchanged; scores identical except
`visual` 0.862 → 0.979 (the natural consequence of the cleaner, artifact-free visual diff). The
required G1 anchor (`Spam, robocalls, and scams` → Georgia) is present and the forbidden H1
font-family change absent, in both goldens — intent fully exercised.

**Blast radius.** Re-running the full suite after vendoring, **only v04's golden changed**; the other
20 goldens remained byte-identical (the alt-text artifact only produced a *diff* on v04, whose change
affects text rendering — for identical/other variants the broken hero rendered identically on both
sides). This discharges the issue-doc fix-plan step 3 ("confirm the repair did not silently mutate
other goldens") for the current suite. Determinism: three consecutive fresh v04 captures were
byte-identical (486/32/0/0).

**Audit (golden-auditor, 2026-06-11): APPROVE**, pasted verbatim:

```
VERDICT: APPROVE
EXPECTATION(S): /home/admin/MatchyMatchy/testbed/goldens/v04-font-family.diffresult.json
(re-recorded vs /tmp/v04-golden-prererecord.json)
REASONING: This is a legitimate re-record under CLAUDE.md golden discipline rule 3 / spec §3.3: the
hero <img> was 404ing and rendering as alt text, so the repaired-hero page produces correct layout —
an approved testbed-defect fix documented in docs/issue-v08-srcset-404-flake.md (root cause:
unvendored srcset variants; fix plan step 1). Every difference is attributable to the repair with no
weakening: all 454 style_changed are font-family and all 454 deltas are exactly "Nunito Sans"→Georgia
in both goldens (0 non-conforming), the required G1 anchor "Spam, robocalls, and scams"→Georgia is
present, and the forbidden H1 "Reach more customers" font-family change is absent in both. The 6
"anchor shifts" are 1:1 same-text/heading/landmark/role pairs whose only locator delta is bboxNew
(downstream y-shift from the now-rendered hero), with identical evidence, message, and remediation —
so the content-addressed id rotated purely on geometry, not semantics; visual_region_changed 60→32
and page_height_changed 1→0 are the expected removal of the alt-text-vs-image visual artifact, not a
weakened detector. G1 traceability (spec §1) and the intent file
testbed/variants/v04-font-family/expected-issues.json are still fully exercised.
CONDITIONS: Add a docs/golden-changelog.md entry recording: v04-font-family golden re-recorded after
the srcset-404 hero-repair (cross-reference docs/issue-v08-srcset-404-flake.md); old expectation was
contaminated by an unvendored-asset testbed defect (broken hero rendered as alt text); net 515→486
issues = removal of 28 visual_region_changed + 1 page_height_changed alt-text artifacts plus 6
bbox-only id rotations, with zero font-family/G1 loss. Paste this APPROVE verdict into that entry.
Also confirm the same hero repair did not silently mutate other early-milestone goldens (issue-doc
fix-plan step 3) — out of scope for this single-golden audit but required before tagging v0.1.0.
```

Condition satisfied by this entry; the blast-radius paragraph above records the full-suite
confirmation that only v04 changed.

## 2026-06-11 — 12-bug fix session: contract v1.1, all 21 goldens re-recorded

**What changed.** All 21 `testbed/goldens/*.diffresult.json` re-recorded from
`testbed/.runs/<variant>/diff-result.json` after the field-test bug-fix session
(reports in `docs/bugs/`, 5-whys analysis and work packages in
`docs/bugs/ROOT-CAUSE-AND-PLAN.md`). **No `expected-issues.json` file was modified** —
all 21 intent checks pass unmodified against the new outputs, check-m8 passes, and the
determinism spot-check passed 3/3 (`python3 testbed/determinism-check.py` on
v02-banner-added, v08-cta-removed, v06-gradient-removed — byte-deterministic).

Every golden delta falls into six classes:

1. **Contract v1.1 additive fields** (every variant): `warnings: []`, `scopedTo: null`,
   `outOfScope: {count:0, ids:[]}`, `scores.byLandmark`,
   `determinism.{old,new}.integrity`; `schemaVersion` 1.0→1.1. Bugs p0-01 (warnings
   channel), p1-06 (per-landmark scores), p1-03/p0-01 (capture-integrity inventory).
2. **Issue-id rotations** for anchors whose href carries a query/fragment (v03 ×2,
   v04 ×3): id hashing now normalizes URLs to scheme+host+path (bug p0-02). Issue
   content unchanged; ids are now durable against volatile tracking params.
3. **Uncertain-pairing style issues now emitted** (v03 +18, v04 +9) at severity `info`
   with `evidence.match.uncertainPairing: true`, excluded from the style score
   (byte-identical style scores prove the exclusion). The old code dropped uncertain-band
   pairs entirely, citing §3.1 — but spec §3.1 (lines 173, 363) requires the uncertain
   band to be "emitted with low confidence for agent/human review … not silently
   decided". The old goldens contradicted the spec; bug p1-04 supplies the gating.
   *Recorded decision (auditor condition 2):* `severity: info` + `band: "uncertain"` +
   `uncertainPairing: true` is the low-confidence signal; the `confidence` field keeps
   detector semantics (confidence in the property delta, not the pairing). A follow-up
   may scale `confidence` by pairing score; deferred to keep this re-record audited as-is.
4. **`page_height_changed` evidence gains `sectionDeltas` + locator bboxes**
   (v02, v03, v12, v19) — per-landmark height attribution, bug p2-11. Pure enrichment.
5. **v11 per-link `url_protocol_downgrade` error→info** with remediation
   `action: "none"`: the link target is `http://localhost:47011`, where the old
   `https://localhost` rewrite advice was not actionable (bug p2-12). The variant's
   required `broken_link` (error) and forbidden assertions are untouched.
6. **Score deltas derivable from the above**: category scores now count only
   Warning-or-worse issues (info = expected/uncertain, not regression — bugs
   p1-04/p2-12), e.g. v11 hygiene 0.5→1.0, fixableNow 3→2. The decisive M2.md §5.5
   rule is preserved: v18 `scores.technical` remains 0.0 (a refactor regression that
   briefly produced 0.5 was caught against the golden and fixed before re-record —
   `test_compute_scores_status_mismatch_pins_technical_to_zero`).

**Why the old expectations were wrong / superseded.** Class 3 goldens encoded behavior
contrary to spec §3.1 (uncertain band silently dropped). Classes 1, 2, 4, 5, 6 are
approved behavior changes from the documented bug fixes; the goldens are byte baselines
re-recorded after the behavior change per CLAUDE.md golden discipline ("Re-recording
byte goldens after an approved behavior change is fine").

**Audit (golden-auditor, 2026-06-11): APPROVE**, pasted verbatim:

```
VERDICT: APPROVE
EXPECTATION(S): testbed/goldens/v01-identical.diffresult.json through
v21-a11y-lang.diffresult.json (all 21), re-recorded from testbed/.runs/<variant>/diff-result.json.
No expected-issues.json modified (verified: git status shows zero testbed/ changes).
REASONING: Every one of the byte-golden deltas falls into the six claimed classes with zero
unexplained residue, and each class is grounded in approval rule 1 or 3: the uncertain-pairing
emission (class 3) corrects a golden that contradicted spec §3.1 ("emitted with low confidence for
agent/human review rather than a coin-flip verdict" / "emitted low-confidence for review, not
silently decided"); classes 1, 2, 4, 5, 6 are approved behavior changes from the documented
12-bug root-cause plan (docs/bugs/ROOT-CAUSE-AND-PLAN.md, WPs B/D/E/G/H),
each traced to a field bug report, none rationalized as "the code currently produces X." All 21
intent checks PASS against the fresh outputs with unmodified expectations; no forbidden-issue
assertion is weakened, no required issue is dropped, no status flips, v01 control holds
(pass, 0 issues), v18 technical remains 0.0.
CONDITIONS: (1) Before copying, add the docs/golden-changelog.md entry: enumerate the six delta
classes, cite p0-01/p0-02/p1-03/p1-04/p1-06/p2-11/p2-12 + spec §3.1, note class 4 also touches
v03 (the claim listed only v02/v12/v19), record the 3/3 determinism command, and paste this
verdict. (2) File a follow-up: uncertain-pairing issues carry confidence 0.9 — identical to
matched-band issues — while §3.1 says "low confidence"; either lower the confidence field when
uncertainPairing=true or record an explicit decision that severity=info + band=uncertain is the
low-confidence signal. (3) Non-blocking contract nit: diff-result.schema.json's enum still permits
"1.0" but the new fields are unconditionally required, so 1.0 documents can no longer validate.
```

Conditions disposition: (1) this entry; (2) decision recorded in class 3 above;
(3) fixed — schema enum now `["1.1"]`.

---

## 2026-06-18 — Region-saturation rollup: contract v1.2, p01 assertion evolved, all 21 goldens re-recorded

**What changed.** Implements the region-saturation-rollup feature
(`docs/plans/2026-06-17-001-feat-region-saturation-rollup-plan.md`, units U1–U7; origin
`docs/brainstorms/2026-06-17-region-saturation-rollup-requirements.md`). Two golden-discipline
deltas:

1. **All 21 `testbed/goldens/*.diffresult.json` re-recorded — purely additive.** Each gains exactly
   three things and nothing else: `schemaVersion` 1.1→1.2, `agentSummary.regionCount: 0`, and a
   top-level `regions: []`. The single-change variants do not saturate any landmark, so every
   variant's `regions` is empty (AE4). Verified two ways: (a) a transform that inserts only those
   three fields, then schema-validates each file against `contract/diff-result.schema.json`; (b) an
   independent testbed run — all 22 servers up, `check-fixture.py` regenerated fresh runs for all 21
   variants, and `compare-golden.py` reported zero field mismatches beyond the additive fields. No
   issue/cluster/score/topFixes/byType/locator drift anywhere.

2. **`testbed/pairs/p01-hiya-number-registration/expected-issues.json` — assertion evolved (R10).**
   Replaced the raw `maxIssues: 280` cap with `maxTopLevelItems: 48`, and added a
   `regions.required[]` block asserting **exactly one `contentinfo` rollup at `minSaturation >= 0.6`**.
   The `broken_link` `required[0]` true-positive (text "See Branded Call Plans", href
   "/products/connect/pricing", minSeverity error) is **unchanged** and `forbidden` stays `[]` —
   nothing weakened. On the frozen replay: top-level items = 44 (23 standalone + 20 clusters + 1
   region), so cap 48 carries a margin of 4 (non-vacuous). The footer's 88 issues now collapse into
   the single rollup; the broken_link stays `topFixes[0]`, unswallowed in the unsaturated `main`
   landmark (AE5).

**Why the old expectations were superseded.**
- Goldens (class 1): the additive `schemaVersion` 1.1→1.2 bump makes `regions`/`regionCount`
  unconditionally required (`additionalProperties: false`), so 1.1 goldens no longer validate — the
  re-record is forced by the contract change (R4/R7; mirrors the WP-E 1.0→1.1 precedent above).
- p01 (R10): `maxIssues: 280` over-specified a raw issue count that no longer reflects triage burden.
  The rollup feature's optimization target is the *top-level work-item* count, so the expectation now
  asserts that directly, plus the calibration lock (exactly one footer rollup; `main` at 0.02 and
  banner/nav below the 10-node floor must not roll up).

**R9 reinterpretation (recorded per the plan's Key Technical Decisions).** Origin R9 says an
"error/critical-severity member inside a saturated region remains individually reachable." This
implementation deliberately narrows the *standalone-surfacing* floor to `critical` only: error-and-
below in-region members are "individually reachable" via the rollup's `memberIssueIds` drill-down
(and the rollup carries worst-member severity, so it stays high-priority in `topFixes`), **not** as
their own `topFixes` entries; only `critical` members are dual-surfaced standalone. This is grounded
in the recorded p01 footer (45 error / 38 warning / 5 info / **0 critical** members) — an `error`
floor would re-surface all 45 footer errors and defeat the rollup. The broken_link in `main` is
unaffected (outside any saturated region, never claimed).

**Verification.** `make pair CASE=p01-hiya-number-registration` PASS (schema valid; broken_link
matched; `regions[0]` contentinfo 1 region / 88 members; `maxTopLevelItems` 44 ≤ 48; status fail).
All 21 variant `check-fixture` + `compare-golden` PASS against fresh runs. `cargo test` 418 pass.

**golden-auditor verdict (pasted verbatim):**

```
VERDICT: APPROVE (conditional — the changelog entry that this verdict must be pasted into does not
yet exist; it is a hard precondition for merge)  [SATISFIED BY THIS ENTRY]

REASONING:

Goldens (all 21) — purely additive (AE4), verified line-by-line, not from the summary. Across every
one of the 21 goldens the ONLY changed content lines are exactly three: schemaVersion "1.1"→"1.2",
agentSummary.regionCount: 0, and top-level regions: []. Zero issue/cluster/score/topFixes/byType/
locator deltas anywhere. The contract schema is consistent: enum ["1.2"], regions + regionCount both
required, additionalProperties: false — so the re-record was forced by the additive bump (1.1 no
longer validates), the legitimate changelogged-behavior-change justification (WP-E precedent).

p01 expectation — not weakened; tighter, not looser. The broken_link required[0] matcher is byte-
identical to HEAD apart from its note; forbidden stays []. The maxIssues→maxTopLevelItems shift is
justified by plan R10, not tool convenience. Independently recomputed top-level items from the
recorded run: 44 = 23 standalone + 20 clusters + 1 region, so cap 48 carries a margin of 4 —
non-vacuous and not gratuitously loose.

Region assertion is a real lock, not vacuous — verified against the engine. check-fixture.py
genuinely implements regions.required[] and maxTopLevelItems: a missing region FAILs, exactlyOne
FAILs on 0 or 2+, minSaturation FAILs at sat < 0.6 - 1e-9. check-pair.py p01 returns PASS on all four
checks; the recorded run confirms main is at 0.02 and banner/nav are below the 10-node floor (one
region total), so the calibration intent is locked.

R9 relaxation is data-grounded and correctly applied. The footer rollup has 45 error / 38 warning /
5 info / zero critical members; region severity = error (stays high-priority); topFixes[0] is the
broken_link in main (real defect top of queue, unswallowed — AE5). An error floor would re-surface
45 footer errors and defeat the rollup, so "individually reachable for error-and-below via
memberIssueIds drill-down, only critical dual-surfaces" is the defensible reading. None of the
REJECT conditions are met: no real defect made undetected, no forbidden assertion removed, no matcher
broadened to vacuous, tolerance widened only on a float saturation score (allowed).

CONDITIONS: (1) Write this changelog entry [DONE — this entry]. (2) This audit covers only the
expectation/golden deltas, not a full implementation review of regions.rs/assemble_diff_result; a
separate code review of the U1–U6 implementation should occur before merge (outside golden-audit
scope) — addressed by the ce-work code-review pass.
```

---

## 2026-08-25 — Issue-id derivation fix (U2, port-parity plan R4c); re-record landed in U14

**Status: DRAFT.** This entry records a *derivation* change (`packages/analyze/src/issue.rs`), not
yet a golden re-record — every recorded `.diffresult.json` under `testbed/goldens/` still reflects
the *old* hash and is now expected to diverge (every issue id changes at least once). The batched,
audited re-record — including the mechanical id-migration pre-pass described in the port-parity
plan's U14 — happens there, with its own `golden-auditor` verdict pasted in. No golden files or
`expected-issues.json` files were touched by this change.

**What changed.** `compute_issue_id`'s canonical hash input list dropped `anchors.ordinalInLandmark`
unconditionally, and made `anchors.nearestHeading` conditional: it now contributes to the hash only
when `text`, `href`, `alt`, and `ariaLabel` are *all* absent/empty for that issue (the bare-
decorative-element case, where nothing else is left to identify it). Whenever any of those four
anchors is present, `nearestHeading` contributes nothing, even if its value changes between
captures. `resolve_id_collisions`'s suffix assignment (`-2`, `-3`, …) switched from a bbox-pixel
sort to document order: `seqIndexOld` ascending (`None` last), then `seqIndexNew` ascending (`None`
last), then the pre-existing insertion-stable tie-break. The styleProperty hash slot is otherwise
unchanged; it will also carry which-pseudo values (`"::before"`, `"::before.background-color"`) once
U10 lands, sharing the same slot.

**Why the old derivation was wrong.** `docs/bugs/p0-02-issue-ids-unstable-across-runs.md` documents
that issue-id instability has now failed *twice* against the same root cause (fixing one volatile
hash input while another survives): first tracking query parameters in hrefs (fixed by
`id_stable_url`), then `ordinalInLandmark` and `nearestHeading`. The `p01` real-pair regression
memory note records the empirical cost: only **2 of 129** issue ids survived a genuine re-capture of
a diverged live-page pair while both fields were hashed unconditionally. Both are capture-volatile
for structural reasons, not incidental bugs: `ordinalInLandmark` shifts whenever an unrelated sibling
is inserted or removed near a surviving defect (independent of whether the defect itself changed),
and `nearestHeading` is computed from "first visible heading," which itself shifts with load/
visibility state between re-captures of the same live page. Keeping either in the hash defeats the
`--baseline` accept-list's entire premise (spec §7.4): a defect that hasn't changed must keep its id
so the fix→re-run loop and baseline ledgers work. The bbox-pixel collision-suffix sort had the same
disease one level down — bbox jitters with viewport reflow and ad-tech noise independent of document
position, so which content-identical twin got which suffix was itself unstable.

**Spec justification.** `docs/prds/page-pair-diff-spec.md` §7.1 ("Issue identity") amended in the
same commit: the hash-input list now excludes `ordinalInLandmark` unconditionally and states the
`nearestHeading` conditional-inclusion rule explicitly, documents the which-pseudo-shares-the-
styleProperty-slot decision (for U10, landed ahead of need per the port-parity plan's contract-first
batching), and adds a new "Collision suffixing (document order, not bbox)" subsection recording the
document-order tie-break and the residual limitation (adding/removing a *content-identical twin*
still shifts only that twin's suffix position — collision suffixes are not a substitute for identity
when a defect type genuinely has no distinguishing anchors). The §15 invariant checklist line
restating the §7.1 inputs was updated to match.

**Test evidence (test-first, per the port-parity plan's U2 execution note).** New/rewritten unit
tests were written in `packages/analyze/src/issue.rs` and confirmed **failing** against the
unmodified derivation before implementation, then confirmed passing after:
- `test_ordinal_in_landmark_never_affects_id` — ordinal shift (simulated sibling removal) survives.
- `test_nearest_heading_excluded_when_strong_anchor_present` — heading rewrite survives when `text`
  is present.
- `test_nearest_heading_is_last_resort_disambiguator_when_bare` (converse) — heading rewrite *does*
  change the id when text/href/alt/ariaLabel are all absent.
- `test_identical_twins_suffix_by_document_order_stable_under_bbox_jitter` — three content-identical
  twins get three distinct, `seqIndexOld`-ordered suffixes, stable across two independently
  bbox-jittered re-derivations.
- `test_removing_middle_twin_keeps_first_twins_id_unchanged` — the residual limitation, demonstrated.
- `test_collision_suffix_ignores_bbox_uses_document_order` (rewrite of the pre-existing
  `test_collision_suffix_determinism`, which hard-coded bbox-sort-order expectations the new design
  deliberately invalidates) — bbox no longer determines suffix order.
- `test_pseudo_style_property_slot_distinguishes_which_pseudo` — `::before`/`::after`/
  `::before.background-image` remain three distinct ids through the shared slot.
All pre-existing `issue.rs` id tests (tracking-param stability, format, anchor strength, etc.) still
pass unmodified. Full workspace: `cargo test` — 526 tests pass (488 lib + 6 + 6 + 7 + 8 + 11 across
the integration suites), 0 failed.

**Known downstream effect (expected, not a defect).** Every issue id in every committed golden and
every Tier-3 pair's frozen expectation now hashes differently at least once (any issue whose anchors
include `ordinalInLandmark` alone, or `nearestHeading` alongside a present `text`/`href`/`alt`/
`ariaLabel`, gets a new id; collision-suffix assignment for any group that previously relied on bbox
order may also reorder). This is the sanctioned "one-time break to committed ledgers/goldens" the
port-parity plan calls out (user-approved; `baseline_stale_ids` fires loudly for now-stale ledger
entries rather than silently dropping them). `make verify` is expected to fail until U14's audited
re-record; it was intentionally **not run** for this change per the U2 brief.

**golden-auditor verdict:** _not yet requested — this is a derivation-only change; the audit is
deferred to U14 where the actual re-recorded goldens exist to audit against._

**Update (2026-08-25, U14 attempt).** `testbed/migrate-golden-ids.py` (new) recomputes each of the
21 existing goldens' issue ids under the derivation above, using only fields already present in
the golden JSON (`type`, `viewport`, anchors minus `ordinalInLandmark`, the conditional
`nearestHeading` rule, and `remediation.property` recovered as the `styleProperty` hash slot for
style-category issues), and rewrites every id reference (`issues[].id`, `clusters[].issueIds`,
`agentSummary.topFixes`, `suppressed.ids`, `regions[].memberIssueIds`) in place. Verified
byte-exact against the pre-migration goldens (`apply_mapping(original, mapping) == migrated` for
all 21 files — i.e. the migration is a pure id-substitution, zero other-field drift). All 1,418
issue ids across the 21 goldens changed (expected: the derivation change is unconditional). Fresh
builds were then captured for all 24 variants (v01–v24) and diffed by id (not array position, since
final output order tie-breaks on id and therefore reorders) against this migrated baseline: **zero
issue ids present in the golden and absent from the fresh run, and zero ids present in the fresh
run and absent from the golden, on every one of the 21 variants** — i.e. the migration script's
replication of `compute_issue_id`/`resolve_id_collisions` matches the real Rust derivation exactly,
with no residual bug. This is the empirical confirmation the derivation fix intended. At the time
this update was written the full audited byte re-record had not yet proceeded (see the entry
immediately below, which records why, and its own later resolution once the blocking regression
was fixed); this update only closes out the id-stability half of U2.

**golden-auditor verdict:**
> VERDICT: APPROVE
> EXPECTATION(S): testbed/goldens/v01–v21 (id migration in commit 502cf3e + uncommitted re-record); testbed/migrate-golden-ids.py; spec §7.1/§15 amendment
> REASONING: Approval ground 3 with the behavior change verified sound: I reproduced v06's old and new ids by hand from the two canonical forms (main issue_7114fb31c5c9 → issue_4abe5fef4ef2), confirmed packages/analyze/src/issue.rs implements exactly the amended §7.1 (ordinalInLandmark unconditionally out, nearestHeading identity-grade only when text/href/alt/ariaLabel are all absent, seqIndex-ordered collision suffixes), and confirmed the migration is a pure id substitution (positional mapping applied to main's goldens reproduces HEAD byte-exactly, 1,418/1,418 ids changed) while the fresh run's id sets match the migrated baseline exactly on all 21 variants — the two-way verification holds. The change strengthens the §7.4 baseline/identity guarantee against the documented p01 2/129 survival failure (docs/bugs/p0-02) and honestly records its residual twin-suffix limitation in the spec; detection power is untouched (ids gate identity, not emission), and all seven claimed identity-boundary tests exist in issue.rs.
> CONDITIONS: Fix the stale comment at issue.rs ~line 153 ("Sort colliders by (bboxNew.y, …)") which still describes the deleted bbox sort; commit the uncommitted matchy.rs --no-settle doc-comment update together with this re-record.

*Conditions satisfied in the re-record commit: the stale issue.rs collision-sort comment now describes document order, and the matchy.rs `--no-settle` doc-comment update is committed alongside.*

---

## 2026-08-25 — U14 golden re-record: confidence-penalty regression found, fixed, RESOLVED

**Status: RESOLVED — goldens re-recorded for real.** This entry originally recorded the re-record
as BLOCKED. It is now resolved: `packages/analyze/src/contract.rs::has_confidence_penalty()` was
fixed in commit `502cf3e` (see "The regression" / "The fix" below), a fresh 24-variant capture was
re-run against the fixed build, and `testbed/goldens/*.diffresult.json` (all 24, including
first-time recordings v22–v24) were overwritten with those fresh results. `compare-golden.py`
confirms byte-exact equality (within the standard float tolerance and `runId`/`capturedAt`
exclusion) between every golden and its source run. No `expected-issues.json` was modified.

The paragraphs below are kept as originally written (the blocking finding, for the record), with
the resolution appended at the end.

**The regression.** `packages/analyze/src/contract.rs::CaptureDeterminism::has_confidence_penalty()`
(pre-existing, predates the port-parity plan) still keys unconditionally off the legacy
`lazyLoadPass` determinism field:

```rust
pub fn has_confidence_penalty(&self) -> bool {
    self.time_frozen != StepStatus::Ran
        || self.lazy_load_pass != StepStatus::Ran
        || self.fonts_ready != StepStatus::Ran
}
```

U12 made the "full" settle stage (now the default) unconditionally report
`lazyLoadPass: "skipped"` once `settle` takes over step 8 (`stabilizer.ts:628`) — a deliberate
"legacy key kept for continuity" design, not a failure. `has_confidence_penalty()` was never
updated to account for this. The result: on every capture with settle on (the default), every
single issue of every type — not only `clickable_area_regressed`/pseudo issues on an actual settle
failure, which was the intended scope of the settle↔confidence coupling per the port-parity plan's
"Hit-test/pseudo confidence couples to settle outcome" decision — has its `confidence` multiplied
by 0.8, unconditionally, even on a fully clean capture with `settle: "ran"` and
`quiescence: "reached"` and zero warnings. Verified on `v01-identical` (a byte-static control):
`determinism.old.lazyLoadPass = "skipped"`, `determinism.old.settle = "ran"`,
`determinism.old.quiescence = "reached"`, `warnings: []` — no failure anywhere, yet every issue's
`confidence` in every other variant comes back multiplied by 0.8 (`0.9→0.72`, `0.95→0.76`, etc.,
confirmed across v06/v08/v09/v10/v11/v20/v21/v22/v23, spanning visual, content, style, hygiene,
a11y, network, and the two new port-parity types). This is silent (no warning names it) and
universal (every issue type, every variant, including the brand-new v22–v24 fixtures with no prior
baseline). Confidence is a primary-contract field an LLM consumer reads directly; a blanket,
undocumented 20% reduction defeats the signal-to-noise goal (spec, Problem Frame) and was not one
of the plan's declared drift sources.

**Disposition per CLAUDE.md golden discipline:** "the default response to a failing fixture is to
FIX THE CODE, never to edit the expectation" and "never weaken an expectation merely because the
current code can't meet it." Writing the current fresh-run confidence values into
`testbed/goldens/*.diffresult.json` would bake this regression into the permanent baseline as if it
were sanctioned drift. It is not sanctioned — it is not in the plan's expected-drift list
(a)–(f), and it fails the U14 brief's own STOP criterion ("unexplained drift = STOP and report, do
not bless"). The fix is a design decision (whether `has_confidence_penalty()` should key off
`settle`/`quiescence` instead of/in addition to `lazy_load_pass`, whether `lazy_load_pass` should
report an alias of `Ran` when superseded, or something else) that belongs to the orchestrator, not
to this triage pass.

**Everything else triaged clean — see the completion report's per-variant drift table.** With
`confidence` excluded from the comparison, all other drift classes are individually explained:
schema/`bySeverity` additive fields, the U3 built-in `letter-spacing`/`line-height` → `info`
demotion (v03 only: 351/351 changes, all on those two properties, all `warning`→`info`; v04
confirmed **zero** severity drift, matching the plan's expectation that `font-family` carries no
built-in demotion), the resulting `scores.style` shift on v03 (info excluded from category scores,
v1.1-class-6 precedent), settle/hit-test/quiescence determinism fields appearing, `lazyLoadPass`
flipping to `skipped`, and bbox/ordinal/selector/crop/`styleSim`/`regionChangedRatio` numeric
re-baselining on the variants with real style/visual diffing (v02, v03, v04, v05, v07, v12, v19) —
attributable to the settle stage now loading lazy content more thoroughly before extraction (U12)
and the larger style-property list shifting the `styleSim` pairing sub-signal (U4). No issue was
added or removed anywhere (`only_in_golden_ids` / `only_in_fresh_ids` both empty on every variant),
no `status` flipped, and v22–v24 fire exactly their intended detector and nothing else.

**The fix (commit `502cf3e`).** `has_confidence_penalty()` now treats the lazy-load *function* as
satisfied by EITHER the legacy `lazyLoadPass` step (`Ran`) OR the full settle stage (`settle ==
Some(Ran)`, which subsumes it): `lazy_load_satisfied = lazy_load_pass == Ran || settle ==
Some(Ran)`; the predicate becomes `time_frozen != Ran || !lazy_load_satisfied || fonts_ready !=
Ran`. Pre-settle bundles (`settle: None`) keep the legacy rule (a skipped `lazyLoadPass` still
penalizes when there is no settle stage to have subsumed it); a settle stage that itself
failed/timed out still fails to satisfy the function (no confidence rescue from a broken settle).
Three pinning tests added (`test_no_confidence_penalty_when_settle_subsumes_lazy_load`,
`test_confidence_penalty_legacy_lazy_load_skipped_without_settle`,
`test_confidence_penalty_when_settle_failed_and_lazy_load_skipped`). This is a narrow, targeted
fix — it does not touch the intended settle-outcome→confidence coupling for
`clickable_area_regressed`/pseudo issues (`CLICKABLE_SETTLE_DEMOTION`, `config.rs`), which is a
separate, per-detector mechanism keyed on `quiescence`/`settle` directly, not on this predicate.

**Re-verification (post-fix).** All 24 variants re-captured against the fixed build and re-triaged
against the id-migrated baseline with `confidence` isolated: **zero** confidence diffs on every one
of the 21 pre-existing variants (`0.9` stays `0.9`, `0.95` stays `0.95`, etc. — confirmed on
v01/v06/v08/v09/v10/v11/v20/v21 and spot-checked broadly); v22/v23/v24 (first-time recordings) also
carry the correct, unpenalized base confidences (`clickable_area_regressed` 0.9,
`pseudo_element_missing` 0.9) rather than the previously-observed 0.72. No other drift class
changed as a result of this fix (severity/schema/settle-numeric drift, described below, is
identical to what was observed pre-fix, as expected — the fix touches only `confidence`).

**golden-auditor verdict:** APPROVE — covered by the settle-stage default-flip verdict below,
whose EXPECTATION(S) line explicitly names this `has_confidence_penalty` fix (502cf3e) and whose
reasoning verified the fix and its three pinning tests at contract.rs:459–465/1503–1526 and
confirmed no golden carries a 0.72/0.76-band confidence (the regression was fixed, not blessed).

---

## 2026-08-25 — severity-default demotions batched into the re-record

**Status: recorded.** Describes a real, already-shipped behavior change (U3, commit `68ca301`),
now reflected in the re-recorded goldens above.

**What changed.** Built-in, evidence-annotated severity demotions ship by default:
`letter-spacing` and `line-height` style diffs demote from the profile's default `warning` to
`info` (all channels: leaf, ancestor, and — once painted — pseudo). `clickable_area_regressed` is
pinned to `error` regardless of profile (deny-listed from demotion in the other direction).

**Why the old expectation was absent, not wrong.** The pre-U3 goldens simply predate per-property
severity resolution; `docs/calibration-note.md` and issue #4's field-review evidence (hundreds of
sub-pixel letter-spacing/line-height `style_changed` issues flooding a real port comparison) is the
justification for demoting exactly these two properties by default, cited in the port-parity plan's
Key Technical Decisions ("Severity mapping"). No other property demotes by default — confirmed
empirically in the triage: `v04-font-family`'s issues (a `font-family` diff) show **zero** severity
change, matching the plan's own expectation check.

**Spec justification.** `docs/prds/page-pair-diff-spec.md` §9 (parity profiles) — profile category
defaults remain the baseline; the built-in per-property table is a documented refinement layered
underneath user `--severity-map` files, per the README's "Severity mapping" section (already
landed).

**Effect on goldens (recorded).** v03 (`letter-spacing`/`line-height` heavy): 351 of 776 issues
demote `warning`→`info`, all on exactly those two properties (7 `letter-spacing`, 344
`line-height`); `scores.style` rises (0.00141→0.00280) because info-severity issues are excluded
from category scores (v1.1 class-6 precedent); `status` unchanged (`warn`). v04-font-family
confirmed **zero** severity change. No other variant's severities change.

**Mechanical consequence (auditor condition):** v03's `agentSummary.fixableNow` drops 707→356 —
exactly the 351 demoted issues leaving the fixable set (info-severity issues are excluded from
`fixableNow`'s severity ≥ warning criterion).

**golden-auditor verdict:**
> VERDICT: APPROVE
> EXPECTATION(S): testbed/goldens/v03-font-size.diffresult.json (severity fields, scores.style, byLandmark style scores, fixableNow); packages/analyze/src/config.rs BUILTIN_PROPERTY_SEVERITY/BUILTIN_TYPE_SEVERITY
> REASONING: This is a sanctioned severity-policy change, not a weakening: spec §9 ("explicit per-type severity config overrides them") sanctions the layer, the frozen constants carry evidence comments citing issue #4's flood data and docs/calibration-note.md, user maps can override per-run, and HARD_CRITICAL_TYPES/clickable_area_regressed are deny-listed against demotion abuse. I independently diffed v03: exactly 351 warning→info confined to letter-spacing (7) + line-height (344), scores.style 0.00141→0.00280 per the v1.1 class-6 info-exclusion precedent, status unchanged, v04 zero severity drift, and no other variant's severities moved. No forbidden assertion was deleted and no intent file changed (git diff: only the three additive v22–v24 files exist); v03's required font-size matcher is unaffected by the demotion table.
> CONDITIONS: Record the mechanical v03 agentSummary.fixableNow 707→356 delta (= exactly the 351 info demotions excluded) in this entry — it is currently unaccounted for.

*Condition satisfied: the fixableNow delta is recorded above.*

---

## 2026-08-25 — schemaVersion 1.3 batch (`bySeverity` + two new issue types)

**Status: recorded.** Describes the U1 contract bump (commit `c3c4ca5`), now reflected in the
re-recorded goldens above.

**What changed.** `DiffResult.schemaVersion` "1.2"→"1.3"; `agentSummary.bySeverity` becomes a
required field (present, possibly `{}`, on every result); `clickable_area_regressed` and
`pseudo_element_missing` join the `type` enum. Mirrors the v1.1→v1.2 precedent exactly: additive,
forced re-record because `additionalProperties: false` means older documents no longer validate.

**Why the old expectation was superseded.** The additive bump makes `bySeverity` unconditionally
required per the new schema; 1.2 goldens no longer validate against `diff-result.schema.json`
as-is. This is the same forcing mechanism as the 1.1→1.2 bump (region-rollup changelog entry
above).

**Spec justification.** `docs/prds/page-pair-diff-spec.md` §7 (added in U5's commit): `byType` and
`bySeverity` are both counts over the identical post-baseline/post-scope kept set, always present,
serialized empty rather than omitted.

**Effect on goldens (recorded).** Every variant gains `schemaVersion: "1.3"` and
`agentSummary.bySeverity`; no issue-level content changes from this class alone.

**golden-auditor verdict:**
> VERDICT: APPROVE
> EXPECTATION(S): testbed/goldens/* (all 24) schemaVersion 1.2→1.3 + agentSummary.bySeverity; contract/diff-result.schema.json
> REASONING: Exact replay of the audited 1.1→1.2 precedent: the schema enum is ["1.3"], bySeverity is required under agentSummary (severityMap stays optional), and clickable_area_regressed/pseudo_element_missing are in the type enum in lockstep with spec §7.3 — so 1.2 documents no longer validate and the re-record is contract-forced (approval ground 3). I validated all 24 goldens against the schema (24/24 pass) and verified every bySeverity object equals the actual severity multiset of its issues array. Spec §7 (U5) documents byType/bySeverity as always-present counts over the identical kept set.
> CONDITIONS: None.

---

## 2026-08-25 — settle-stage default flip (numeric-drift attribution confirmed)

**Status: recorded.** Describes the U12/U13 default flip (commit `ae65b66`), now reflected in the
re-recorded goldens above.

**What changed.** `settleMode` default flips from `"legacy"` to `"full"` (§4.2 amendment above).
Every capture now runs the evolved settle stage (viewport-height scroll steps, quiescence wait,
growth cap) instead of the original lazy-load pass.

**Why the old expectation was superseded.** The port-parity plan's Key Technical Decision
("Settle ships on by default") required verifying near-zero *structural* drift (no phantom
new/removed issues) before flipping the default; that was checked on a 9-variant cross-section
during U12 and, again here, confirmed across the full 21-variant set both before and after the
confidence-penalty fix: **zero** issues appear or disappear on any of v01–v21 as a result of the
flip (id sets identical, before AND after the fix — the fix does not touch issue emission), no
`status` flip on any variant, and `confidence` is now byte-stable too (see the resolved regression
entry above).

**Confirmed field-level attribution of the numeric drift.** The flip does shift numeric/locator
evidence — `bboxOld`/`bboxNew`, `cssSelectorOld`/`New`, `ordinalInLandmark`, `nearestHeading`,
crop artifact filenames, `regionChangedRatio`/`changedPixels`, and the `styleSim` match sub-signal —
confined entirely to `locator`/`evidence` subtrees, never `type`/`category`/`id`/`status`, on
variants with lazy-loaded below-the-fold content or many style/visual comparisons (v02, v03, v04,
v05, v07, v12, v19). Two distinct, fully-explained sources, both confirmed by direct inspection
rather than inferred:
1. **Real, settle-driven layout re-baseline (the dominant source, v02–v04/v12/v19).** The settle
   stage loads lazy content more completely before extraction than the legacy pass did (the point
   of the feature), genuinely shifting bbox/ordinal/selector values for a large fraction of a
   variant's issues (e.g. v03: ~1,249 of 3,100 bbox coordinate values across 776 issues) and the
   `styleSim` pairing sub-signal (more captured style properties, U4, changes the raw ratio without
   changing which band a pairing falls into).
2. **A triage-tooling artifact, not a product behavior (small, isolated, v05).** `resolve_id_collisions`
   has no `seqIndex` to sort by for the ancestor style channel (`style_diff.rs`: "ancestors have no
   seqIndex"); `testbed/migrate-golden-ids.py`'s tie-break for that case falls back to the *migrated
   golden's post-output-sort array position* as a stand-in for the real engine's pre-sort insertion
   order (documented as a residual approximation in the script's own module docstring). For one
   ancestor-channel identical-twin pair in v05 (two `padding-left`-changed CTAs sharing the anchor
   text "Get a Demo"), that stand-in guessed the opposite suffix assignment from the real engine's
   deterministic-by-construction order (pinned by `issue.rs`'s
   `test_identical_twins_suffix_by_document_order_stable_under_bbox_jitter` — the *engine* is not
   non-deterministic here). This showed up as swapped-looking `bboxOld`/`bboxNew`/`cssSelector*`/
   `nearestHeading`/evidence values between the id-migrated triage baseline and the fresh run for
   that one twin pair — moot now that the actual fresh output (not the script's approximation) is
   the recorded golden; noted here only so a future re-triage against a *new* migrated baseline
   isn't surprised by the same artifact recurring on other ancestor-channel collision groups.

No forbidden-issue assertion was weakened, no required issue dropped, and every variant's
`expected-issues.json` intent check still passes unmodified against the re-recorded goldens.

**Spec justification.** `docs/prds/page-pair-diff-spec.md` §4.2 (settle stage, added above).

**Residual delta classes (auditor condition — enumerated with attribution):**
1. 46 `message` strings changed (region geometry wording within v02's collision group) — U2
   collision-suffix reassignment, not a detection change.
2. v02 `agentSummary.topFixes` suffix swap (`issue_06602ef43981-2` ↔ `-10`) — same U2 suffix
   reassignment.
3. v05's identical-twin pair swapped `remediation.from/to` values — same U2 suffix reassignment
   (documented twin limitation).
4. v18 gains two `capability_mismatch` warnings (`hitTests`/`pseudoElements`, `missingOn: "new"`)
   — spec §11-sanctioned warning behavior: the new side is a rendered 404 page with zero
   interactive/pseudo-painted content, so the channels are absent there.

**Non-blocking follow-up filed:** on v18, `determinism.new.hitTestProbe` reads `"ran"` while the
`capability_mismatch` warning declares the `hitTests` channel unavailable on new (the probe ran
but found zero eligible nodes, and empty maps are omitted from the bundle). The probe status and
channel-presence signal should not contradict; candidate fix is emitting an empty-but-present map
or a distinct `missingOn` reason when the probe ran with zero eligible nodes.

**golden-auditor verdict:**
> VERDICT: APPROVE
> EXPECTATION(S): testbed/goldens/v01–v21 settle-driven drift; packages/analyze/src/contract.rs has_confidence_penalty fix (502cf3e)
> REASONING: I verified the entry's core claims: zero issue-set changes, zero status flips, zero id churn on all 21 variants; the RESOLVED regression account matches the code (full settle records lazyLoadPass=skipped by design, the old predicate read it as degradation) and the fix plus its three pinning tests exist at contract.rs:459–465/1503–1526; no golden carries a 0.72/0.76-band confidence, so the regression was fixed, not blessed — exactly the golden-discipline STOP the entry describes. However the "confined entirely to locator/evidence subtrees" claim is overstated: 46 message strings (region geometry on v02's collision group), the v02 topFixes swap issue_06602ef43981-2→-10, v05's twin remediation.from/to swaps, and v18 gaining two capability_mismatch warnings (hitTests/pseudoElements missing on new) also changed — all mechanical consequences of the U2 suffix reassignment or spec-§11-mandated warning behavior, none a detection change.
> CONDITIONS: Amend this entry to enumerate the four residual delta classes above with their attributions (U2 suffix reassignment for message/topFixes/remediation swaps; spec §11 capability_mismatch for v18). File a non-blocking follow-up: v18's determinism.new.hitTestProbe reads "ran" while the warning declares the hitTests channel unavailable on new — the probe status and channel presence should not contradict.

*Conditions satisfied: the four residual delta classes are enumerated above with attributions,
and the v18 probe-status/channel-presence follow-up is filed above.*

---

## 2026-08-25 — first-time goldens v22–v24

**Status: recorded.** `v22-cta-occluded`, `v23-pseudo-rule-removed`, and `v24-scroll-reveal`
(commits `be0d3c4`, `6a93891`, `6a4824e`) have no prior golden — this is a new recording, not a
re-record, so no prior expectation is being superseded.

**What the recorded goldens show.** `v22`: exactly one `clickable_area_regressed`, severity
`error`, confidence `0.9` (unpenalized post-fix), matching its `expected-issues.json` intent.
`v23`: exactly three `pseudo_element_missing`, severity `warning`, confidence `0.9` each, matching
its intent. `v24`: zero issues (the settle-pass acceptance case — the scroll-reveal content must
not false-positive as missing content), matching its intent. All three were captured against the
fixed build (commit `502cf3e`), so none carry the confidence-penalty regression that blocked their
first recording — recorded confidence values are the correct base values (`0.9`), not the
previously-observed `0.72`.

**Spec justification.** §7.3 (taxonomy additions, above), §11 (clickable-area/pseudo-element diff
notes, above).

**golden-auditor verdict:**
> VERDICT: APPROVE
> EXPECTATION(S): testbed/goldens/{v22-cta-occluded,v23-pseudo-rule-removed,v24-scroll-reveal}.diffresult.json (first recordings); their committed expected-issues.json intent files
> REASONING: First recordings after intent-first authoring (intent files committed in be0d3c4/6a93891/6a4824e before the uncommitted goldens were recorded), so nothing pre-existing is superseded. I verified golden-intent coherence directly: v22 has exactly one clickable_area_regressed (error, 0.9) anchored to "See pricing and sign up"/pricing.html with evidence.new containing the required img:nth-of-type(1) miss-winner; v23 has exactly three pseudo_element_missing (warning, 0.9) anchored to the three non-last li texts with "content" present in evidence.old, honoring maxIssues:3 and the last-li forbidden trap; v24 is status pass with 0 issues and 0 warnings, locked by ten forbidden assertions plus maxIssues:0 that make any settle failure detectable. The intent files are non-vacuous and adversarially constructed (spared-CTA forbidden entry, dead-CSS/Webflow-fallback selection trail, documented working --no-settle negative control), and confidences are the correct base 0.9, not the pre-fix 0.72.
> CONDITIONS: None.

---

## 2026-08-25 — first-time golden v25-swiper-carousel

**Status: recorded, PENDING golden-auditor review.** `v25-swiper-carousel` has no prior golden —
this is a new recording, not a re-record, so no prior expectation is being superseded.

**What.** A permanent regression fixture for the p0-01 time-freeze capture-corruption mechanism
(`docs/bugs/p0-01-time-freeze-corrupts-baseline-capture.md`): Swiper.js v11's init sequence calls
`clock.runFor` internally, which under matchy's default frozen virtual clock previously fired a
fake timer mid-init (`re.slideTo` before the carousel DOM was ready), throwing and silently
gutting sections from the baseline capture. v25 vendors Swiper **11.2.10** locally
(`testbed/variants/v25-swiper-carousel/site/assets/vendor/`, fetched once from
`https://cdn.jsdelivr.net/npm/swiper@11.2.10/`, no CDN reference in the served page) and inserts
one new `<section class="v25-carousel-section">` mid-page — between the "Easy to use — no
integration required" and "Secure Branding: Your Brand, Only on Verified Calls" sections (siblings
#6/#7 of golden's 13 top-level `section-zero` elements) — containing a 4-slide carousel (autoplay
off, loop off, pagination dots on, initialized on `DOMContentLoaded`), reusing 4 logo images
already vendored in golden's `assets/images/`. `expected-issues.json` was authored intent-first:
15 CORRUPTION-SIGNATURE forbidden matchers (`missing_*`/`missing_link`/`changed_h1`) anchored to
distinctive text/link content spread from the hero H1 down through the sections immediately
adjacent to the insertion point to the page's last section (FAQs) — written and committed before
any matchy run, per the fixture-builder brief — plus two required matchers (`page_height_changed`;
`visual_region_changed` anchored to "Secure Branding...") pinned only *after* observing a run,
never speculated.

**What the recorded golden shows.** `status: pass`, 23 issues (22 `visual_region_changed` info +
1 `page_height_changed` info, confidence 0.9/0.95), 0 warnings. All 15 forbidden matchers pass (no
corruption signature fired); both required matchers pass. Both sides' bundles record
`timeFrozen: "ran"`, `retriedWithoutTimeFreeze: false`, `settle: "ran"`, `quiescence: "reached"` —
Swiper v11.2.10 did not crash under the frozen clock; the p0-01 mechanism does not recur with this
Swiper version/config. Capture-integrity `pre`/`post` counts are equal on both sides (old
10/103/25 heading/image/landmark; new 11/107/25), confirming no stabilization-phase content loss.
Two full independent `matchy` runs (fresh capture, not bundle replay) produced byte-identical
`diff-result.json` (excl. `runId`/`capturedAt`); `determinism-check.py` (analyze-only,
bundle-replay) also passed.

**Honest deviation from the brief's assumption, disclosed rather than hidden.** Direct bbox
inspection of the new-side bundle shows only 2 of the 4 carousel slides' text ("The AA reduced
spoofed-call complaints...", "State Farm agents saw higher answer rates...") are present as
`page.nodes` entries; the 3rd/4th slides ("Geico's contact center...", "Protect Line
customers...") are absent from the node list entirely (both their `<img>` and `<p>`). Root cause,
confirmed by bbox coordinates, is NOT a crash: with `loop:false` and slide 1 active, Swiper lays
slides out untransformed at x=120/1320/2520/3720 against a 1440px page; slides with bbox x under
the page width are kept (slide 1, slide 2) and slides at/beyond it are pruned (slide 3, slide 4) —
a capture-completeness nuance in matchy's node-extraction bbox filter, distinct from the p0-01
crash class (no exception, no retry, clean settle/quiescence, integrity pre==post on both sides).
Filed as a tracked follow-up: the §4.3 bounds test keys on bbox origin — slide 2 (x=1320, equally
visually clipped by the swiper's overflow:hidden) is kept while slide 3 (x=2520) is pruned — a
boundary worth an explicit post-v1 decision alongside the deferred carousel capability probes
(spec §2). It does not weaken the fixture's corruption-signature teeth (the 10 corruption-signature
matchers all anchor to pre-existing golden content, well within page bounds). Auditor correction
applied: ALL FOUR slide-text `missing_*` matchers are inert-by-construction — `missing_*` requires
old-side presence and the slide text exists only on the new side — with the two beyond-bounds
slides doubly inert. They are kept as documentation; the fixture's live teeth are the 10 anchored
corruption tripwires + `changed_h1` + the `maxIssues` cap + the byte golden.

**Spec justification.** `docs/prds/page-pair-diff-spec.md` §7.3 (missing_*/visual_region_changed/
page_height_changed taxonomy), §11 (visual-region emission, info severity for non-overlapping
region diffs), §3.3/§15 (byte-determinism, confirmed by the two-full-run + analyze-only checks
above). CLAUDE.md testbed conventions (variant = golden + one deliberate change; intent-first
`expected-issues.json`; required side pinned from observation, never speculation).

**golden-auditor verdict:**
> APPROVE — First-time recording, purely additive (only the changelog and a Makefile verify-line touch tracked files; nothing pre-existing weakened). Verified independently: all 10 corruption-signature anchors exist verbatim in golden's HTML and would fire via the shared matcher DSL on any p0-01-style DOM gutting; the golden (pass, 23 issues, 0 warnings, zero missing_*/changed_h1), both required matchers, the determinism blocks (timeFrozen=ran, retried=false), integrity pre==post (10/103/25 old, 11/107/25 new), and slide bboxes (x=120/1320/2520/3720 vs 1440px) all match the entry's claims, and pruning the beyond-bounds slides is conformant with spec §4.3's visibility rule with carousel probing deferred per §2. One correction required before commit: all four slide-text missing_* matchers are inert-by-construction (missing_* requires old-side presence; slide text exists only on the new side — verified against the old bundle), so the live teeth number 11 (10 anchored tripwires + changed_h1), and the parenthetical "all 15 forbidden matchers anchor to existing, well-within-bounds content" must be corrected to name only the 10 corruption-signature matchers.

*Conditions satisfied above: the parenthetical is corrected, the inert-by-construction status of
all four slide-text matchers is stated, and the bbox-pruning boundary follow-up is filed.*
