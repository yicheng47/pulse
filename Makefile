.PHONY: build release run check test clippy fmt fmt-check verify

build:
	cargo build --workspace

release:
	cargo build --release --workspace

run:
	cargo run -p pulse-app

check:
	cargo check --workspace

test:
	cargo test --workspace

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

verify: check test clippy fmt-check
