# Resource peas

`inspect_resources` returns bounded, read-only facts. `change_resources` accepts
closed typed operations. Peas need domain-specific read/write permissions;
diagnostics has no write permission. Resource identity is rechecked before apply.

| Pea | Supported operations |
| --- | --- |
| [Diagnostics](../peapod/diagnostics/README.md) | Load, memory, capacity, pressure, failed services and routes |
| [Services](../peapod/services/README.md) | Status, start/stop/restart, supported boot enablement |
| [Storage](../peapod/storage/README.md) | Removable filesystems and UUID mounts |
| [Nix maintenance](../peapod/nix_maintenance/README.md) | Generations, optimisation and garbage collection |
| [Users](../peapod/users/README.md) | Locked accounts, owned account enablement and caller groups |
| [Firewall](../peapod/firewall/README.md) | Peasy-owned ports and trusted interfaces |
| [Printing](../peapod/printing/README.md) | Discovered IPP printers, user default and test page |
| [Displays](../peapod/displays/README.md) | Supported modes, position, scale and primary output |
| [Audio](../peapod/audio/README.md) | Speakers/microphones, defaults, volume and mute |
| [Power](../peapod/power/README.md) | Profiles and persistent lid/idle policy |
| [Applications](../peapod/applications/README.md) | Discover and open installed desktop entries |

Persistent changes use the shared authenticated NixOS transaction. Privileged live
changes require authorization; session operations use the receiving service's
permissions. Only service inspection accepts a target, as an exact `.service` name.

## Important limits

- Formatting erases data; only eligible removable filesystems are accepted.
- Garbage collection and generation deletion cannot be undone by rollback.
- Account disable preserves homes and does not terminate existing sessions.
- Trusted firewall interfaces allow all incoming traffic.
- Display changes require **Keep** within 20 seconds; timeout or disconnect triggers
  restoration. Removed hardware or a failed compositor can prevent recovery.
- Peasy preserves administrator contributions. Portable restore keeps destination
  resource bindings. Live changes do not claim generation rollback.

No arbitrary commands, paths, raw logs or credentials enter resource operations.
See each pea for its exact limitations and the [shared contract](../peapod/README.md).

## Checks

```sh
nix develop --command bash scripts/check-rust.sh
nix build .#checks.x86_64-linux.resources-lifecycle-vm .#checks.x86_64-linux.resources-session-vm
```

Physical display, storage and peripheral testing remains necessary.
