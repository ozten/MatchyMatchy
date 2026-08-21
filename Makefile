.PHONY: testbed-up testbed-down testbed-check build verify fixture pair pair-add pair-refresh

# ---------------------------------------------------------------------------
# Playwright browser isolation
# ---------------------------------------------------------------------------
# Matchy keeps its Chromium in a repo-local cache instead of the shared
# ~/.cache/ms-playwright. This makes the checkout self-contained and avoids
# clobbering any other tool's browsers (e.g. agent-browser, which pins its own
# chromium build in the shared cache). Exported so every recipe — npm install's
# postinstall, capture runs spawned by matchy, the testbed harnesses — inherits
# it. See docs/playwright-setup.md.
export PLAYWRIGHT_BROWSERS_PATH := $(CURDIR)/.pw-browsers

# ---------------------------------------------------------------------------
# Testbed server management
# ---------------------------------------------------------------------------

testbed-up:
	python3 testbed/run-all.py start

testbed-down:
	python3 testbed/run-all.py stop

testbed-check:
	python3 testbed/run-all.py check

# ---------------------------------------------------------------------------
# Build: compile Rust binary + capture TS bundle
# ---------------------------------------------------------------------------

build:
	cargo build --release
	cd packages/capture && npm install --no-audit --no-fund && npm run build

# ---------------------------------------------------------------------------
# fixture: run check-fixture.py for a single variant
# Requires: make fixture VARIANT=v02-banner-added
# ---------------------------------------------------------------------------

fixture:
ifndef VARIANT
	$(error VARIANT is not set. Usage: make fixture VARIANT=v02-banner-added)
endif
	python3 testbed/check-fixture.py $(VARIANT)

# ---------------------------------------------------------------------------
# pair: run check-pair.py for a single case
# Requires: make pair CASE=p01-hiya-number-registration
# ---------------------------------------------------------------------------

pair:
ifndef CASE
	$(error CASE is not set. Usage: make pair CASE=p01-hiya-number-registration)
endif
	python3 testbed/check-pair.py $(CASE)

# ---------------------------------------------------------------------------
# pair-add: capture, freeze, and scaffold a new Tier-3 fixture
# Requires: make pair-add CASE=… URL_OLD=… URL_NEW=…
# Optional: PROFILE= VIEWPORT= HIDE= MASK=
# ---------------------------------------------------------------------------

pair-add:
ifndef CASE
	$(error CASE is not set. Usage: make pair-add CASE=p01-hiya-number-registration URL_OLD=http://… URL_NEW=http://…)
endif
ifndef URL_OLD
	$(error URL_OLD is not set. Usage: make pair-add CASE=p01-hiya-number-registration URL_OLD=http://… URL_NEW=http://…)
endif
ifndef URL_NEW
	$(error URL_NEW is not set. Usage: make pair-add CASE=p01-hiya-number-registration URL_OLD=http://… URL_NEW=http://…)
endif
	python3 testbed/pair-add.py --case $(CASE) --url-old $(URL_OLD) --url-new $(URL_NEW) $(if $(PROFILE),--profile $(PROFILE)) $(if $(VIEWPORT),--viewport $(VIEWPORT)) $(foreach s,$(HIDE),--hide $(s)) $(foreach s,$(MASK),--mask $(s))

# ---------------------------------------------------------------------------
# pair-refresh: re-capture bundles for an existing Tier-3 fixture
# Requires: make pair-refresh CASE=…
# NOTE: re-recording bundles is a golden-discipline event — add an entry to
#       docs/golden-changelog.md before committing the refreshed fixture.
# ---------------------------------------------------------------------------

pair-refresh:
ifndef CASE
	$(error CASE is not set. Usage: make pair-refresh CASE=p01-hiya-number-registration)
endif
	@echo "REMINDER: re-recording bundles is a golden-discipline event. Add an entry to docs/golden-changelog.md before committing."
	python3 testbed/pair-add.py --refresh --case $(CASE)

# ---------------------------------------------------------------------------
# verify: full CI gate (M1 set)
# ---------------------------------------------------------------------------

verify:
	@echo "=== 1/9  cargo build + test ==="
	cargo build --release
	cargo test

	@echo "=== 2/9  capture build + test ==="
	cd packages/capture && npm install --no-audit --no-fund && npm run build && npm test

	@echo "=== 3/9  testbed servers ==="
	python3 testbed/run-all.py check

	@echo "=== 4/9  M1–M8 fixture gate (issues + clusters) ==="
	python3 testbed/check-fixture.py v01-identical
	python3 testbed/check-fixture.py v02-banner-added
	python3 testbed/check-fixture.py v03-font-size
	python3 testbed/check-fixture.py v04-font-family
	python3 testbed/check-fixture.py v05-cta-style
	python3 testbed/check-fixture.py v06-gradient-removed
	python3 testbed/check-fixture.py v07-sections-swapped
	python3 testbed/check-fixture.py v08-cta-removed
	python3 testbed/check-fixture.py v09-h1-changed
	python3 testbed/check-fixture.py v10-paragraph-removed
	python3 testbed/check-fixture.py v11-broken-link
	python3 testbed/check-fixture.py v12-image-404
	python3 testbed/check-fixture.py v13-render-equivalent
	python3 testbed/check-fixture.py v14-trailing-slash
	python3 testbed/check-fixture.py v15-locale-underscore
	python3 testbed/check-fixture.py v16-locale-lowercase
	python3 testbed/check-fixture.py v17-redirect-chain
	python3 testbed/check-fixture.py v18-status-mismatch
	python3 testbed/check-fixture.py v19-container-gap
	python3 testbed/check-fixture.py v20-console-error
	python3 testbed/check-fixture.py v21-a11y-lang
	python3 testbed/check-fixture.py v22-cta-occluded
	python3 testbed/check-fixture.py v23-pseudo-rule-removed

	@echo "=== 5/9  M8 acceptance (reporters, profiles, baseline) ==="
	python3 testbed/check-m8.py

	@echo "=== 6/9  pair-add.py unit tests ==="
	python3 testbed/tests/test_pair_add.py

	@echo "=== 7/9  Tier-3 real-pair regression gate ==="
	@if [ -d testbed/pairs ] && [ -n "$$(ls -d testbed/pairs/*/ 2>/dev/null)" ]; then \
		for dir in testbed/pairs/*/; do \
			[ -f "$$dir/pair.json" ] || continue; \
			case=$$(basename "$$dir"); \
			echo "  checking pair: $$case"; \
			python3 testbed/check-pair.py "$$case" || exit 1; \
		done; \
	else \
		echo "  no Tier-3 pairs yet — skipping"; \
	fi

	@echo "=== 8/9  golden comparisons ==="
	@if [ -d testbed/goldens ] && [ -n "$$(ls testbed/goldens/*.diffresult.json 2>/dev/null)" ]; then \
		for golden in testbed/goldens/*.diffresult.json; do \
			variant=$$(basename "$$golden" .diffresult.json); \
			fresh=testbed/.runs/$$variant/diff-result.json; \
			echo "  comparing golden: $$variant"; \
			python3 testbed/compare-golden.py "$$golden" "$$fresh" || exit 1; \
		done; \
	else \
		echo "  no goldens yet — skipping golden comparison"; \
	fi

	@echo "=== 9/9  determinism spot-check ==="
	python3 testbed/determinism-check.py v02-banner-added
	python3 testbed/determinism-check.py v08-cta-removed
	python3 testbed/determinism-check.py v06-gradient-removed

	@echo ""
	@echo "=== verify: PASS ==="
