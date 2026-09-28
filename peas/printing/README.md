# Printing pea

Inspect CUPS queues and discovered driverless IPP printers. Add a new queue only for a discovered credential-free IPP URI, set the current user default, or send the fixed test page. CUPS must be running and existing CUPS authorization applies. No replacement of existing queues, driver downloads, arbitrary files or print-job contents.

## Contract

- `inspect_resources` accepts domain `printing`; only service inspection accepts a target.
- `change_resources` accepts only this domain's typed operations. Every mutation requires review.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
