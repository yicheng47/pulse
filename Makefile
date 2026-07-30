.PHONY: build release run check test clippy fmt fmt-check verify clean-rust-stale

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

# Drop accumulated dev generations of the workspace crates, keeping
# dependency artifacts warm
clean-rust-stale:
	cargo clean -p pulse-app -p pulse-engine -p pulse-cli --profile dev
