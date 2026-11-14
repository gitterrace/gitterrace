# gitterrace. `make` lists the targets a person runs; the steps `make check` composes are
# commented with `#` and left out of the list. One .PHONY line per section.

.DEFAULT_GOAL := help

# The compiler rust-toolchain.toml pins. `cargo install` warns when that file picks it
# implicitly, so install names it.
TOOLCHAIN := $(shell sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
# Where cargo puts its output: CARGO_TARGET_DIR when the shell sets it, else target.
CARGO_OUT := $${CARGO_TARGET_DIR:-target}
RELEASE_BIN := target/release/git-terrace
BENCH_REPOS := --repos control,git,rust
# The release memory budget, `make test-memory`: about 35 minutes on a 4-core runner.
MEMORY_TEST := cargo test --release --no-fail-fast -p gitterrace-web --test memory_budget

# This make's flags stay here: `make -k check` put `k` in MAKEFLAGS, and the C builds cargo
# runs (jemalloc's) read it as a target and failed. A recipe's own `make` passes its flags.
unexport MAKEFLAGS MFLAGS

##@ Build and run
.PHONY: deps setup build install preview

## deps: Install Rust and every tool the gate runs, at CI's versions (BROWSERS=1 adds browsers)
deps:
	bash scripts/dev/deps-run.sh $(if $(BROWSERS),--browsers)

## setup: Keep search indexers out of build, bench and agent dirs (once per machine)
setup:
	python3 scripts/bench/indexing-run.py --apply

## build: Release binary
build:
	@python3 scripts/bench/indexing-run.py --apply-dir "$(CARGO_OUT)"
	cargo build --release -p gitterrace

## install: Install git-terrace into ~/.cargo/bin, so git runs it as git terrace
install:
	cargo +$(TOOLCHAIN) install --locked --path crates/gitterrace

## preview: Rebuild and restart the preview (PREVIEW_ADDR, PREVIEW_REPOS="a b")
preview:
	@bash scripts/dev/preview-run.sh

##@ Check gates
.PHONY: check fmt clippy clippy-features clippy-cross lint harness-links test doc theme \
	actions send-vectors names lines docs-check gc-guard bench-guard

## check: Every gate, in the order CI runs them
check: gc-guard theme actions send-vectors names lines bench-guard docs-check fmt clippy \
	clippy-features clippy-cross lint test doc

# fmt: Check formatting
fmt:
	cargo fmt --all -- --check

# clippy: Clippy, warnings are errors. Tests marked cfg(not(debug_assertions)) compile only in
# release, so every such test target is linted there too: found by its marker, never by name.
clippy:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo clippy --release $$(grep -l 'cfg(not(debug_assertions))' crates/*/tests/*.rs \
		| sed -E 's#crates/([^/]+)/tests/(.+)\.rs#-p \1 --test \2#') -- -D warnings

# clippy-features: Clippy on each crate that has features, built without them. The workspace run
# above unifies features (dev-dependencies turn them on), so a lint that only fires in the shipped
# build, such as an item used only under a feature, would never run.
clippy-features:
	cargo clippy -p gitterrace-index -p gitterrace-types --lib --no-default-features -- -D warnings

# clippy-cross: Clippy on every target, tests included, for each target under [cross] in
# lint.toml (or CROSS="a b"; CROSS=none skips): Windows natively, Linux in Docker. Code behind a
# cfg the host never builds fails here, not on a weekly runner. A target that cannot run (no
# rustup target, no Docker) fails in CI and is skipped with a note elsewhere.
clippy-cross:
	@if [ "$(CROSS)" = none ]; then echo "clippy-cross: CROSS=none, skipped"; exit 0; fi; \
	bash vendor/schwarzwald/kit/tools/cross-clippy.sh $(CROSS); rc=$$?; \
	if [ $$rc -eq 2 ] && [ -z "$$CI" ]; then echo "clippy-cross: a target could not run, skipped"; \
	else exit $$rc; fi

## lint: External linters (typos, vale, shellcheck, ruff, pyright, ast-grep, ...)
lint: harness-links
	bash vendor/schwarzwald/kit/lint/run.sh

# harness-links: The shared skills and agents are linked and harness.env names every key.
# Skipped, with a line saying so, on a clone without the shared kit checkout.
harness-links:
	@set -e; h="$${HARNESS_KIT:-$(CURDIR)/../harness-kit}/source/harness/tools"; \
	if [ ! -d "$$h" ]; then echo "harness-links: skipped, no shared kit checkout (set HARNESS_KIT)"; exit 0; fi; \
	python3 "$$h/harness-env-check.py" schwarzwald/config/harness.env; \
	python3 "$$h/harness-link.py" --check || { echo "links missing: run python3 $$h/harness-link.py" >&2; exit 1; }

## test: All tests
# Every suite runs even after one fails, and every test binary within it (--no-fail-fast), so
# one run lists every failure; the target still fails at the end if any did. MEMORY=0 leaves
# the release memory budget out: CI runs it as a job of its own, beside the gate.
test:
	@rc=0; \
	cargo test --workspace --no-fail-fast || rc=1; \
	$(if $(filter 0,$(MEMORY)),,$(MEMORY_TEST) || rc=1;) \
	cargo test --release --no-fail-fast -p gitterrace-index --test latency || rc=1; \
	exit $$rc

# test-memory: The release memory budget alone
test-memory:
	$(MEMORY_TEST)

# doc: Docs, warnings are errors
doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace

# theme: Snapshot matches its manifest, generated files match the snapshot
theme:
	python3 vendor/schwarzwald/kit/check.py
	python3 scripts/release/site/theme-gen.py --check
	python3 scripts/release/site/theme-logo-gen.py --check
	python3 scripts/release/site/theme-tagline-gen.py --check

# actions: Example actions work per OS profile (stubbed PATH)
actions:
	sh scripts/lint/actions-check.sh

# send-vectors: Check send.js against the shared send vectors
send-vectors:
	node scripts/verify/send-vectors-check.mjs

# names: File, env and project names follow the conventions
names:
	python3 scripts/lint/names-check.py --self-test
	python3 scripts/lint/names-check.py

# lines: Source files stay under 600 lines, comments carry no process labels
lines:
	python3 scripts/lint/lines-check.py --repo .
	python3 scripts/lint/width-check.py --self-test && python3 scripts/lint/width-check.py
	python3 scripts/lint/status-words-check.py
	python3 scripts/lint/stubs-check.py --self-test && python3 scripts/lint/stubs-check.py
	python3 scripts/lint/types-check.py --self-test && python3 scripts/lint/types-check.py
	python3 scripts/dev/keep-report.py --self-test

# docs-check: Generated files are up to date, and the dev scripts pass their self-tests
docs-check:
	python3 scripts/bench/bench-report.py --check
	bash scripts/land-check.sh --self-test
	python3 scripts/release/site/site-check.py --self-test
	python3 scripts/lint/commits-check.py --self-test
	python3 scripts/lint/tracked-check.py --self-test
	bash scripts/lib/python-run.sh vendor/schwarzwald/kit/tools/golden.py --self-test
	bash scripts/verify/golden-proof-run.sh --self-test
	bash scripts/dev/linux-run.sh --self-test
	bash scripts/dev/macos-run.sh --self-test
	bash scripts/verify/docker-check.sh --self-test
	cargo xtask check

# gc-guard: Daily silent gc, then the gc self-test on a fixture tree
gc-guard:
	@python3 scripts/dev/target-run.py --auto
	@python3 scripts/dev/target-run.py --self-test

# bench-guard: The bench harness still works, on a tiny generated repo
bench-guard:
	python3 scripts/bench/indexing-run.py --self-test
	cargo build -q -p gitterrace
	python3 scripts/bench/bench-run.py --synthetic --bin $(CARGO_OUT)/debug/git-terrace >/dev/null
	bash scripts/bench/bench-budget-check.sh $(CARGO_OUT)/debug/git-terrace
	python3 scripts/bench/bench-report.py --self-test
	python3 scripts/bench/bench-quiet-lib.py
	python3 scripts/bench/speed-check.py --self-test

##@ Slow checks, not in check
.PHONY: counting-parity memory-check speed-check browser-deps browser-check browser-reap \
	docker-check

# Red until every page reads one Counted.
## counting-parity: Count parity fuzz, pages against reducers
counting-parity:
	cargo test --no-fail-fast -p gitterrace-index --test it count_parity -- --include-ignored

## memory-check: An idle server's memory stays flat as history and tree grow (about 5 minutes)
memory-check: build
	python3 scripts/bench/memory-check.py --bin $(RELEASE_BIN)
	TEST_MEMORY_WIDE=100000 cargo test --release --no-fail-fast -p gitterrace-web --test memory_budget idle_memory_is_flat_in_tree_size

## speed-check: Cold insights pages on the rust bench clone stay in budget (about 10 minutes)
speed-check: build
	python3 scripts/bench/speed-check.py --bin $(RELEASE_BIN)

## browser-deps: Install the pinned browser check deps and their Playwright browsers
browser-deps:
	npm ci --no-audit --no-fund
	PLAYWRIGHT_SKIP_BROWSER_GC=1 node node_modules/playwright-core/cli.js install chromium webkit

## browser-check: Real-browser checks of the served pages (BIN=path to git-terrace)
browser-check: browser-reap
	node scripts/verify/browser/browser-check.mjs $(if $(BIN),--bin $(BIN))

## browser-reap: A killed or throwing browser check leaves no test server behind
browser-reap:
	node scripts/verify/browser/reap-check.mjs $(if $(BIN),--bin $(BIN))

## docker-check: Build the Docker image and serve a mounted repository with it (needs docker)
docker-check:
	bash scripts/verify/docker-check.sh

##@ Bench
.PHONY: bench bench-report

## bench: Benchmark the pinned corpus (needs the network). LARGE=1 adds linux (about 30 GB)
bench: build
	$(if $(LARGE),BENCH_LARGE=1 )python3 scripts/bench/bench-run.py --bin $(RELEASE_BIN) $(if $(LARGE),--large,$(BENCH_REPOS))
	python3 scripts/bench/bench-report.py

## bench-report: Write the newest results into docs/benchmarks.md
bench-report:
	python3 scripts/bench/bench-report.py

##@ Site and release
.PHONY: docs-gen domains changelog site deploy demo demo-deploy site-preview app-icon site-render \
	site-shots shipped

## docs-gen: Regenerate every generated file (docs, README blocks) and build the site
docs-gen:
	cargo xtask generate

## domains: gitterrace.dev serves and every other host redirects to it
domains:
	python3 scripts/release/site/site-check.py domains

# Writes the section from the commits since the last tag, in place of the generated Unreleased
# section. Needs git-cliff 2.14.2.
## changelog: Prepend the release's section to CHANGELOG.md (VERSION=x.y.z)
changelog:
	python3 scripts/release/changelog-run.py
	bash vendor/schwarzwald/kit/tools/changelog-gen.sh $(or $(VERSION),$(error VERSION=x.y.z is required))

# site: Refuse to publish install lines before the repository, tap and release tag answer
site: docs-check
	python3 scripts/release/site/site-check.py release

## deploy: Build the site from a clean HEAD into target/site and ship it with wrangler deploy
deploy: shipped site site-render
	@test -s target/site/404.html || { echo "target/site/404.html missing: not_found_handling has nothing to serve"; exit 1; }
	cargo run -q -p xtask -- deploy-check
	npx wrangler@4 deploy

# demo: the pinned jq and ripgrep (scripts/lib/corpus-lib.mjs, the screenshots' trees) exported in
# public mode, then the export checks. Needs node and the network on the first run.
## demo: Build the live demo (jq and ripgrep, exported) into target/demo
demo: build
	node scripts/release/demo-export.mjs

## demo-deploy: Ship target/demo, rebuilt from a clean HEAD, to demo.gitterrace.dev
demo-deploy: shipped demo
	npx wrangler@4 deploy --config deploy/demo/wrangler.toml

# site-preview serves target/site through the same assets runtime as the deploy, so the
# headers, the redirects and the 404 page behave as on gitterrace.dev. It builds the site from
# the files as they are and writes no tracked file, so it is safe in any checkout. Needs node.
# The touch icon and the link preview come from site-render, which needs the browsers.
## site-preview: Build the website and serve it on http://127.0.0.1:4343 (SITE_PORT)
site-preview:
	cargo xtask site
	npx --yes wrangler@4 dev --ip 127.0.0.1 --port $(or $(SITE_PORT),4343)

# app-icon: Render the macOS icon install-handler gives gitterrace-open.app, from docs/brand/mark-dark.svg (macOS, needs browsers)
app-icon:
	node scripts/release/site/icon-render.mjs

# site-render: Render the touch icon from docs/brand, the link preview into target/site, and the WebPs from target/site-shots when present (needs cwebp then)
site-render:
	node scripts/release/site/site-render.mjs

# site-shots builds, serves jq and ripgrep pinned in ~/.cache/gitterrace-shots with
# scripts/release/site/site-shots/config, shoots to target/site-shots and docs/screenshots,
# builds the site the render checks its links against, renders the WebPs, then generates.
# Needs browsers (make browser-deps), so not in check.
## site-shots: Retake every screenshot (needs browsers, so not in check)
site-shots: build
	@test -f node_modules/playwright-core/index.mjs || { echo "site-shots: no browsers, run make browser-deps first" >&2; exit 1; }
	node scripts/release/site/site-shots-run.mjs
	cargo xtask site
	node scripts/release/site/site-render.mjs
	cargo xtask generate

# shipped: The tree is clean, so the site deploy builds is the one HEAD builds
shipped:
	@test -z "$$(git status --porcelain)" || { echo "uncommitted changes: commit them, a deploy builds HEAD only"; git status --short; exit 1; }

##@ Maintenance
.PHONY: gc keep

# DRY=1 only reports. BENCH=1 also removes the cached benchmark clones.
## gc: Prune cargo units and scratch output unused for DAYS (default 7, DRY=1, BENCH=1)
gc:
	python3 scripts/dev/target-run.py --days $(or $(DAYS),7) $(if $(filter 1,$(DRY)),--dry-run) $(if $(filter 1,$(BENCH)),--bench)

## keep: Every item with its prod and test callers, as TSV in target/keep-inventory.tsv
keep:
	python3 scripts/dev/keep-report.py . --config schwarzwald/config/keep-inventory.json --out target/keep-inventory.tsv

include vendor/schwarzwald/kit/configs/help.mk
