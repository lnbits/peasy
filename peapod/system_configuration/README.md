# System configuration pea

Compose a primary package with supporting packages, allowlisted Boolean enable
options and caller access. Use `setup: null` for a plain install.

| Setup field | Limit |
| --- | --- |
| `packages` | 8 verified supporting attributes |
| `enable` | 8 catalogue options |
| `groups` | 4 catalogue groups with prerequisites |
| `postgresql` | Optional local server configuration |

[types.rs](types.rs) defines the accepted options and effects. No arbitrary Nix,
scripts, account names or option/value dictionaries are accepted. The daemon binds
access to the authenticated caller and verifies packages against host Nixpkgs.

Review includes service and access effects. Docker membership is effectively root
access; other groups can expose cameras, packet capture or hardware writes. Group
changes need logout/login. Conflicting host settings fail rather than being forced.

Replacing a setup replaces its owned plan. Removal retains shared dependencies,
administrator configuration and user data; it emits no disabling overrides.

## PostgreSQL

Supported versions provide a local server, optionally the caller's peer-authenticated
database without superuser access. Reject administrator-server takeover, conflicting
endpoints and major-version/data mismatches. Removal and rollback preserve data,
roles and grants. Remote access, passwords and migrations require manual management.

## Checks

```sh
nix build .#checks.x86_64-linux.system-configuration
nix build .#checks.x86_64-linux.postgresql-vm
```

Follow the [pea contract](../README.md); use reusable operations rather than
application-specific installer scripts.
