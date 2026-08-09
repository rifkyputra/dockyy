.PHONY: build check test fmt

build:
	cargo build

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

test:
	cargo test --all

fmt:
	cargo fmt
