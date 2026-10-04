# Networking pea

Compose reviewed NetworkManager profiles from discovered interfaces and connection
UUIDs. NetworkManager must already manage the interface.

- **System:** up to eight profile additions/removals through authenticated NixOS
  changes. Reloading profiles does not reactivate current connections.
- **Session:** activate/deactivate one UUID or create one temporary profile.
  Failed activation attempts to restore the prior connection; recovery may be partial.
- Activation after a system change needs a separate review and both
  `network.system` and `network.session` permissions.

Supports IPv4 DHCP/manual/shared/disabled and WPA2 personal infrastructure/AP mode.
New profiles disable IPv6. Sharing uses the default route; it cannot pin an uplink.
Persistent shared profiles add interface-scoped DNS/DHCP ports. VPNs, bridges,
bonds, 802.1X, open Wi-Fi and arbitrary NetworkManager properties are unsupported.

Passwords stay in local fields and reach `nmcli --ask` through stdin. Temporary
profiles may remain until removed or rebooted. Rollback restores declared profile
files, not live connectivity; successful activation does not prove Internet access.

[Types and rendering](types.rs) · [Pea contract](../README.md)
