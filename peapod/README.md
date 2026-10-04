# Peas

A pea describes a domain and the operations the model may request. `pea.json`
contains instructions, permissions, version and schema. Downloaded peas are data;
new native capabilities require a reviewed host update.

## Design a domain, not a recipe

Expose reusable, typed operations based on discovered resources. Do not dispatch
by application keywords or add per-app installer scripts. Define preconditions,
effects, ownership, persistence, review, failure and recovery for every operation.

Keep provider access, validation, authorization, cancellation and transactions in
the shared host. Instructions cannot expand permissions. Secrets use local fields;
no arbitrary shell/Nix executor or Wasm host imports may be added.

## Native layout

| File | Responsibility |
| --- | --- |
| `types.rs` | Types, validation and rendering (`peasy-core`) |
| `client.rs` | Client discovery and session actions |
| `system.rs` | Daemon validation and proposals |
| `native.rs` | Fixed resource adapters |
| `policy.rs` | Pure Wasm policy, where needed |

Files are optional and included explicitly by their owning crates.

System changes use reviewed, authorized proposals and shared NixOS transactions.
Session changes use their service's permissions and persistence. A unique explicit
app-opening request may launch directly; it grants no system-change authority.
Generation rollback does not undo personal data or live service effects.

## Domains

[Packages](packages/README.md), [setup](system_configuration/README.md),
[AppImages](appimages/README.md), [appearance](appearance/README.md),
[networking](networking/README.md), [Wi-Fi](wifi/README.md),
[Bluetooth](bluetooth/README.md), [calendar](calendar/README.md),
[Hyprland](hyprland/README.md), [applications](applications/README.md).

[Resource peas](../docs/resources.md) cover diagnostics, services, storage, Nix
maintenance, users, firewall, printing, displays, audio and power.

## Add or extend a pea

1. Prefer a [data package](../docs/pea-packages.md#authoring-and-compatibility)
   when existing host operations suffice.
2. For new operations, update the owning native layers, schemas and dispatch.
   Preserve old meanings or explicitly version the host API.
3. Test validation, permissions, resource identity, review, cancellation and recovery.
   Persistent changes also need ownership, removal and rollback coverage.
4. Regenerate catalogue artifacts and update the domain's short README.

```sh
nix develop --command bash scripts/check-rust.sh
nix build .#peasy .#peasy-core
```

Run affected VM checks too. See [release validation](../docs/release-validation.md).
