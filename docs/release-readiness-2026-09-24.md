# Release readiness review — 24 September 2026

The nine audit findings have implementation fixes and regression coverage.
Release approval still requires the final commit's KVM and installed-ISO gates
to pass. This workspace has no `/dev/kvm`; the new VM cases have not been booted
here. No live host activation, reboot, tag, upload or publication was performed.

## Fixes

| Finding | Change | Regression coverage |
| --- | --- | --- |
| Old pea schemas prevent requests after an upgrade | Host API 2 and manifest 1.1.0 advertise the new schema; exact API 1 manifests and pins remain accepted, including removal | Legacy schema acceptance, permission expansion rejection, managed-state round trips |
| PostgreSQL data directories become unreadable to the daemon | A fixed read-only oneshot inventories version names with `CAP_DAC_READ_SEARCH`; its explicit writable path is a private report directory; the daemon retains zero capabilities | Directory-version tests; production-daemon VM cases for an installed server, conflicting retained data and reinstallation |
| Restored private exports retain user ownership | Restore instructions assign root ownership to the four copied trees without widening their permissions or following directory symlinks | Export recipe assertion; capability-free root read checks in the sandbox VM |
| A successful stale build can replace administrator changes | Source and generation guards run after the build, before handoff, in the helper and under NixOS's switch lock; conflicts preserve state and a recovery record | Successful-build source conflict and generation/source guard tests |
| Vulnerable TLS dependency | `rustls` upgraded from 0.23.43 to 0.23.45 | Dependency audit of the updated lockfile |
| Headless recovery requires AI setup | `peasy --status` and `peasy --recover` run before provider or engine initialization; recovery retains review and authorization | Executable CLI tests with no provider files or engine, covering status, approval, decline and EOF |
| Empty or missing generation themes leave overrides behind | Per-user ownership records preserve original native values; removed fields restore them; unset GNOME keys reset; Plasma palettes are explicitly refreshed | Native GNOME and offscreen Plasma smoke tests with disposable homes and private D-Bus sessions; unit and desktop VM cases |
| Cancellation leaves proposals or continuations allocated | CLI review guards release tokens on all exit paths; GUI cancellation clears client continuations; failed applies discard continuations | Repeated cancellation with an unavailable daemon; CLI decline/EOF tests |
| Wi-Fi selection and password prompts are misleading | Exact case-sensitive SSIDs take priority; ambiguous names/security fail explicitly; open/saved profiles do not require a new password; explicitly unsaved secured profiles do | SSID ambiguity/security tests and open, saved, agent-owned and unsaved credential cases |

The appearance rollback record starts when this version first manages a setting.
Original preferences overwritten by an older version without such a record
cannot be reconstructed automatically. Users with those historical overrides
may need to reset them once through their desktop's settings.

## Verification

- Full Rust/Wasm workspace checks: **155 tests passed**, none ignored; formatting,
  Clippy with warnings denied, and generated catalogue contracts passed.
- The subsequent Plasma default-scheme correction passed all six appearance
  tests and the native smoke test, including the palette's background colour.
- Native GNOME tests verified custom and unset original values, empty themes,
  and missing theme files. Native Plasma tests verified accent, scheme and palette
  restoration for empty and missing files. These used isolated user directories
  and private buses; no workstation preferences were changed.
- Full dependency audit: **336 dependencies, zero vulnerabilities or warnings**,
  including yanked-release checks. The 1,267-advisory database was at revision
  `1931a145168d457fd79b321277b25b0e51777157` (23 September 2026).
- Release safeguard tests: **58 Python tests and seven browser-script tests**
  passed, along with workflow linting.
- Portable export evaluation: **four tests passed**, including real NixOS module
  assertions. The headless package also built and passed **142 sandboxed tests** and
  graphical-dependency exclusion check.
- Final NixOS module evaluation passed all assertions and confirmed the separate
  daemon and database-helper capability and writable-path restrictions.
- Final aggregate **x86_64-linux flake evaluation passed** after realizing the
  generated networking and PostgreSQL fixtures. The evaluated GNOME, Plasma,
  PostgreSQL and sandbox VM scripts, plus the PostgreSQL IPC fixture, passed
  Python syntax checks. AArch64 evaluation and VM boot execution were not run.

## Release gates

`scripts/check-release.sh` now uses the full Rust/Wasm check script before the
dependency audit and Nix checks. Both pull-request CI and the tag workflow audit
dependencies. Publication additionally depends on the full Rust checks,
PostgreSQL, networking, pea-fetch and daemon sandbox VMs. Desktop release checks
cover GNOME, Plasma and XFCE, including the new theme rollback assertions.
Generated VM fixtures are realized before aggregate read-only flake evaluation,
so a cold runner can reach those checks without depending on an earlier build.

Before release, run those gates on a Linux runner with KVM, and complete the
existing fresh offline BIOS/UEFI installation tests against the actual release
ISO. The ISO workflow's manual dispatch runs these checks without publishing;
use it before pushing a release tag. Verify authentication, install/removal,
cancellation, recovery and rollback
on a representative machine; exercise real Wi-Fi and Bluetooth hardware. NixOS
generation rollback does not reverse PostgreSQL transactions, role changes or
other external service effects.
