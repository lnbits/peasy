# Users pea

Inspect local normal accounts and supplementary memberships without reading password hashes or home contents. Create a normal account with a locked initial password, disable or re-enable a Peasy-created account, or replace Peasy-owned groups for the authenticated caller. Preserve administrator accounts and memberships. Existing sessions and home data are retained. Password changes and login-shell customisation remain local account-tool operations.

## Contract

- `inspect_resources` accepts domain `users`; only service inspection accepts a target.
- `change_resources` accepts only this domain's typed operations. Every mutation requires review.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
