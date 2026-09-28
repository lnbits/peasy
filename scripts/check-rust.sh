#!/usr/bin/env bash
# Run inside `nix develop --command bash scripts/check-rust.sh`.
set -euo pipefail
cargo fmt --all --check
cargo build --locked --release -p peasy-engine --target wasm32-unknown-unknown
export PEASY_TEST_ENGINE="$PWD/target/wasm32-unknown-unknown/release/peasy_engine.wasm"
cargo test --locked --workspace -- --test-threads=1 --include-ignored
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo run --locked -p peasy-core --example pea_catalogue -- --check
python3 scripts/pea-catalogue.py --check
