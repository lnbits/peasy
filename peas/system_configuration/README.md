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

A plain install uses `setup: null`. The primary package must still be a returned
candidate. Search results and fallback selections are assessed for integration;
the latter may require one additional model call. Supporting package existence is
verified independently by the daemon, not assumed from model output.

The initial **setting catalogue**, shared by schema and validation, supports local
libvirt, virt-manager integration, CUPS printing, SANE scanning and Bluetooth.
The group catalogue supports `libvirtd`, `scanner`, and `lp`, each requiring its
related enable option in the same plan. These are generic primitives, not a
promise to configure every application. Unsupported requirements must be explained.
Network listeners, firewall rules, sudo/polkit rules, arbitrary services/scripts,
paths, account creation and arbitrary Nix expressions are **not** available.

The daemon obtains the caller's account from Unix socket credentials, binds its
name and UID, and rechecks it at apply. Only normal user accounts can receive
groups. Generated Nix asserts that the account is declared as a normal user.
`libvirtd` access is powerful and is explicitly warned about in the review.
Log out completely and back in after adding or removing group access; existing
sessions can retain old groups until they end.

## Ownership, updates and rollback

Each setup is keyed by its primary package in the existing canonical
`.peasy/peasy-managed.nix` state. Updating a setup replaces its whole owned plan;
the AI must retain still-required settings. An existing standalone Peasy install
of the primary package is absorbed into the setup, avoiding an orphan on uninstall.
Supporting packages already installed independently by Peasy remain independent.

Packages, settings and group contributions are combined across setups. Removing
one drops only its contributions: shared dependencies remain, as does anything
declared in administrator configuration. No `false` override is written, and no
VM disks or user data are deleted. Explicit conflicting host settings cause the
ordinary NixOS build to fail and the managed source to be restored.

The existing expiring, user-bound proposal tokens, administrator approval,
transaction lock, state comparison, build/switch and generation reconciliation
apply unchanged. Rolling back to a Peasy generation restores its setup state as
well as packages and themes. Empty setup state retains the old canonical format.

## Extending the catalogue

Add reviewed option names/group prerequisites in `types.rs`, explain their effects
in `client.rs`, and test them against pinned Nixpkgs. Do not add keyword-to-package
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
module, list merging, uninstall withdrawal and host-setting conflicts; it does
not boot a VM. The sandbox VM also checks setup authorization and hostile IPC.
