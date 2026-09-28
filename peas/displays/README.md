# Displays pea

Inspect and change GNOME, Plasma or Hyprland display layouts using exact discovered outputs and modes. Propose position, refresh/mode, supported scale and primary output where the desktop supports it. Preserve other outputs. Hyprland has no primary flag; use false. No output disabling, mirrored GNOME layout edits, custom modes, compositor configuration files or claims of NixOS rollback.

## Contract

- `inspect_resources` accepts domain `displays`; only service inspection accepts a target.
- `change_resources` accepts only this domain's typed operations. Every mutation requires review.
- Native discovery bounds responses; resource identities and preconditions are checked again before applying.
- Persistent changes use the shared authenticated NixOS transaction. Live changes use their service's authorization and persistence, and do not claim generation rollback.
- Model explanations are never executed. Unsupported operations return an explicit limitation.

`native.rs` implements the fixed resource adapter, compiled into the shared native host.
The [resource protocol](../../docs/resources.md) defines permissions, ownership and recovery.
Follow the [pea contract](../README.md).
