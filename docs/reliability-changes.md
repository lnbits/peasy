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

### Follow-up CI fixes (2026-09-10)

Direct search now wraps the effective host package set in `legacyPackages` so
Nix skips individual failing entries, including Nixpkgs' deliberate evaluation
sentinel. The host package set itself is forced first so invalid host definitions
still return an error. A real-Nix regression test covers failing top-level and
nested entries, broken metadata, retained package metadata and invalid hosts;
it uses a dummy store so it also runs inside the package build sandbox.

The export integration test now runs as `checks.export`, explicitly included in
the ISO CI job, using the production exporter in a small Rust harness. Keeping
`pkgs.path` out of the application derivation restores identical live-ISO and
installed-channel packages, allowing offline installation to reuse the bundled
binary. The check also asserts source-path independence for desktop and core
packages. Local evaluation with the actual bundled channel and copied Peasy
source confirmed identical derivations; embedding the test source path reproduced
different derivations before the fix.

The 109 native tests passed, followed by the final search regression and workspace
Clippy with warnings denied. Rust/Nix formatting and patch whitespace checks
passed. The separate Nix export check passed all four tests, including real NixOS
restore evaluation. A full pinned-Nixpkgs search returned `hello`; a separate
instance of the fixed daemon then returned a real host overlay's overridden
`hello` version through IPC in 30 seconds, using only temporary configuration,
state and socket paths and disabled activation tools.

The focused release daemon passed all 34 tests inside Nix's build sandbox. The
local security VM passed the initial configuration and filesystem restrictions,
but its full search did not complete: a bounded diagnostic run showed Nix
waiting in `p9_virtio_zc_request`/`p9_client_rpc` for most of the request. Larger
9p transfers did not resolve the delay; a local store image also stalled on host
disk reads. Those experimental VM storage changes were removed. The VM search
now has an explicit 15-minute deadline and asserts the parsed response type and
exact `hello` attribute. A complete security VM and full offline ISO installation
still need a fresh CI run; neither is claimed to have passed locally.

### Reconnecting after activation (2026-09-10)

CI run `34509218250` passed the security job and offline installation, then
successfully activated Patchelf. Its immediate package-list read raced the
daemon's controlled exit and received a connection reset. The cache's HTTP 418
warning was separate: the build and installation continued successfully.

The installer acceptance helper now gives package-list reads a 15-second
reconnection deadline. The app client also retries observational requests during
that window on a missing/refused socket, a dropped connection or the existing
"Peasy is updating" response. Application errors and malformed complete responses
still fail immediately. Proposals, applies, recovery proposals and cancellation
requests are never replayed automatically. Client cancellation remains responsive,
and the reconnect window does not impose a new timeout on healthy package searches.

Regression coverage exercises install and remove verification across connection
loss, interrupted response frames, restart replies, unavailable sockets, retry
expiry, cancellation and mutation responses that must not be replayed. The release
check explicitly includes the production installer helper in its Nix sandbox.
All 98 native client/core/daemon tests passed (two existing integration tests remain
ignored), along with all 57 Python tests, seven website tests and workflow lint in
the pinned Nix release check. Clippy for the changed Rust crates passed with
warnings denied; Rust/Nix formatting and patch whitespace checks also passed.
The full ISO VM test still needs a fresh CI run.
