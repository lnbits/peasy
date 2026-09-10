# Reliability and recovery changes

This change addresses the export, daemon upgrade, package review, interruption
recovery and progress findings from the September 2026 review.

- **Export:** one managed module, reachable from both the wrapper and original
  host imports; bundled Peasy imports rooted at the restore destination; private output permissions;
  common secret/Git/backup exclusions; an included/excluded file inventory; and
  explicit limits on privacy, flake support and package-version reproducibility.
- **Upgrades:** immutable service identity, actual executable/Nixpkgs inspection,
  and a drain-and-exit handoff after the active generation changes. The first
  upgrade from the old daemon still requires a manual restart after work ends.
- **Package review:** candidates are verified against effective host packages.
  Proposals retain exact derivation identities, asserted in the same evaluation
  that produces the system toplevel. Removal remains possible when a managed
  attribute has disappeared from the package set.
- **Recovery:** a synced private journal precedes source writes and activation.
  Startup restores an owned interrupted build; uncertain activation blocks new
  mutations and exposes a reviewed, authorized previous-generation recovery.
  Recovery does not promise to undo arbitrary service side effects.
- **Feedback:** real authentication, evaluation, download, build and activation
  stages; request retention on errors; fresh review/retry actions; diagnostics
  copying; and status/recovery access without configuring an AI provider.

## Validation

The workspace suite passed 108 tests after removing the optional package index,
including the rebuilt Wasm guest and the
real NixOS export evaluation. Workspace Clippy with warnings denied, Rust
formatting, Nix formatting and patch whitespace checks passed. The subsequent
recovery-notice adjustment passed its targeted tests and workspace Clippy.

The native checks exercise process interruption without running destructors,
recovery boundaries, request draining and result delivery during upgrades,
fragmented progress IPC, host package verification, source rollback, export
exclusions, relative symlink boundaries and the existing authorization/token,
Wasm-policy, provider and desktop-action tests.

A separate export test invokes real NixOS evaluation on the production export
output with the production Peasy module. The sandbox VM checks production
filesystem restrictions and Polkit, changed-overlay rejection before activation,
streamed progress, ordinary host activation and local-flake activation with an
untracked managed file. Its activation target is an inert fixture. The local VM
run uses a focused release build of the daemon (34 package tests passed), with
cached CLI/Wasm artifacts only for executable-presence checks; the current
CLI/Wasm are covered by the native workspace run. The final fallback-search,
export-import and recovery-notice adjustments are covered by native checks,
not that VM package snapshot.

The combined VM run passed the sandbox, Polkit, overlay rejection and traditional
activation checks. Its final case initially attempted to reinstall an already
installed package; the fixture now resets desired state before the independent
flake case and prints unexpected proposal responses. The isolated flake run
then passed activation, progress, replay rejection and no-lock-file checks with
a longer test deadline (430 seconds for proposal plus activation on this host).
The combined suite was not repeated after these test-only corrections.
These VM runs preceded removal of the package index; the workspace checks above
cover the current direct-search implementation.

No live host generation, service or provider configuration is changed by these
checks. They do not replace the full graphical desktop/installer VM matrix or
hardware testing before a release.
