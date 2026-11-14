test:
	cargo test --workspace --no-fail-fast || rc=$$?
	cargo test -p it --no-fail-fast -- --test-threads=1
