# GNOME and Plasma installers

The installers retain Peasy after installation. Installed-disk boot is verified
in GNOME/BIOS and Plasma/UEFI VMs; physical-hardware testing remains important.
Pushing a `v*` tag automatically publishes a release only after both desktops'
CI checks and asset verification succeed. Complete ISOs are hosted on Cloudflare
R2; GitHub Releases carries the download links and whole-image checksums.

Installing Peasy on an existing NixOS system is separately supported: follow
[manual installation](install.md). It does not apply the distro wallpaper or accent.

## Build and install

Choose GNOME or Plasma from the [latest release](https://github.com/lnbits/peasy/releases/latest)
or the website's download section. Download the complete `.iso` using its release
link, and the matching `.iso.sha256` GitHub asset into the same directory. Verify
before writing it to USB or booting it in a VM (replace `v1.2.3` with the tag):

```console
sha256sum --check peasy-nixos-v1.2.3-gnome-x86_64.iso.sha256
```

Use `plasma` instead of `gnome` for Plasma. No joining or extraction is needed.
Checksums detect corruption, not publisher impersonation; get them from the
trusted repository's release page. Only the latest release's ISOs are retained.
Older GitHub checksums and source archives remain, but old ISO links expire.

Historical releases that shipped `.partNNN` files still use the included
`join_iso.py`: download every part and its `.iso.parts.json` manifest, then run
`python3 join_iso.py peasy-nixos-v1.2.3-gnome-x86_64.iso.parts.json` (Python 3.11+).
The helper checks every part and the whole image. Parts alone are not bootable.

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

Both ISOs retain the NixOS UEFI/GRUB menu design and logo, with a pale green
background, green highlights and **Includes Peasy** below the menu. Boot entries
also mention Peasy, including in the legacy BIOS menu. The graphical installer
retains its NixOS logos, wording and layout, with a green sidebar and
**Includes Peasy** beneath the main logo. The trusted artwork and theme live in
[`nix/iso-branding`](../nix/iso-branding/README.md).
This does not change the installed system's bootloader, partitioning, account
creation or installation steps; manual Peasy installations are unaffected.

The image also carries a representative prebuilt installed desktop and the
tools needed to assemble machine-specific NixOS configuration. The seed's
configuration and locked placeholder account are never used as the installed
system: Calamares still generates the user's own configuration. A shared locale
archive avoids compiling a separate archive for each language selection.

NixOS still evaluates configuration and builds small system-specific outputs
(accounts, service definitions, boot files). "Building the system" is not the
same as compiling the desktop or Peasy. Offline coverage is limited to the
desktop shipped on that image and tested hardware/installer selections; choosing
another desktop, proprietary drivers, or extra software may require downloads.

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

See [local ISO measurements](iso-benchmarks.md) for measured results and their
limitations.

### Actual ISO: fresh installation and timing

The release gate boots the **actual ISO**, with no virtual network adapter and
no access to the host's Nix store. It runs the ISO's shipped Calamares NixOS job,
boots the installed disk without the ISO, and tests Peasy's tray, authorization,
`patchelf` installation, ordinary rebuild, and removal. Its complete package is
already cached by the normal configuration builders, but it is not initially a
global command. This exercises package management without injecting test
packages or promising that arbitrary new applications are available offline.
Wizard selections are supplied
by the harness; disk formatting and all installer operations execute in the VM.
BIOS uses the image's existing serial-console boot entry; UEFI boots normally
and enables a live serial getty through a text console. Both then boot the
installed graphical desktop. This does not test clicking every installer screen.

On an x86-64 Linux host with KVM access and Python 3.11 or newer, from the repository:

```console
nix build .#iso-test-tools --out-link result-iso-test-tools
python3 -B scripts/iso_vm.py --iso /path/to/peasy-gnome.iso --desktop gnome \
  --qemu "$PWD/result-iso-test-tools/bin/qemu-system-x86_64" \
  --qemu-img "$PWD/result-iso-test-tools/bin/qemu-img" \
  --output /tmp/peasy-gnome-fresh
```

For Plasma/UEFI, use `--desktop plasma --firmware uefi` and add:

```console
--ovmf-code "$PWD/result-iso-test-tools/FV/OVMF_CODE.fd" \
--ovmf-vars "$PWD/result-iso-test-tools/FV/OVMF_VARS.fd"
```

Choose a new output directory for every run. `result.json` records ISO size/hash,
VM resources, firmware, networking mode, separate installation/runtime timings,
and pass/fail status. Full logs and an installed-desktop screenshot are retained.
Default resources are 8 GiB RAM and four virtual CPUs; CI uses 6 GiB and two CPUs.
`--online` permits downloads for diagnosis and is **not** an offline acceptance test.

### Faster developer tests with disposable overlays

Repeat the same command with `--reuse-base` and a new output directory to skip
installation when a matching base exists. Each test boots a new qcow2 overlay
and a separate copy of the UEFI variables; runtime changes never enter the base.
The base is saved only after a clean installed-system boot and shutdown.

Cache identity includes the ISO's full SHA-256, desktop, firmware, VM resources,
networking mode, QEMU version, firmware images, and test-driver contents. Cached
disk and firmware contents are also hash-checked. A changed ISO cannot silently
test an older Peasy binary. This tests the supplied ISO, not unbuilt source edits.

Bases live in `~/.cache/peasy/iso-vm` by default (`--cache-dir` overrides it), in
private user-owned directories. They contain **test-only passwords** and can use
several GiB per identity; do not distribute them as production VM images. Nix
garbage collection does not remove this cache. Remove unused cache entries when
finished with them. Temporary overlays are removed even when a test fails.

CI refuses `--reuse-base`; every release must complete a fresh installation.

### Configuration and enriched-fixture regressions

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

These older fixtures remain useful for focused regressions, but supply additional
test instrumentation. They are not proof that a released ISO can install offline.
Release acceptance instead uses the actual-ISO test above.

`iso-config` checks live desktop selection, wallpaper, green defaults and consistent
release flags. The desktop runtime tests check the UI/tray and appearance adapters.

## CI and publication

The single `iso.yml` workflow uses a GNOME/Plasma matrix. Each build requires its
desktop runtime test, a fresh offline installation of the actual ISO, and the
release/test-harness unit tests. Timings and logs are uploaded separately from
the release payload. Manual dispatch produces complete ISO Actions artifacts and SHA-256
files only; it does not publish. Tags `v*` are checked out at their exact event
commit. `lib.isoReleaseStatus` in `flake.nix` enables release eligibility; it does
not bypass CI checks or guarantee compatibility with every physical machine.

Desktop images can exceed GitHub's
[2 GiB limit per release asset](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases).
The publish job therefore uploads complete images to R2 using multipart transfers
(a transport detail: users download a single `.iso`, not parts). It validates both
local images before any release writes, then checks stored size/metadata and
streams each image back from R2 to verify its full SHA-256. A public HEAD request
also checks that the download domain serves each image at its expected size.
Versioned paths include the tag and exact commit; conflicting objects are never
overwritten. No second local ISO copy is needed for publication.

GitHub receives each `.iso.sha256`, `SHA256SUMS` covering both **whole images**,
and `iso-downloads.json`. Release notes contain direct download links and the same
machine-readable metadata. The website reads GitHub's latest published release,
validates its metadata, and displays both download cards. No R2 CORS or Worker is
needed. With no JavaScript, unavailable API, or a legacy release, buttons fall back
to GitHub Releases rather than guessing nonexistent ISO URLs.

The job creates a temporary private draft, uploads assets, checks GitHub's stored
sizes and SHA-256 digests, and verifies the tag still identifies the tested commit.
Only then does it **automatically publish** the completed release. No manual
Publish click is required. A newly published R2-backed release is explicitly
marked Latest; rerunning an already published release does not change Latest.
Failed uploads leave the draft private; rerunning the
job resumes matching uploads. Conflicting assets, unrelated releases and changed
tags cause failure without overwriting anything. An incomplete asset left by
GitHub may need manual removal before retrying. Do not publish incomplete drafts.

Actions and Nix are pinned. Only the publication job receives a repository-write
token; no AI provider credentials or signing keys are required. Commit the
workflow, scripts and configuration together before pushing your release tag.

### R2 configuration

Create a Standard R2 bucket and connect `downloads.askpeasy.com` under its custom
domains. Leave the development `r2.dev` endpoint disabled. In repository Settings
→ Secrets and variables → Actions, configure:

| Kind | Name | Value |
| --- | --- | --- |
| Secret | `R2_ACCESS_KEY_ID` | R2 S3 access key ID |
| Secret | `R2_SECRET_ACCESS_KEY` | R2 S3 secret access key |
| Variable | `R2_BUCKET` | `peasy-releases` |
| Variable | `R2_ENDPOINT_URL` | `https://<account-id>.r2.cloudflarestorage.com` (no bucket suffix) |
| Variable | `R2_PUBLIC_URL` | `https://downloads.askpeasy.com` |

Use an Object Read & Write token restricted to this bucket. Do not commit keys or
put them in variables. The publisher uses `iso-release-tools`, a Python/Boto3
environment pinned by `flake.lock`. Credentials enter only the upload step, after
tool installation, and are never put in release metadata or download requests.
If you change the public hostname, update the website's allowlisted origin in
`assets/downloads.js` too.

Publication jobs are serialized across tags. Only after publication succeeds and
GitHub confirms it as the latest stable release does cleanup delete superseded
Peasy ISO objects under `peasy/releases/`. Exact paths and ownership metadata are
checked; unrelated bucket objects and the latest images are left alone. Older
GitHub releases keep their checksums and sources, but their ISO links expire.
Rerunning an older published release cannot restore expired ISOs or prune the
latest. A failed upload leaves previous downloads available; a failed cleanup can
be retried by rerunning the publish job. There is no age-based expiration rule that
could remove the latest images during a quiet period.

Allow temporary storage for **both** releases during publication. Failed drafts
may also leave objects until a later successful cleanup. R2's storage/request
allowances still apply even with latest-only retention. Configure an R2 lifecycle
rule to abort incomplete multipart uploads if you change the bucket's default
rule; do not configure an age-based rule to expire completed release objects.

Local regression checks (no cloud credentials or ISO builds required):

```console
python3 -B -m unittest discover -s scripts/tests -v
node --test scripts/tests/downloads.test.cjs
```
