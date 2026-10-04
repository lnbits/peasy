# Installer ISO

Download the GNOME ISO and matching checksum from the
[latest release](https://github.com/lnbits/peasy/releases/latest). Verify before
writing it to USB, replacing the example version:

```sh
sha256sum --check peasy-nixos-v1.2.3-gnome-x86_64.iso.sha256
```

Boot it, run the graphical installer, then remove the USB and boot the installed
disk. Peasy, local Ollama and Qwen3 0.6B are included. GNOME installs offline for
tested configurations; optional XFCE, extra software or drivers may need Internet.
The live desktop always runs GNOME. Only the latest release's ISO links are retained.

The installer adds `/etc/nixos/peasy.nix`, bundled source under `/etc/nixos/peasy/`,
and an optional managed-state import. Installed systems require normal administrator
authentication. Live-session credentials and settings are not transferred.

## Build

```sh
nix build .#iso-gnome --out-link result-iso-gnome
```

The image is in `result-iso-gnome/iso/`. Build on x86_64 Linux with substantial
free disk space. `iso-plasma` is available for local development but is not published.
Existing images do not change when source files are edited.

## Verification

On Linux with KVM and Python 3.11+, test the actual image in disposable VMs:

```sh
nix build .#iso-test-tools --out-link result-iso-test-tools
python3 -B scripts/iso_vm.py --iso /path/to/peasy-gnome.iso --desktop gnome \
  --qemu "$PWD/result-iso-test-tools/bin/qemu-system-x86_64" \
  --qemu-img "$PWD/result-iso-test-tools/bin/qemu-img" \
  --output /tmp/peasy-gnome-fresh
```

For UEFI add `--firmware uefi` and:

```sh
--ovmf-code "$PWD/result-iso-test-tools/FV/OVMF_CODE.fd" \
--ovmf-vars "$PWD/result-iso-test-tools/FV/OVMF_VARS.fd"
```

Use a new output directory each time. The default VM uses 8 GiB and four CPUs.
The offline test runs Calamares' installation job, boots the installed disk and
checks Peasy startup, authorization, bundled model availability and package
install/remove. Results and logs identify the exact ISO. It does not click every
wizard page or replace hardware testing.

`--reuse-base` speeds developer checks; it is not fresh-install evidence and CI
rejects it. Private bases live in `~/.cache/peasy/iso-vm`, contain test passwords
and need manual cleanup. `--online` is diagnostic, not offline acceptance.

[Historical measurements](iso-benchmarks.md) · [All release checks](release-validation.md)

## CI and publication

`.github/workflows/iso.yml` requires security/resource checks, desktop regressions
and fresh offline BIOS/UEFI installs of the actual GNOME ISO before publishing.
Manual dispatch validates without publishing. Visual checks and physical hardware
acceptance remain separate.

For a release:

1. Update the workspace version in `Cargo.toml` and run `cargo update --workspace`.
2. Validate the intended tag: `python3 scripts/update_metadata.py --tag vX.Y.Z --check`.
3. Commit and push all intended changes, then push a new matching `vX.Y.Z` tag.
   Tags contain committed files only; do not move an existing release tag.

CI verifies and publishes the ISO to R2, with checksums, download metadata and
`peasy-update.json` on GitHub. Failed drafts stay private; rerun the job to resume.
Do not manually publish incomplete drafts.

### R2 configuration

Use a Standard bucket with the `downloads.askpeasy.com` custom domain:

| Actions setting | Name |
| --- | --- |
| Secret | `R2_ACCESS_KEY_ID` |
| Secret | `R2_SECRET_ACCESS_KEY` |
| Variable | `R2_BUCKET` (for example `peasy-releases`) |
| Variable | `R2_ENDPOINT_URL` (`https://<account-id>.r2.cloudflarestorage.com`) |
| Variable | `R2_PUBLIC_URL` (`https://downloads.askpeasy.com`) |

Restrict credentials to object read/write on this bucket. Keep `r2.dev` disabled.
A hostname change also requires updating `assets/downloads.js`'s allowed origin.
Allow space for two releases during publication. Cleanup removes superseded ISOs
only after successful publication; do not add age-based expiry for completed objects.

Test publication logic without cloud credentials or ISO builds:

```sh
nix build --no-link -L .#checks.x86_64-linux.release
```
