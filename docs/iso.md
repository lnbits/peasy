# GNOME and Plasma installers

The installers retain Peasy after installation. Installed-disk boot is verified
in GNOME/BIOS and Plasma/UEFI VMs; physical-hardware testing remains important.
Pushing a `v*` tag automatically publishes a release only after both desktops'
CI checks and asset verification succeed. Large ISOs are delivered as lossless
parts to fit GitHub's per-file limit; see the download instructions below.

Installing Peasy on an existing NixOS system is separately supported: follow
[manual installation](install.md). It does not apply the distro wallpaper or accent.

## Build and install

For a published release, choose GNOME or Plasma. If it provides a complete `.iso`,
download that image and verify it against the accompanying `.iso.sha256` file.
Otherwise, download **all parts for your chosen desktop**, its `.iso.parts.json`
manifest, and `join_iso.py` into one directory. With Python 3.11 or later, run
(replace `v1.2.3` with the release tag):

```console
python3 join_iso.py peasy-nixos-v1.2.3-gnome-x86_64.iso.parts.json
```

Use `plasma` instead of `gnome` for Plasma. The helper verifies every part and the
complete image, refuses to overwrite an existing ISO, and removes incomplete
output if verification fails. Allow enough free space for both the parts and
the reconstructed image. Individual parts are **not bootable**; write the
resulting `.iso` to your installation media. Checksums detect corruption, not
publisher impersonation: download the helper and manifests from the trusted
repository's release page.

On an x86_64 Linux Nix builder, from this checkout:

```console
nix build .#iso-gnome --out-link result-iso-gnome
nix build .#iso-plasma --out-link result-iso-plasma
```

Images are below each output's `iso/` directory. Both use `flake.lock`; there are
no additional inputs, generators or mutable wallpaper downloads. Allow substantial
store and temporary disk space for two desktops. Old `experimental-live-only`
images predate the target integration and do **not** retain Peasy after installation.

Boot the image and use the normal graphical installer. Its disk, encryption,
desktop and account choices remain upstream's. After installation, remove the
installation media and boot the installed disk. Peasy is configured through an
ordinary local Nix module; no follow-up Peasy installation command is intended.
Configure your own OpenAI credentials or reachable Ollama provider in Peasy.
No provider account, API key or local model is bundled.

`assets/peasy_bg.png` is the default wallpaper in both the live and installed
systems. GNOME gets light/dark wallpaper defaults and a green accent. Plasma
applies the bundled wallpaper and green accent once after plasmashell is ready.
A per-user marker prevents subsequent logins from overwriting the user's choices.
These are trusted build-time defaults, not AI-controlled scripts or file paths.

## What the installer changes

The lock pins nixpkgs revision `e8be7818e19ada32105a8af937a6a473b38167ca`.
The upstream Calamares NixOS job generates its own `configuration.nix`, so adding
Peasy only to the live image cannot preserve it in the installed system.

`nix/calamares-peasy.patch` makes two narrow additions to that pinned job:

1. Include `./peasy.nix` in the generated target configuration.
2. Run a fixed target-staging helper before the normal configuration write and
   single `nixos-install`. A helper failure stops installation with an error.

The helper copies only the build's immutable Peasy source snapshot and target
entry module. It requires root-owned target directories not writable by others,
refuses symlinks and conflicting existing files, and accepts identical retries.
It cannot copy arbitrary caller-selected source files. The patch must apply
without fuzz; configuration tests detect changes to upstream's generated output.

The resulting source layout is:

```text
/etc/nixos/
├── configuration.nix          # Standard installer output, imports peasy.nix
├── hardware-configuration.nix # Standard hardware detection
├── peasy.nix                  # Enables Peasy and distro appearance defaults
├── peasy/                     # Bundled, versioned Peasy source (not a Git checkout)
└── .peasy/peasy-managed.nix    # Created when Peasy manages system state
```

`peasy.nix` optionally imports the managed file, so ordinary `nixos-rebuild`
includes packages added through Peasy. It does not replace `configuration.nix`
or introduce a separate package manager. Standard NixOS generations remain the
mechanism for system builds and activation.

Only the ISO imports the live-session configuration. The installed target does
not inherit the live account, passwordless installer authorization, Peasy's
live-only Apply denial, or settings/API keys entered during the live session.
Installed system changes retain Peasy's normal administrator authentication.
The helper is an installer component, not an action available to the AI.

The upstream live image grants passwordless wheel Polkit access for installation.
An earlier rule denies **only** Peasy's privileged Apply action in that live image;
search and supported local desktop actions remain available. Avoid entering
production API keys in a shared live session.

## Verification

```console
nix build --no-link .#checks.x86_64-linux.installer-target
nix build --no-link .#checks.x86_64-linux.installed-gnome
nix build --no-link .#checks.x86_64-linux.installed-plasma
```

`installer-target` compares original and patched Calamares configuration generation
for both desktops, mocks external effects, and checks helper failure/retry safety.
The expected generated configuration differs only by the Peasy import.

`installed-gnome` (BIOS) and `installed-plasma` (UEFI) run the patched Calamares
NixOS job against disposable VM disks, including real hardware generation and
`nixos-install`. Only the graphical UI's stored choices are supplied by the test.
They then boot the installed disk, check Peasy's daemon/tray and authorization
policy, install `hello` through the real daemon, verify an ordinary rebuild retains
it, and remove it again. Test-only console/password instrumentation is not part
of the shipped installer. These tests do not automate clicking every installer
screen or cover every physical GPU, storage controller or encryption combination.

Both complete install/boot/package/rebuild/removal flows passed locally while
developing the integration, reusing Peasy's previously tested, unchanged native
binaries. CI runs these checks against each revision's package. Do not mistake a
configuration-only check, or an old live-only image, for an installed-disk test.

`iso-config` checks live desktop selection, wallpaper, green defaults and consistent
release flags. The desktop runtime tests check the UI/tray and appearance adapters.

## CI and publication

The single `iso.yml` workflow uses a GNOME/Plasma matrix. Each build requires its
desktop runtime and installed-system regression tests, plus the release script's
unit tests. Manual dispatch produces complete ISO Actions artifacts and SHA-256
files only; it does not publish. Tags `v*` are checked out at their exact event
commit. `lib.isoReleaseStatus` in `flake.nix` enables release eligibility; it does
not bypass CI checks or guarantee compatibility with every physical machine.

Current standard desktop images are roughly 3 GiB each, exceeding GitHub's
[2 GiB limit per release asset](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases).
The publish job validates both complete images, splits oversized images into
1 GiB parts, and includes reconstruction manifests, `join_iso.py`, whole-image
checksums and `SHA256SUMS` for the uploaded payloads. Smaller ISOs upload directly.

The job creates a temporary private draft, uploads assets, checks GitHub's stored
sizes and SHA-256 digests, and verifies the tag still identifies the tested commit.
Only then does it **automatically publish** the completed release. No manual
Publish click is required. Failed uploads leave the draft private; rerunning the
job resumes matching uploads. Conflicting assets, unrelated releases and changed
tags cause failure without overwriting anything. An incomplete asset left by
GitHub may need manual removal before retrying. Do not publish incomplete drafts.

Actions and Nix are pinned. Only the publication job receives a repository-write
token; no AI provider credentials or signing keys are required. Commit the
workflow, scripts and configuration together before pushing your release tag.
