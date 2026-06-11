.PHONY: testbed-up testbed-down testbed-check build verify fixture

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
# verify: full CI gate (M1 set)
# ---------------------------------------------------------------------------

verify:
	@echo "=== 1/7  cargo build + test ==="
	cargo build --release
	cargo test

	@echo "=== 2/7  capture build + test ==="
	cd packages/capture && npm install --no-audit --no-fund && npm run build && npm test

	@echo "=== 3/7  testbed servers ==="
	python3 testbed/run-all.py check

	@echo "=== 4/7  M1–M8 fixture gate (issues + clusters) ==="
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

	@echo "=== 5/7  M8 acceptance (reporters, profiles, baseline) ==="
	python3 testbed/check-m8.py

	@echo "=== 6/7  golden comparisons ==="
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

	@echo "=== 7/7  determinism spot-check ==="
	python3 testbed/determinism-check.py v02-banner-added
	python3 testbed/determinism-check.py v08-cta-removed
	python3 testbed/determinism-check.py v06-gradient-removed

	@echo ""
	@echo "=== verify: PASS ==="
