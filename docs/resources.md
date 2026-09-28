# Resource pea protocol

Host API 4 adds diagnostics, services, storage, Nix maintenance, users, firewall,
printing, displays, audio and power. These are native host capabilities with
data-only pea manifests. API 1–3 schemas and permissions remain unchanged.

## Discovery and permission

`inspect_resources` takes `resource_query`: a domain and nullable target. Only
services accepts a target, which must be an exact `.service` name. Discovery is
bounded and read-only. A pea requires `<domain>.read`; changes require
`<domain>.write`. Diagnostics has no write permission. The native permission gate
checks the domain inside the operation, including every model follow-up.

Inspection returns structured facts for a bounded model continuation. Raw journal
messages, command lines, password hashes and arbitrary files are excluded. Failed
services expose systemd result and exit status. Diagnostics reports a snapshot;
it does not prove a cause, test Internet reachability or scan the entire Nix store.
Store filesystem capacity is labelled separately from exact store size.

## Changes

`change_resources` takes one closed `resource_change` variant. Native validation
rejects unsupported fields, executable content, invalid identities and excessive
collections. The host discovers and captures the selected resource independently
of the model, shows the operation and its effects, and rechecks before execution.

| Domain | Initial operations | Persistence and boundary |
| --- | --- | --- |
| Diagnostics | Load, memory, disk capacity, pressure, failed units, routes | Read-only; no automatic repair |
| Services | List/status, start/stop/restart; supported service enablement | Live systemd control requires administrator approval; boot enablement uses NixOS |
| Storage | Inspect; removable mount/unmount/format; UUID mounts | UDisks authorizes live effects; `/mnt/peasy-NAME` mounts use NixOS |
| Nix maintenance | Generations, capacity, optimise, collect garbage, remove exact generations | Authenticated live operations; deleted data and rollback references cannot be restored by generation rollback |
| Users | Inspect; create locked account; disable/re-enable owned account; caller groups | NixOS; account names and UIDs are bound and checked; home data survives |
| Firewall | Inspect declared policy; TCP/UDP ports and trusted interfaces | NixOS contributions; administrator and other contributions remain |
| Printing | Discover, add driverless IPP queue, user default, test page | Existing CUPS authorization and persistence |
| Displays | Mode/refresh, position, scale, primary where supported | GNOME, Plasma or Hyprland session; desktop persistence varies |
| Audio | Sink/source inspection, default, volume, mute | PipeWire/WirePlumber; existing session authority |
| Power | Battery/profiles; select profile; lid/idle policy | Profiles use desktop authorization; logind policy uses NixOS |

Persistent changes are recorded in managed state and use the existing UID-bound
proposal, administrator authentication, build, verification and activation flow.
Resource definitions merge with package setups. Rollback reconciles the managed
source. Portable restore preserves destination resource settings in both modes;
it does not import another machine's device or account bindings.

Privileged live changes use the same authenticated proposal flow without a NixOS
build. Generation inspection/deletion and block-device discovery use a private, short-lived
helper; the main daemon retains its device and filesystem sandbox. The helper
accepts three closed tasks and cannot receive commands or paths to execute.

Session mutations retain the discovered snapshot in their local review. Hardware
replacement, changed service identity and reused audio IDs require a new review.
Once mutation begins, cancellation does not pretend to undo it. Tool failure is
reported; live operations do not claim transactional or generation rollback.

## Limits

- Storage refuses internal/system disks, layered devices, non-leaf targets and
  mounted format targets. Formatting erases a filesystem, never a partition table.
  Repartitioning, LUKS and SMART diagnostics are outside this version.
- Service control excludes Peasy, systemd, D-Bus, Nix daemon, Polkit and display
  manager infrastructure. Persistent disable withdraws only owned enablement.
- Generation deletion protects the profile's current system, active/booted systems
  and a distinct rollback generation. Garbage collection does not delete generation
  roots. No flake/channel mutation or arbitrary rebuild command is exposed.
- Account disable does not terminate existing sessions. Re-enabling does not set
  a password; use local account tools. Existing administrator accounts cannot be
  disabled through this API. Passwords and arbitrary shell changes are excluded.
- Firewall trusted interfaces allow all incoming traffic. Source-limited rules,
  custom nftables and proof of external port reachability are excluded.
- Printers must be discovered, credential-free IPP endpoints. Existing queues
  cannot be overwritten; custom drivers and arbitrary documents are excluded.
- Displays preserve other outputs. GNOME layouts with HDR, underscanning or
  mirrored target displays, custom modes,
  output disabling and Hyprland primary selection are unsupported. GNOME changes
  require the packaged `gdctl` tool. No automatic display-revert timer is provided.
- Power policy does not enable a daemon or configure hibernation storage. Desktop
  inhibitors may override logind. Sleep inhibition and forced shutdown are excluded.

No generic files, terminal, shell or commands pea is added. Native adapters live in
each domain's `native.rs`; shared types, dispatch and proposal handling remain in
the host. Follow the [pea contract](../peas/README.md).
