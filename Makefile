.DEFAULT_GOAL := help
.PHONY: build run fmt lint test doc theme actions docs-gen docs-check names check help bench bench-large bench-report bench-clean bench-guard

## build: Release binary
build:
	cargo build --release -p gitterrace

## run: Serve the current repository
run:
	cargo run -p gitterrace --bin gitterrace -- .

## fmt: Check formatting
fmt:
	cargo fmt --all -- --check

## lint: Clippy, warnings are errors
lint:
	cargo clippy --workspace --all-targets -- -D warnings

## test: All tests
test:
	cargo test --workspace

## doc: Docs, warnings are errors
doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace

## theme: Snapshot matches its manifest, generated files match the snapshot
theme:
	python3 scripts/theme-snapshot-check.py
	python3 scripts/theme-gen.py --check
	python3 scripts/theme-logo.py --check
	@if command -v uv >/dev/null; then uv run -q --with fonttools --with brotli python scripts/theme-tagline.py --check; \
	else echo "SKIP theme-tagline: uv not installed, the README header tagline was NOT verified"; fi

## actions: Example actions work per OS profile (stubbed PATH)
actions:
	sh scripts/actions-test.sh

## docs-gen: Regenerate generated files (docs/api.md)
docs-gen:
	cargo xtask generate

## docs-check: Generated files are up to date
docs-check:
	cargo xtask check

## names: File, env and project names follow the conventions
names:
	python3 scripts/names-check.py

## check: Every gate
check: theme actions send-vectors names bench-guard docs-check fmt lint test doc

## send-vectors: Check send.js against the shared send vectors
send-vectors:
	python3 scripts/send-vectors-check.py

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

## help: This list
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## //' | column -t -s ':'
