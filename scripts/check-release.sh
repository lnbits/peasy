#!/usr/bin/env bash
# Run from the repository root on a Linux host with Nix and KVM available.
# Does not activate a system generation or contact a model provider.
set -euo pipefail

nix develop -c cargo fmt --all -- --check
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
nix develop -c cargo test --workspace --locked
nix shell --inputs-from . nixpkgs#cargo-audit -c cargo audit
nix flake check -L
