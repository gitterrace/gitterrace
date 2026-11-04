.DEFAULT_GOAL := help
.PHONY: counting-parity app-icon build install fmt clippy lint test doc theme actions docs-gen docs-check names lines check site-render site-shots help bench bench-report bench-guard memory-check browser-check site deploy shipped domains send-vectors preview gc gc-guard changelog

## build: Release binary
build:
	cargo build --release -p gitterrace

## install: Install git-terrace into ~/.cargo/bin, so git runs it as git terrace
install:
	cargo +$(TOOLCHAIN) install --locked --path crates/gitterrace

# The compiler rust-toolchain.toml pins. `cargo install` warns when that file picks it implicitly,
# so install names it.
TOOLCHAIN := $(shell sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)

#: fmt: Check formatting
fmt:
	cargo fmt --all -- --check

#: clippy: Clippy, warnings are errors
clippy:
	cargo clippy --workspace --all-targets -- -D warnings

## lint: External linters (typos, vale, gitleaks, markdownlint, shellcheck, shfmt, ruff, pyright, tsc, actionlint, ast-grep, machete)
lint:
	sh scripts/lint-run.sh

## test: All tests
test:
	cargo test --workspace
	cargo test --release -p gitterrace-web --test memory_budget
	cargo test --release -p gitterrace-index --test latency

#: doc: Docs, warnings are errors
doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace

#: theme: Snapshot matches its manifest, generated files match the snapshot
theme:
	python3 scripts/theme-gen.py --check
	python3 scripts/theme-logo-gen.py --check
	python3 scripts/theme-tagline-gen.py --check

#: actions: Example actions work per OS profile (stubbed PATH)
actions:
	sh scripts/actions-check.sh

## docs-gen: Regenerate every generated file (docs, README blocks, site)
docs-gen:
	cargo xtask generate

#: docs-check: Generated files are up to date
docs-check:
	python3 scripts/bench-report.py --check
	python3 scripts/make-targets-check.py
	python3 scripts/changelog-check.py --self-test && python3 scripts/changelog-check.py
	cargo xtask check

#: names: File, env and project names follow the conventions
names:
	python3 scripts/names-check.py

#: lines: Source files stay under 600 lines, comments carry no process labels
lines:
	python3 scripts/lines-check.py
	python3 scripts/status-words-check.py
	python3 scripts/stubs-check.py --self-test && python3 scripts/stubs-check.py

## gc: Prune cargo units and site-shots/app-icon output unused for DAYS (default 7). DRY=1 only reports, BENCH=1 also removes the cached benchmark clones
gc:
	python3 scripts/target-run.py --days $(or $(DAYS),7) $(if $(filter 1,$(DRY)),--dry-run)
	@if [ "$(BENCH)" = 1 ]; then \
		d="$${GITTERRACE_BENCH_DIR:-$$HOME/.cache/gitterrace-bench}"; \
		if [ ! -d "$$d" ]; then echo "no benchmark clones at $$d"; \
		elif [ "$(DRY)" = 1 ]; then echo "would remove $$d ($$(du -sh "$$d" | cut -f1))"; \
		else echo "removing $$d ($$(du -sh "$$d" | cut -f1))"; rm -rf "$$d"; fi; \
	fi

#: gc-guard: Daily silent gc, then the gc self-test on a fixture tree (part of check)
gc-guard:
	@python3 scripts/target-run.py --auto
	@python3 scripts/target-run.py --self-test

## changelog: Prepend the section for the release about to be tagged to CHANGELOG.md, from the commits since the last tag (VERSION=x.y.z, needs git-cliff 2.14.2)
changelog:
	sh scripts/changelog-gen.sh $(or $(VERSION),$(error VERSION=x.y.z is required))

## check: Every gate (fmt, clippy, lint, test, doc, theme, actions, send-vectors, names, docs-check, gc-guard, bench-guard)
check: gc-guard theme actions send-vectors names lines bench-guard docs-check fmt clippy lint test doc

#: send-vectors: Check send.js against the shared send vectors
send-vectors:
	node scripts/send-vectors-check.mjs

## preview: Rebuild and restart the preview on DEFAULT_ADDR (PREVIEW_ADDR overrides). PREVIEW_REPOS="a b" picks repos, default this checkout
preview:
	@set -eu; \
	addr="$${PREVIEW_ADDR:-$$(sed -n 's/.*DEFAULT_ADDR: &str = "\(.*\)";/\1/p' crates/gitterrace-config/src/schema.rs)}"; \
	log="/tmp/gt-$${addr##*:}.log"; \
	cargo build -q --release -p gitterrace; \
	pkill -f "target/release/git-terrace .*$$addr" 2>/dev/null || true; \
	sleep 0.3; \
	nohup ./target/release/git-terrace $${PREVIEW_REPOS:-$$PWD} --addr "$$addr" >"$$log" 2>&1 & \
	for _ in $$(seq 1 50); do \
		curl -s -o /dev/null -m 1 "$$addr"/ && { echo "preview up: http://$$addr ($$(git rev-parse --short HEAD))"; exit 0; }; \
		sleep 0.2; \
	done; \
	echo "preview did not come up, see $$log" >&2; exit 1

## bench: Benchmark gitterrace on the pinned corpus (small, medium, rust). Needs the network. LARGE=1 adds the linux kernel (about 30 GB)
BENCH_REPOS = --repos control,git,rust
bench: build
	$(if $(LARGE),BENCH_LARGE=1 )python3 scripts/bench-run.py --bin target/release/git-terrace $(if $(LARGE),--large,$(BENCH_REPOS))
	python3 scripts/bench-report.py

#: bench-report: Write the newest results into docs/benchmarks.md
bench-report:
	python3 scripts/bench-report.py

#: bench-guard: The harness still works, on a tiny generated repo (part of check)
bench-guard:
	cargo build -q -p gitterrace
	python3 scripts/bench-run.py --synthetic --bin $${CARGO_TARGET_DIR:-target}/debug/git-terrace >/dev/null
	python3 scripts/bench-report.py --self-test

## counting-parity: The count parity fuzz including the pages-vs-reducers properties still red until the insights pages read Counted, and the report-only day-math lint. Not in check
counting-parity:
	cargo test -p gitterrace-index --test it count_parity -- --include-ignored
	ast-grep scan -c sgconfig-report.yml

## memory-check: An idle server's memory stays flat as the history and the tree grow (about 5 minutes, so not in check)
memory-check: build
	python3 scripts/memory-check.py --bin target/release/git-terrace
	TEST_MEMORY_WIDE=100000 cargo test --release -p gitterrace-web --test memory_budget idle_memory_is_flat_in_tree_size

## browser-check: Real-browser checks of the served pages (BIN=path to git-terrace). Needs browsers, so not in check
browser-check:
	node scripts/browser-check.mjs $(if $(BIN),--bin $(BIN))

#: site: Refuse to publish install lines before the repository, tap and release tag answer
site: docs-check
	python3 scripts/site-check.py release

## deploy: Ship site/ to gitterrace.dev (wrangler deploy, the Worker is in wrangler.toml)
deploy: site shipped site-render
	npx wrangler@4 deploy

#: app-icon: Render the macOS icon install-handler gives gitterrace-open.app, from docs/brand/mark-dark.svg (macOS, needs browsers)
app-icon:
	node scripts/icon-render.mjs

#: site-render: Render the touch icon and link-preview image from docs/brand (ignored), and the site's WebPs from target/site-shots when present (needs cwebp then)
site-render:
	node scripts/site-render.mjs

## site-shots: Retake every screenshot: build, serve jq and ripgrep pinned in ~/.cache/gitterrace-shots with scripts/site-shots/config, shoot to target/site-shots and docs/screenshots, the site WebPs, then generate. Needs browsers, so not in check
site-shots: build
	node scripts/site-shots-run.mjs
	node scripts/site-render.mjs
	cargo xtask generate

#: shipped: site/ and wrangler.toml are committed, so a deploy ships a known commit
shipped:
	@git diff --quiet HEAD -- site wrangler.toml || { echo "site/ or wrangler.toml has uncommitted changes"; exit 1; }
	@test -s site/404.html || { echo "site/404.html missing: not_found_handling has nothing to serve"; exit 1; }

#: domains: Check that gitterrace.dev serves and every other host redirects to it
domains:
	python3 scripts/site-check.py domains

## help: This list
help:
	@sed -n 's/^## \([^:]*\): \(.*\)/\1\t\2/p' $(MAKEFILE_LIST) | awk -F '\t' '{printf "%-14s %s\n", $$1, $$2}'
