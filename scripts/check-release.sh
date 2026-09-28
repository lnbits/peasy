#!/usr/bin/env bash
# Run from the repository root on a Linux host with Nix and KVM available.
# Does not activate a system generation or contact a model provider.
set -euo pipefail

nix develop --command bash scripts/check-rust.sh
nix shell --inputs-from . nixpkgs#cargo-audit --command cargo audit --file Cargo.lock
# Build each native check first: VM definitions import generated daemon fixtures
# which must exist before flake check's read-only aggregate evaluation.
peasy_check_system=$(nix eval --impure --raw --expr builtins.currentSystem)
peasy_checks=$(nix eval --raw ".#checks.$peasy_check_system" --apply 'checks: builtins.concatStringsSep "\n" (builtins.attrNames checks)')
while IFS= read -r peasy_check; do
  nix build --no-link -L ".#checks.$peasy_check_system.$peasy_check"
done <<< "$peasy_checks"
nix flake check --no-build -L
