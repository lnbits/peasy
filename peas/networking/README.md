# Networking pea

Discover NetworkManager resources and let the LLM compose a reviewed `NetworkPlan`.
Interfaces, connection UUIDs, addressing, gateways, DNS and routes are bounded,
nonsecret inputs. There is no hotspot-specific recipe dispatcher.

## Inputs and validation

`inspect_network` supplies a snapshot for a second model turn. `configure_network`
accepts complete profiles or activation changes through the host schema in
[types.rs](types.rs). Profiles specify identity, discovered interface, Ethernet/Wi-Fi
kind, Wi-Fi mode/SSID, IPv4 settings and autoconnect.

The host checks interface ownership, capabilities and profile identity. Live plans
retain their review snapshot and reject changed resources before mutation.
NetworkManager must already manage the discovered interface; this API does not
migrate another network manager.

## Persistence and execution

| Scope | Effects | Approval and recovery |
| --- | --- | --- |
| `system` | Upsert at most 8 Peasy profiles; remove at most 8 owned ids | Shared authenticated NixOS transaction and generation reconciliation |
| `session` | Activate/deactivate one discovered UUID, or activate one temporary profile | Local review and NetworkManager authorization |

Persistent profiles declare NetworkManager as a dependency and render mode-0600
keyfiles managed by NixOS. Rebuild reloads profiles; active connections retain live
settings until reactivated. Removal withdraws owned files; rollback restores files,
not live connectivity.

A system plan may request activation of one declared profile after the build.
This requires a separate local review and, for data peas, both `network.system`
and `network.session` permissions. Cancelling leaves the saved profile in place.

Temporary profiles use `save no` and may remain in `/run` until removed or reboot.
Do not assume logout or service restart removes them. Failed activation cleans up
the new profile and attempts to restore the prior connection; incomplete recovery
is reported. Success confirms connection state, not Internet reachability.

Passwords are collected locally and sent over stdin to `nmcli --ask`. They must
not enter model context, argv, logs, IPC, managed state or the Nix store. Generated
WPA2 profiles use `psk-flags = 2`; activation asks locally instead of saving a password.

## Supported operations and limits

- IPv4: automatic/DHCP, manual addresses, shared connectivity or disabled.
- Wi-Fi: WPA2 personal infrastructure or AP mode; check AP hardware support.
- New profiles disable IPv6, visibly in review. Existing profiles can be activated
  or deactivated by UUID without replacing their settings.
- Shared IPv4 delegates DHCP/DNS/NAT to NetworkManager and uses the default route.
  It does not pin an uplink. Persistent shared profiles add interface-scoped DNS/DHCP
  firewall ports; temporary profiles do not change the firewall.
- Custom firewall/VPN forwarding compatibility is not guaranteed. Arbitrary routing,
  IPv6 configuration, VPNs, bridges, bonds, 802.1X, open Wi-Fi and Wi-Fi backend changes
  are unsupported. Report these limits rather than inventing an execution path.

`types.rs` defines plans/rendering; `client.rs` discovers and applies local changes;
`system.rs` prepares persistent proposals. `pea.json` contains data-package metadata.
Example: “Share my Ethernet connection over Wi-Fi.” Follow the
[pea contract](../README.md) and [package contract](../../docs/pea-packages.md).
