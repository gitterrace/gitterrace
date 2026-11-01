.DEFAULT_GOAL := help
.PHONY: build run fmt lint test doc theme actions generate generated check help

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
	python3 scripts/check-theme-snapshot.py
	python3 scripts/gen-theme.py --check

## actions: Example actions work per OS profile (stubbed PATH)
actions:
	sh scripts/test-actions.sh

## generate: Regenerate generated files (docs/api.md)
generate:
	cargo xtask generate

## generated: Generated files are up to date
generated:
	cargo xtask check

## check: Every gate
check: theme actions generated fmt lint test doc

## help: This list
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## //' | column -t -s ':'
