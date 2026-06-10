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
