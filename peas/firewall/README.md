# Firewall pea

Inspect structured NixOS firewall configuration and interfaces. Replace Peasy-owned TCP/UDP port and trusted-interface contributions through reviewed NixOS state. Preserve unrelated Peasy entries and administrator rules. Trusted interfaces allow all incoming traffic; never substitute them for source-restricted rules. No arbitrary nftables expressions, source-limited rules or guarantee that a removed port is closed.

## Contract

- `inspect_resources` accepts domain `firewall`; only service inspection accepts a target.
- `change_resources` accepts only this domain's typed operations. Every mutation requires review.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
