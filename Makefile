.DEFAULT_GOAL := help
.PHONY: build run fmt lint test doc theme actions docs-gen docs-check names check help

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
check: theme actions send-vectors names docs-check fmt lint test doc

## send-vectors: Check send.js against the shared send vectors
send-vectors:
	python3 scripts/send-vectors-check.py

## help: This list
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## //' | column -t -s ':'
