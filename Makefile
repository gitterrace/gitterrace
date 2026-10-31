.DEFAULT_GOAL := help
.PHONY: build run fmt lint test doc check help

## build: Release binary
build:
	cargo build --release -p gitterrace

## run: Serve the current repository
run:
	cargo run -p gitterrace -- .

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

## check: Every gate
check: fmt lint test doc

## help: This list
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## //' | column -t -s ':'
