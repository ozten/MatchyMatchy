.PHONY: testbed-up testbed-down testbed-check build verify fixture

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
	@echo "=== 1/6  cargo build + test ==="
	cargo build --release
	cargo test

	@echo "=== 2/6  capture build + test ==="
	cd packages/capture && npm install --no-audit --no-fund && npm run build && npm test

	@echo "=== 3/6  testbed servers ==="
	python3 testbed/run-all.py check

	@echo "=== 4/6  M1+M2+M3 fixture gate ==="
	python3 testbed/check-fixture.py v01-identical
	python3 testbed/check-fixture.py v02-banner-added
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

	@echo "=== 5/6  golden comparisons ==="
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

	@echo "=== 6/6  determinism spot-check ==="
	python3 testbed/determinism-check.py v02-banner-added
	python3 testbed/determinism-check.py v08-cta-removed

	@echo ""
	@echo "=== verify: PASS ==="
