# Resource pea protocol

Host API 4 adds diagnostics, services, storage, Nix maintenance, users, firewall,
printing, displays, audio and power. These are native host capabilities with
data-only pea manifests. API 5 adds signed display positions and independently
optional power settings. API 1–4 schemas and permissions remain unchanged.

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

The CLI asks for a second confirmation after a display apply; the desktop shows a
countdown with Keep and Revert. The panel worker emits `confirm_display` with
`seconds: 20` and accepts `{"action":"keep_display"}` during that interval. Cancel,
EOF or timeout reverts. The final result reports whether settings were kept or
restored; model responses cannot supply this confirmation.

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
  require the packaged `gdctl` tool. Positions are bounded to −16384…16384; GNOME
  translates the complete layout to a non-negative origin. A separate session
  watchdog restores the previous layout after 20 seconds unless the user selects
  Keep. Revert, closing Peasy, or a failed apply also triggers restoration. One
  trial may run per user session. Restoration failures are reported when the
  client remains open; disconnected hardware or a stopped compositor can prevent
  recovery. Killing the watchdog itself prevents its timer from running.
- Power settings merge only non-null fields into Peasy’s existing contribution.
  `lid` and `idle_minutes` are independently nullable; both null is invalid. Omitted
  fields in native requests have the same preservation semantics. Zero disables
  idle suspension. Existing full power records remain readable.
- Power policy does not enable a daemon or configure hibernation storage. Desktop
  inhibitors may override logind. Sleep inhibition and forced shutdown are excluded.

No generic files, terminal, shell or commands pea is added. Native adapters live in
each domain's `native.rs`; shared types, dispatch and proposal handling remain in
the host. Follow the [pea contract](../peas/README.md).

## Regression checks

- `resources-lifecycle-vm` formats, mounts and unmounts a disposable USB disk with
  real UDisks. It rejects mounted, system and device-mapper targets, activates a
  UUID mount, and checks real firewall traffic and account lifecycle behavior.
  Its NixOS generations are built from the Rust renderer before VM boot, then
  activated with the real switch script. Proposal ownership checks use the daemon.
- `resources-session-vm` exercises a PipeWire null sink, power-profiles-daemon and
  an emulated IPP printer through the native adapters.
- Rust display tests cover all three backend command plans and a separate worker
  process: Keep, Revert, timeout, disconnect, stale snapshots, concurrent trials
  and partial apply failure. These fixtures do not replace physical multi-monitor
  testing on each desktop.

Run the VM checks with `nix build .#checks.x86_64-linux.resources-lifecycle-vm
.#checks.x86_64-linux.resources-session-vm`; run the Rust checks with
`nix develop --command bash scripts/check-rust.sh`.
