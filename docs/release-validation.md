# Release hardening

This change addresses the identified release blockers. It is not a claim of
independent security certification or a guarantee against every regression.

## Changes

- Nix string interpolation is escaped in generated metadata, embedded state,
  and trusted path literals. Model or release text cannot become Nix code.
- System Apply requires daemon-side Polkit administrator authorization bound to
  the IPC peer's UID, PID, and start time. Tokens remain single-use, short-lived,
  and tied to the reviewed state; a wrong user cannot consume another token.
- External AppImage installation shows the source for review and requires
  administrator authentication. At the user's request, prior repository/hash
  approval is now optional rather than mandatory. Discovery and a self-calculated
  hash are not publisher authentication. Strict deployments can still enforce
  the exact repository/hash allowlist. Nixpkgs and removal remain available.
- Key-file reads reject symlinks, wrong ownership, unsafe permissions and
  excessive size. Debug output and provider errors redact the stored key before
  truncation. The active provider key buffer is cleared on drop.
- Credential-looking inline requests are rejected before model calls. Wi-Fi
  passwords are entered locally and sent to NetworkManager through stdin.
- Provider redirects are disabled, and local Ollama traffic ignores proxies.
- The main service drops capabilities and has additional syscall, memory, task,
  swap, executable-memory and core-dump restrictions. Nix operations, IPC
  connections, request rates, pending proposals, command time and output are
  bounded. Activation requests are size-bounded and consumed once.
- Package verification imports the pinned store source directly, avoiding
  repeated path-flake setup. Nix stderr is drained with a bounded tail buffer,
  so verbose evaluation does not fail a valid build and errors show their cause.
- The existing managed-Nix source and generation reconciliation are retained;
  no second persistent state database or replacement rollback system was added.

## Reproducible checks

Run `bash scripts/check-release.sh` from the repository root on Linux with KVM.
It uses the locked Nixpkgs input and fetches current RustSec advisories. No live
host switch or model-provider request is performed by that script.

The Rust tests cover denied direct-IPC apply, token replay, wrong-user tokens,
expiry, resource bounds, secret handling, hostile Wasm imports, failed builds,
stale proposals, generation-state restoration and existing desktop behavior.
The sandbox VM uses the production service settings and real Nix/Polkit calls;
its activation target is a deliberately inert test generation. The GNOME VM
checks desktop startup, the launcher, UI process registration and live appearance
sync; it does not exercise a live provider or complete interactive workflow.

### Results recorded during hardening

The results below precede the subsequent AppImage review-policy adjustment;
they are not a full release validation of that follow-up change.

- Workspace tests: 53 passed, including the pinned-import cache regression.
- Rust formatting and strict workspace/all-target Clippy: passed.
- RustSec audit: no advisories reported for 336 locked dependencies against
  the fetched advisory database (1,239 advisories).
- Isolated Polkit/activation preflight: malformed requests, denied Apply,
  single-use tokens, and authorized activation of the inert generation passed.
- Desktop and headless release packages: compilation and packaged Rust tests
  passed on x86_64-linux. The minimal dependency-closure and zero-Wasm-import
  checks passed as well.
- GNOME VM: active/enabled extension state, live theme synchronization, launcher
  integration and UI process/D-Bus registration passed.
- Full sandbox VM: incomplete. The machine was powered off during the cold
  package-search step. Before interruption, the legacy, generated-theme and
  generated-AppImage Nix builds passed, as did the configured source-directory
  access and private-home isolation checks. This is neither a full pass nor a
  confirmed test failure; the remaining end-to-end sandbox checks still need
  to finish before release sign-off.

VM-only fixture corrections were made after the release binaries were built:
registering the inert generation in the guest store, waiting for extension
readiness, and querying GNOME's extension state directly over D-Bus. The corrected
fixtures reuse those tested binaries; production Rust and service-module code
were unchanged. Other CPU architectures have not been built or boot-tested here.

These results do not replace running the complete release script for the exact
revision being shipped or completing the host acceptance checks below.
Completed checks are retained as evidence; they were not repeated solely because
the machine was powered off. The interrupted long-running VM check was deferred
at the user's request to avoid another lengthy local run.

## Deployment acceptance

### AppImage review-policy follow-up

After making prior hash approval optional, all 49 tests in `peasy-core`,
`peasy-client` and `peasy-system` passed, including valid reviewed installs,
invalid records, strict allowlists, and a policy tightened between preview and
Apply. Strict Clippy for those crates, Rust/Nix formatting and whitespace checks
passed. NixOS module evaluation confirmed the default `null` policy, explicit
empty-map denial, and configured repository/hash restrictions. Release packages
and VM suites were not rebuilt for this follow-up; no live AppImage installation
or host switch was performed.

### Host checks

Rebuild the host and explicitly restart `peasy-system`; these source changes do
not harden an already-running older daemon. See [installation](install.md).

Before broad rollout, exercise administrator authentication, install/remove,
boot and rollback on a representative host. Test Wi-Fi/Bluetooth on real
hardware, and Hyprland in the intended session setup. Do not supply production
API keys to CI. Hardware, interactive authentication and provider-service
availability cannot be established by unit tests alone.

The AI still has no terminal, file tools, arbitrary Nix expressions or general
system-setting capability. Administrators, trusted host configuration, Nixpkgs,
Polkit, systemd and the Nix daemon remain in the trusted computing base. A
compromised same-user process or root can access that user's secrets; natural
language cannot reliably identify arbitrary pasted secrets. NixOS activation
may partially apply before failing, just like a normal rebuild. See the full
[security model](security.md) for these boundaries.
