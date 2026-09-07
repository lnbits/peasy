# ISO installation measurements

Local validation on 2026-09-06. These are observations from one machine, not
minimum hardware requirements or a guarantee for every PC.

Host: Intel Core Ultra 7 255H, Linux/KVM, QEMU 11.1.0. Unless stated otherwise,
VMs use four virtual CPUs, 8 GiB RAM, a new 32 GiB virtual disk, no network
adapter, and no host filesystem or Nix-store share. Installation uses the
image's shipped Calamares NixOS job. See [the test procedure](iso.md#verification)
for its scope and commands.

## Released baseline

The downloaded v0.1.4 GNOME ISO was reconstructed and SHA-256 checked:

```text
61f3ce9eb21e29b84c30eb80c81c1bd2ae36e4b09e3a806a4f3e1d785ff17716
```

Size: 3,064,233,984 bytes (3.06 GB / 2.85 GiB).

| Firmware | Live console ready | Installation attempt | Result |
| --- | ---: | ---: | --- |
| BIOS | 30.94 s | 45.74 s | Failed: flakes disabled |
| UEFI / Q35 | 55.98 s | 28.93 s | Failed: flakes disabled |

The installer resolved `<nixpkgs>` through `flake:nixpkgs`, while its traditional
`nixos-install` invocation did not enable flakes. The ISO now keeps the bundled
traditional channel search path. These attempts ran alongside image-building
work, so the timing differences are not a firmware performance comparison.

There is **no successful baseline installation time**, so a percentage speedup
against this release would be misleading.

## Offline-cache validation

The first revised GNOME ISO passed that evaluation failure, but its offline
installation exposed a documentation-cache mismatch. The seed's generic
`pre-git` library version did not match the installed channel's release metadata.
That changed the NixOS HTML manual derivation and caused attempts to build its
missing dependencies. The seed now uses the matching release library metadata;
the manual remains enabled and is prebuilt rather than removed.

The next actual-image runs exposed a second mismatch in both desktops: reusing
the live installer's package set also reused its reduced speech-voice override.
The seed now starts with the normal installed package set and applies only the
release-library metadata override. Configuration checks cover both the full
speech package and matching manual, in addition to the actual offline gate.

For the corrected images measured below, Peasy resolves to the same derivation
from the build snapshot and from the bundled source with the installed channel:

```text
/nix/store/phgza7r7zvbgdhr7ynsawm9w1knimxnp-peasy-0.1.0.drv
```

Both installation logs confirm that the prebuilt Peasy package was copied into
the target, not recompiled. NixOS still built machine-specific configuration
outputs. The cache was not supplemented by the test host.

## Corrected images: fresh offline acceptance

Both complete runs passed with `online: false` and `reused_base: false`:

| Measurement | GNOME / BIOS | Plasma / UEFI |
| --- | ---: | ---: |
| ISO size (bytes) | 3,763,814,400 | 4,391,202,816 |
| Live console ready | 68.65 s | 55.96 s |
| Installation job | 233.75 s (3 min 54 s) | 320.44 s (5 min 20 s) |
| Installed boot and startup checks | 28.91 s | 33.84 s |
| Disposable-overlay runtime checks | 224.41 s | 308.03 s |
| Complete test, including hashing and shutdowns | 678.93 s | 893.12 s |

The fresh runs overlapped to reduce validation time; shared host disk activity
and warm caches affect these measurements. These are not controlled desktop
performance comparisons or guaranteed installation times.

Exact image SHA-256 values:

```text
GNOME  71e6a99161474763709ebee6ee1366815c6b45a861bd2f59ecb44a791d7e159e
Plasma 5ed7eb85d97a6a206dcfe00436a36fbaae2c019dc7916e6182fb5c617a513d42
```

The tests booted each installed disk without its ISO, checked Peasy daemon/tray
startup and administrator authorization policy, installed `patchelf` through
the real daemon, ran an ordinary `nixos-rebuild build --no-flake` retaining that
package, and removed it through Peasy. The command was absent from the global
system environment before installation and after removal. No AI account was
needed; the test drives the trusted proposal/approval path directly.

`patchelf` is already cached by normal configuration builders. An earlier `jq`
test failed because its optional manual was not retained in the installed
system. This does not promise arbitrary application installation offline, and
the test does not disable documentation or inject extra package outputs.

The driver also needed to consume serial output during shutdown and allow the
desktop's shutdown grace period. It refuses to cache a VM that it had to kill.

## Snapshot reuse

Both repeat runs passed with `reused_base: true` and `online: false`. They used
fresh disposable overlays and, for UEFI, a separate firmware-variable copy.

| Complete test duration | GNOME / BIOS | Plasma / UEFI |
| --- | ---: | ---: |
| Fresh installation plus runtime checks | 11 min 19 s | 14 min 53 s |
| Reused base plus runtime checks | 3 min 23 s | 5 min 53 s |
| Reused run, exact seconds | 202.58 s | 352.85 s |

The reused runtime phases themselves took 152.21 s and 320.37 s; the remaining
time includes checking ISO/base hashes and cleanup. Reuse skips installation,
not Peasy's build/activation checks. The repeat runs also overlapped partially,
so these observations should not be treated as universal speedup ratios.

[Raw results](iso-benchmark-results.json) preserve each run's timings, complete
ISO and driver hashes, resources, networking mode, and pass/reuse flags. The
measured images came from the build snapshot used during this work; release CI
must rebuild and test the exact committed release artifact.

Supporting checks passed: 64 Rust tests during the native package build, 28
Python release/harness tests, workflow linting, Nix formatting and flake
evaluation, plus the formatting, ISO-configuration, installer-target and
desktop-configuration derivation checks. These do not replace physical-hardware
testing or exercising every graphical-installer choice.

## Interpreting subsequent results

`result.json` identifies the exact ISO and harness by hash and separates live
boot, installation, installed boot checks, overlay runtime checks, and total
test duration. A result with `reused_base: true` is a developer runtime test,
**not** proof of a fresh installation. Release CI always requires a fresh run.

Prebuilt desktop packages do not eliminate NixOS evaluation or the assembly of
machine-specific accounts, services, boot files, and system generations. The
goal is to avoid native package compilation and downloads for the tested
installation, not to replace the normal NixOS configuration workflow.
