# System configuration pea

Compose a selected package with supporting packages, NixOS options and caller
access. The LLM chooses the composition; the host validates and reviews it.
Do not add application-name matching or per-application installer scripts.

## Inputs

`install_package` accepts a primary candidate and optional `setup`. Use
`setup: null` for a plain install. A setup contains:

| Field | Accepted values | Limit |
| --- | --- | --- |
| `packages` | Validated supporting Nixpkgs attributes | 8 |
| `enable` | Boolean option names from `SYSTEM_ENABLE_OPTIONS`; contributes `true` | 8 |
| `groups` | Groups from `SYSTEM_GROUPS`, with declared prerequisites | 4 |
| `postgresql` | `null` or `{ package, caller_database }` | One local server configuration |

[types.rs](types.rs) is the authoritative catalogue of options, effects, group
prerequisites and PostgreSQL versions. Schema generation and review use that
catalogue. The current API has no arbitrary option/value map, Nix source, scripts,
account creation, sudo rules or model-selected account names.

The primary package must be a returned candidate. The daemon verifies supporting
packages against host Nixpkgs and binds group/database access to the authenticated
socket caller's normal-user account. It rechecks the account at Apply.

## Review and application

Show the generated Nix diff and catalogue effects, including service startup and
additional access. Docker membership grants effectively root access; other groups
can grant packet capture, camera or hardware access. Group changes require a full
logout/login; existing sessions can retain old access.

Apply uses the shared administrator authorization, expiring UID-bound proposal,
stale-state checks, NixOS build, result verification and activation. Conflicting
host settings fail the build; Peasy does not use `mkForce` to override them.
Success means the generation was applied, not that application workflows were tested.

Integration rules belong to the host contract: AppImage binfmt requires AppImage
support; Flatpak adds a GTK portal fallback while preserving desktop portal choices.
Flatpak remotes/apps, application preferences, vendor graphics drivers, CUDA and
32-bit graphics are outside this setup API. The catalogue describes other limits.

## Ownership and removal

Each setup is keyed by its primary package. Replacing a setup replaces its entire
owned plan, so the proposal must retain still-required contributions. An existing
standalone Peasy install of the primary package is absorbed into that setup.
Supporting packages installed independently remain independent.

Contributions are merged across setups. Removal withdraws only that setup's
packages, options and groups; it retains shared dependencies, administrator
configuration and user data. It emits no `false` overrides. Generation reconciliation
restores Peasy's managed source as well as the active configuration.

## PostgreSQL contract

- Select a versioned package from the catalogue; the daemon verifies host availability.
- Run a local server on the standard port and Unix socket, with no firewall opening.
- `caller_database: true` creates the caller's database and peer-authenticated role,
  owning that database without superuser access.
- Reject takeover of an administrator-managed server, conflicting endpoints/custom
  data directories, major-version changes and retained data from another major.
- Preserve database contents, roles and grants on removal and configuration rollback.
  Removing `ensureUsers` does not revoke existing database access.
- Remote access, passwords, arbitrary role/database names and migrations are unsupported.

Explicit server requests include supported setup; explicit client-only requests
remain package installs. Ambiguous requests must ask which is wanted. Unsupported
requirements receive manual guidance, which Peasy displays but never executes.

## Extension and verification

Reuse existing operations first. Add a missing reusable option/value capability
through a reviewed host API change, including rendering, ownership, visible effects
and compatibility. A broader structured NixOS interface requires such a change;
adding a data pea cannot bypass today's catalogue. Follow the [pea contract](../README.md).

[tests.rs](tests.rs) and daemon tests cover validation, ownership and failure.
[example.json](example.json) and [postgresql-example.json](postgresql-example.json)
are daemon-bound fixtures; their account fields are never model inputs.

```console
nix build .#checks.x86_64-linux.system-configuration
nix build .#checks.x86_64-linux.postgresql-vm
```

The first evaluates production-rendered options, groups, merging and conflicts.
The VM checks database access, startup and retained data across removal/restoration.
Run shared Rust/Wasm checks and affected transaction tests as well.
