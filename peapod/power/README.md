# Power pea

Inspect battery status and available power profiles. Select an existing power-profiles-daemon profile with desktop authorization. Configure persistent logind lid handling and idle suspension through NixOS; desktop inhibitors may override it. Do not claim hibernation works without host support. No automatic installation of competing power managers, sleep inhibition or forced shutdown.

## Contract

- Specify `lid`, `idle_minutes`, or both. Null or omitted fields preserve Peasy’s existing contribution; both absent is invalid. Zero disables idle suspension.

- `inspect_resources` accepts domain `power`; only service inspection accepts a target.
- `change_resources` accepts only this domain's typed operations. Every mutation requires review.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
