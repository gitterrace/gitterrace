.DEFAULT_GOAL := help
.PHONY: app-icon build install run fmt clippy lint test doc theme actions docs-gen docs-check names check site-render site-shots help bench bench-large bench-report bench-clean bench-guard browser-check site deploy stage shipped domains send-vectors preview

## build: Release binary
build:
	cargo build --release -p gitterrace

## install: Install git-terrace into ~/.cargo/bin, so git runs it as git terrace
install:
	cargo install --locked --path crates/gitterrace

## run: Serve the current repository
run:
	cargo run -p gitterrace --bin git-terrace -- .

## fmt: Check formatting
fmt:
	cargo fmt --all -- --check

## clippy: Clippy, warnings are errors
clippy:
	cargo clippy --workspace --all-targets -- -D warnings

## lint: External linters (typos, vale, gitleaks, markdownlint, shellcheck, shfmt, ruff, machete)
lint:
	sh scripts/lint-run.sh

## test: All tests
test:
	cargo test --workspace

## doc: Docs, warnings are errors
doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace

## theme: Snapshot matches its manifest, generated files match the snapshot
theme:
	python3 scripts/theme-gen.py --check
	python3 scripts/theme-logo-gen.py --check
	python3 scripts/theme-tagline-gen.py --check

## actions: Example actions work per OS profile (stubbed PATH)
actions:
	sh scripts/actions-check.sh

## docs-gen: Regenerate generated files (docs/api.md)
docs-gen:
	cargo xtask generate

## docs-check: Generated files are up to date
docs-check:
	python3 scripts/bench-report.py --check
	python3 scripts/make-targets-check.py
	cargo xtask check

## names: File, env and project names follow the conventions
names:
	python3 scripts/names-check.py

## check: Every gate
check: theme actions send-vectors names bench-guard docs-check fmt clippy lint test doc

## send-vectors: Check send.js against the shared send vectors
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

## bench: Benchmark gitterrace on the pinned corpus (small, medium, rust). Needs the network
bench: build
	python3 scripts/bench-run.py --bin target/release/git-terrace --repos control,git,rust

## bench-large: As bench, plus the linux kernel (about 30 GB, opt-in)
bench-large: build
	BENCH_LARGE=1 python3 scripts/bench-run.py --bin target/release/git-terrace --large

## bench-report: Write the newest results into docs/benchmarks.md
bench-report:
	python3 scripts/bench-report.py

## bench-clean: Remove the cached benchmark clones
bench-clean:
	@d="$${GITTERRACE_BENCH_DIR:-$$HOME/.cache/gitterrace-bench}"; \
	if [ -d "$$d" ]; then echo "removing $$d ($$(du -sh "$$d" | cut -f1))"; rm -rf "$$d"; else echo "nothing at $$d"; fi

## bench-guard: The harness still works, on a tiny generated repo (part of check)
bench-guard:
	cargo build -q -p gitterrace
	python3 scripts/bench-run.py --synthetic --bin target/debug/git-terrace >/dev/null

## browser-check: Real-browser checks of the served pages (BIN=path to git-terrace). Needs browsers, so not in check
browser-check:
	node scripts/browser-check.mjs $(if $(BIN),--bin $(BIN))

## site: Refuse to publish install lines before the repository, tap and release tag answer
site: docs-check
	python3 scripts/site-check.py release

## deploy: Ship site/ to gitterrace.dev (wrangler deploy, the Worker is in wrangler.toml)
deploy: site shipped site-render
	npx wrangler@4 deploy

## stage: Deploy without the release check, only while gitterrace.dev is blocked at the edge
stage: docs-check shipped site-render
	npx wrangler@4 deploy

## app-icon: Render the macOS icon install-handler gives gitterrace-open.app, from docs/brand/mark-dark.svg (macOS, needs browsers)
app-icon:
	node scripts/icon-render.mjs

## site-render: Render the touch icon and link-preview image from docs/brand (ignored), and the site's WebPs from target/site-shots when present (needs cwebp then)
site-render:
	node scripts/site-render.mjs

## site-shots: Retake every screenshot from a running preview (URL=http://127.0.0.1:4242): raw PNG files to target/site-shots, the README's to docs/screenshots, the site WebPs, then generate. Needs browsers, so not in check
site-shots:
	node scripts/site-shots-run.mjs $(URL)
	node scripts/site-render.mjs
	cargo xtask generate

## shipped: site/ and wrangler.toml are committed, so a deploy ships a known commit
shipped:
	@git diff --quiet HEAD -- site wrangler.toml || { echo "site/ or wrangler.toml has uncommitted changes"; exit 1; }
	@test -s site/404.html || { echo "site/404.html missing: not_found_handling has nothing to serve"; exit 1; }

## domains: Check that gitterrace.dev serves and every other host redirects to it
domains:
	python3 scripts/site-check.py domains

## help: This list
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## //' | column -t -s ':'
