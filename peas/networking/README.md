# Networking pea

A domain adapter for NetworkManager resources and guarded changes. Peasy discovers
interfaces, connection UUIDs, device capabilities, IPv4 addresses, gateways, DNS
and routes. The AI reasons from that bounded, nonsecret snapshot and the request
to a `NetworkPlan`; the host validates, reviews and executes it. There is no
hotspot-specific intent matching or recipe dispatcher.

Example requests exercise the same primitives:

- “Share my Ethernet connection over Wi-Fi.”
- “Use DHCP on this Ethernet adapter.”
- “Give this interface a static IPv4 address and these DNS servers.”
- “Activate that connection”, “disconnect it”, or “remove this Peasy profile”.
- “Explain which interfaces and routes I have.”

`inspect_network` obtains data for a second model turn. `configure_network`
accepts `network`, whose closed schema is exported by the core host API. Profiles
have an id, a discovered interface, Ethernet/Wi-Fi type, Wi-Fi mode/SSID, IPv4
method, addresses, gateway, DNS, and autoconnect. The model supplies complete
replacement profiles, not commands or arbitrary NetworkManager properties.

## Persistence and execution

`scope: system` upserts up to eight Peasy-owned profiles and removes up to eight
owned ids through the existing authenticated NixOS transaction. Profiles render
as mode-0600 NetworkManager keyfiles managed by NixOS `/etc`; removing a profile
withdraws its file on rebuild and rollback restores the selected generation's
files. NixOS reloads the profiles after a state change. Existing active connections
retain their live settings until explicitly reactivated.

A system plan can set `activate` to one of its declared profile ids. After the
system change succeeds, CLI and GUI prepare a separate local activation review
against the newly discovered UUID. Password input happens at this local stage.
Cancelling that review leaves the already-saved configuration in place.

`scope: session` activates/deactivates one discovered connection UUID, or creates
and activates one temporary profile with `save no`. It does not modify existing
profiles or persist new ones in NixOS. Temporary profiles can remain in NetworkManager
runtime storage under `/run` until removed or reboot; do not rely on a daemon
restart or logout to remove them. Failed activation attempts clean
up the newly created profile and attempt to restore the prior interface connection;
incomplete recovery is reported. Successful activation is checked against observed
connection state, not claimed as proof of Internet reachability.

Live plans retain their reviewed snapshot and reject changed resources before
mutation. Native code chooses all executable paths and arguments. Read subprocesses
use the shared cancellation and output limits. Passwords travel over stdin to
`nmcli --ask`, never in model text, argv, logs, IPC, managed state, or the Nix store.
WPA2 credentials are not saved (`psk-flags = 2`); activation asks locally.

## Host API v1 limits

IPv4 supports automatic/DHCP, manual addresses, shared connectivity and disabled.
Wi-Fi supports secured WPA2 personal infrastructure and AP modes, with AP hardware
capability checked before review. IPv6 is disabled on newly created profiles and
shown in the review. Other existing profiles can be activated/deactivated by UUID.

Shared IPv4 delegates DHCP, DNS and NAT to NetworkManager and follows the current
default route. It does not pin a specific uplink; the AI must assess discovered
routing and explain ambiguous or unsupported routing requirements. Persistent
shared profiles contribute DNS/DHCP firewall ports on their interface only;
temporary profiles do not change the host firewall. Forwarding compatibility with
custom firewall/VPN policies is not guaranteed by profile creation.

NetworkManager must already manage the discovered interface. Migrating another
network manager, arbitrary routing/firewall changes, IPv6 configuration, VPNs,
bridges, bonds, 802.1X, open Wi-Fi, and changing the Wi-Fi backend are not host API
v1 primitives. Explain unsupported requirements; do not invent an escape hatch.

The native adapter lives in `types.rs`, `client.rs`, and `system.rs`. The separately
packaged `pea.json` carries its instructions, exact host response schema and
permissions. See [package loading](../../docs/pea-packages.md) and the
[domain design guidance](../README.md#design-a-domain-not-a-recipe).
