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
