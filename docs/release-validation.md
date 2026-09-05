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

## Completed automated validation — 2026-09-05

Tested revision: `2fd8a71767a78e3df34e5102441c3478e50c722a` on x86_64-linux,
including the optional AppImage allowlist and all committed VM fixture fixes.
The worktree was clean during testing. No production code or test-fixture changes
were needed to complete this run; only this report was updated afterward.

All checks from the release script completed successfully. Formatting, lint and
workspace tests ran through `nix develop`; the dependency audit ran against the
current RustSec database; `nix flake check -L --keep-going` exited successfully
with **all checks passed**.

| Check | Result |
|---|---|
| Workspace regression tests | 54 passed |
| Rust formatting and strict workspace/all-target Clippy | Passed |
| RustSec audit | No advisories reported for 336 locked dependencies; 1,239 advisories loaded |
| Desktop release package | Build and all 54 packaged tests passed |
| Headless release package | Build and all 51 packaged tests passed |
| Headless dependency closure | Passed; no prohibited graphical dependencies |
| Built Wasm imports | Passed; zero host imports |
| Nix formatting | Passed |
| Full sandbox VM | Passed in approximately 423 seconds |
| GNOME integration VM | Passed in approximately 57 seconds |

The sandbox exercised real Nix evaluation/build calls for legacy, generated-theme
and generated-AppImage modules; cold package search; allowed configuration reads;
denied private-home reads and arbitrary `/etc` writes; empty daemon capabilities;
Polkit policy registration; hostile IPC rejection; denied Apply without
authorization; single-use tokens; and authorized build/helper activation of the
inert test generation. The test-only authorization rule exists only inside the
VM. This is not a real-host boot/rollback or interactive password-dialog test.

GNOME checks covered the active/enabled extension, launcher integration, live
theme synchronization, provider-not-configured handling and UI process/D-Bus
registration. No production API key or live model-provider request was used.

### Build artifacts and logs

The tested outputs are retained in the local Nix store, subject to normal garbage
collection:

```text
Desktop: /nix/store/hwlxz80qsznac06szwfm9pqsrz38mvfj-peasy-0.1.0
Headless: /nix/store/wi0ygxmqnqslgwi3gmjh05wisga4wyq1-peasy-core-0.1.0
Sandbox: /nix/store/54zlqxm91mcdl1xxy29109669pl912q9-vm-test-run-peasy-sandbox
GNOME: /nix/store/z77fiany7z9cvyab21librj8bp20gp1c-vm-test-run-peasy-gnome-tray
```

The GNOME output includes `peasy-mint-launcher-and-provider-setup.png`.
Read the VM build logs with:

```console
nix log /nix/store/kpx127q3yhkq3r5y2z4g2ra9jch4k9z2-vm-test-run-peasy-sandbox.drv
nix log /nix/store/lhpaq10dfczdlpq4fl0l5rpnvbxagd6j-vm-test-run-peasy-gnome-tray.drv
```

### Earlier interrupted runs and search timing

Earlier release/VM runs were interrupted by shutdown or conversation cancellation.
The completed run above supersedes their incomplete status, including the cold
search that previously had no result. It used the committed fixtures and current
release packages directly through the flake, not a reduced substitute test.

Cold search took about 304 seconds in the single-vCPU VM with a shared 9p store.
A separate read-only host search against the same pinned Nixpkgs source succeeded
in about 77 seconds. These are diagnostic observations, not a controlled benchmark;
they show that the VM search finished but leave cold-search performance as an
improvement opportunity. No security limits or test assertions were weakened.

Prior focused checks also verified the default `null` AppImage policy, explicit
empty-map denial and configured repository/hash restrictions. Their Rust
regressions are included in the full suite above.

## Deployment acceptance and remaining limits

No live host switch, service restart, reboot, rollback or AppImage installation
was performed during this validation. The user reported testing install/remove
with `hello`; that is user-reported acceptance evidence, not an independently
observed password-dialog or rollback result. AArch64 was explicitly omitted by
the host's flake check and has not been built or boot-tested here.

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
