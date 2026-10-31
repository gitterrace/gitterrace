.DEFAULT_GOAL := help
.PHONY: build run fmt lint test doc theme check help

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

## check: Every gate
check: theme fmt lint test doc

## help: This list
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## //' | column -t -s ':'
