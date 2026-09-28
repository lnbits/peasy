# Audio pea

Inspect PipeWire audio sinks/sources. Use a discovered object ID and native serial revalidation to set default speakers or microphone, volume from zero to 100 percent, mute or unmute. WirePlumber controls persistence. No recording, arbitrary node properties or per-application routing.

## Contract

- `inspect_resources` accepts domain `audio`; only service inspection accepts a target.
- `change_resources` accepts only this domain's typed operations. Every mutation requires review.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
