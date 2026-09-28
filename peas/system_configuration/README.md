# System configuration pea

Generic installation **and uninstallation** of application setup, not a collection
of app-name recipes. The model reasons about the selected application's needs and
combines supporting packages, reviewed NixOS enable options and user-group access
into one proposal. There is no `if request contains virt-manager` routing.

For example, “install Virtual Machine Manager” can select `virt-manager`, enable
`programs.virt-manager.enable` and `virtualisation.libvirtd.enable`, and request
`libvirtd` membership for the caller. “Uninstall Virtual Machine Manager” removes
the Peasy-owned package and setup together. See [example.json](example.json) for a
daemon-bound test record; the AI never supplies its `user` or `uid` fields.

## Contract and boundaries

`install_package` keeps its existing primary `package` and optional `message`.
Its new optional `setup` field contains three required arrays:

| Field | Meaning | Limit |
| --- | --- | --- |
| `packages` | Supporting Nixpkgs attributes, checked by the daemon against pinned Nixpkgs | 8 |
| `enable` | Reviewed Boolean NixOS options; contributes `true`, never `mkForce` | 8 |
| `groups` | Reviewed supplementary groups for the authenticated caller only | 4 |

It also accepts `postgresql`, a nullable typed service configuration containing
`package` (a versioned `postgresql_14` through `postgresql_18` attribute, verified
against the host package set) and `caller_database` (Boolean). Older saved setup
records without this field remain valid. The model schema requires an explicit
null when PostgreSQL is not needed.

A plain install uses `setup: null`. The primary package must still be a returned
candidate. Search results and fallback selections are assessed for integration;
the latter may require one additional model call. Supporting package existence is
verified independently by the daemon, not assumed from model output.

The setting catalogue supports 28 Boolean options and the typed local PostgreSQL
capability. The model selects only the integration needed for the request:

| Use case | Reviewed options |
| --- | --- |
| Virtual machines | `virtualisation.libvirtd.enable`, `programs.virt-manager.enable` |
| Containers | `virtualisation.podman.enable`, `virtualisation.docker.enable` |
| Printing, scanning, Bluetooth | `services.printing.enable`, `hardware.sane.enable`, `hardware.bluetooth.enable` |
| Development environments | `programs.direnv.enable` (includes nix-direnv), `programs.nix-ld.enable` |
| Shell integration | `programs.zsh.enable`, `programs.fish.enable` (does not change login shell) |
| Network diagnostics | `programs.wireshark.enable`, `programs.mtr.enable` |
| Application runtimes | `programs.appimage.enable`, `programs.appimage.binfmt`, `services.flatpak.enable` |
| Desktop integration | `programs.dconf.enable`, `services.gnome.gnome-keyring.enable`, `services.gvfs.enable`, `services.udisks2.enable`, `services.upower.enable` |
| Smart cards and peripherals | `services.pcscd.enable`, `services.ratbagd.enable`, `hardware.i2c.enable`, `hardware.openrazer.enable` |
| Gaming and graphics | `hardware.steam-hardware.enable`, `programs.gamemode.enable`, `hardware.graphics.enable` |

AppImage `binfmt` requires `programs.appimage.enable` in the same setup. Flatpak
also contributes the portal service and a GTK fallback, preserving desktop-specific
portal choices. It does not add remotes, install Flatpaks, grant sandbox overrides
or configure screen sharing. `nix-ld` supplies base libraries, not every binary's
dependencies. Graphics support uses existing host drivers; it does not choose
vendor drivers, enable CUDA or add 32-bit support. Device-specific drivers and
application preferences can still need manual setup.

The group catalogue supports `libvirtd`, `scanner`, `lp`, `docker`, `wireshark`,
`gamemode`, `i2c` and `openrazer`, each requiring a related enable option in the
same plan. Standard NixOS device groups `dialout`, `kvm`, `render` and `video` can
be requested independently for serial devices, direct virtualisation, GPU rendering
and video/camera devices. Request these only when needed; an active desktop seat
often already has device access. Rootful Podman group access is not available;
ordinary Podman use should be rootless.

Review displays each option's effects and additional access. Docker group access
is effectively root access; Wireshark allows capturing private network traffic;
video access includes cameras outside the active session; I2C permits raw hardware
writes. Container runtimes can change networking and published ports can expose
services. Peasy does not configure arbitrary listeners or firewall rules.
Arbitrary sudo/polkit rules, services/scripts, paths, account creation and Nix
expressions remain unavailable as actions.

### Manual steps and installation speed

When a requirement is unsupported, the AI is instructed to return concrete numbered
steps, including where to make changes, known settings or commands, and any local
credential entry, logout or reboot. It uses the host's existing traditional or
flake rebuild workflow and asks for missing details rather than inventing paths or
settings. Users should edit an imported host module, not Peasy's generated state.
Guidance is displayed as text and never executed by Peasy.

A useful partial install can include supported changes and clearly list remaining
manual work. Otherwise Peasy returns an explanation without installing. Explanations
support up to 8,000 characters; oversized responses are rejected rather than
silently truncating commands. CLI and GUI preserve the guidance through package
selection, review and successful completion; the GUI supports scrolling and copying it. The model's
instructions are a behavior contract, not proof of the accuracy of its advice.

Installation uses the normal Nix build and activation checks. Peasy does not launch
applications or run additional service-readiness probes afterward, including for
PostgreSQL. Success means that the reviewed configuration was applied; application
workflows are not automatically tested.

The daemon obtains the caller's account from Unix socket credentials, binds its
name and UID, and rechecks it at apply. Only normal user accounts can receive
groups. Generated Nix asserts that the account is declared as a normal user.
`libvirtd` access is powerful and is explicitly warned about in the review.
Log out completely and back in after adding or removing group access; existing
sessions can retain old groups until they end.

## Service requests

Explicit service requests include supported setup in the same reviewed proposal.
Explicit client/tools-only requests remain plain installations. An ambiguous
request such as "install PostgreSQL" asks whether a local server or tools are
wanted. Package selections retain the original request, and explanations of
unsupported requirements are not replaced by a list of installable executables.

PostgreSQL setup starts the selected server now and at boot, using localhost and
the Unix socket on the standard port, without opening a firewall port. Its NixOS
module supplies the executable and system account. `caller_database: true` also
creates a database and peer-authenticated role named for the authenticated caller,
with ownership of that database. No superuser privilege is granted. Account names
that cannot safely name a database role are rejected before configuration changes.
See [postgresql-example.json](postgresql-example.json) for a daemon-bound record.

The model receives the effective enabled state and server version in the system
profile. The daemon independently inspects current host configuration at preview
and apply, rejects takeover of an administrator-managed server, and blocks major
version changes and retained data directories from another major version. Custom
data directories and conflicting local endpoints fail NixOS validation. The
review explains startup, access, version selection and data retention. The normal
NixOS activation handles service startup; Peasy does not run a separate readiness
probe.

Remote access, passwords, arbitrary database/role names and database migrations
remain unsupported. Users receive manual steps when these are required. Existing roles,
grants and database contents survive removal and configuration rollback: removing
`ensureUsers` does not revoke existing access, and NixOS generations do not undo
SQL writes. Shared service contributions remain until their last setup is removed.

## Ownership, updates and rollback

Each setup is keyed by its primary package in the existing canonical
`.peasy/peasy-managed.nix` state. Updating a setup replaces its whole owned plan;
the AI must retain still-required settings. An existing standalone Peasy install
of the primary package is absorbed into the setup, avoiding an orphan on uninstall.
Supporting packages already installed independently by Peasy remain independent.

Packages, settings and group contributions are combined across setups. Removing
one drops only its contributions: shared dependencies remain, as does anything
declared in administrator configuration. Removal writes no `false` override, and no
VM disks or user data are deleted. Explicit conflicting host settings cause the
ordinary NixOS build to fail and the managed source to be restored.

The existing expiring, user-bound proposal tokens, administrator approval,
transaction lock, state comparison, build/switch and generation reconciliation
apply unchanged. Rolling back to a Peasy generation restores its setup state as
well as packages and themes. Empty setup state retains the old canonical format.

## Extending the catalogue

Add reviewed option names, effects and group prerequisites in `types.rs`. The
model and review use the same descriptions. Test them against pinned Nixpkgs. Do not add keyword-to-package
recipes. A new value type needs a closed schema, validation in both Wasm and the
daemon, a data-only renderer, ownership rules, visible permission warnings and
install/remove/rollback tests. Do not expose arbitrary Nix as an escape hatch.

Example contributor prompt:

> Extend the generic system-configuration pea with the reviewed primitives needed
> for [capability]. Let the model infer when they are appropriate; do not hardcode
> application names or user-prompt matching. Verify NixOS options against our pinned
> Nixpkgs. Preserve existing configuration and all other peas. Keep account binding,
> administrator review, bounded typed input, dependency ownership and generation
> rollback. Add install, uninstall, shared-dependency, malicious-input and failed-build
> tests, and document the exact added permissions and unsupported requirements.

Tests live in `tests.rs`, the shared model/native/Wasm corpus, the daemon tests,
and `nix/tests/system-configuration*.nix`. `nix build
.#checks.x86_64-linux.system-configuration` evaluates the production Rust-rendered
module, every catalogue option, caller groups, coexistence with themes, list merging,
uninstall withdrawal and host-setting conflicts; it does not boot a VM. The sandbox VM also checks setup authorization and hostile IPC.
`nix build .#checks.x86_64-linux.postgresql-vm` checks local database access,
non-superuser ownership, boot startup and data retention across withdrawal and
restoration of the production-rendered PostgreSQL module.
