# Wi-Fi pea

List visible networks and prepare a connection to an exact discovered SSID.

Example requests: “What Wi-Fi is available?”, “connect to Cafe”.

`types.rs` validates SSIDs. `client.rs` owns scanning, review and the fixed NetworkManager connection call.

Passwords come from the separate local field and are sent over stdin, not model text or process arguments. The shared client guards credentials before contacting a provider. This remains a reviewed session action, not a NixOS rebuild.

See the [peas contributor guide](../README.md) for shared wiring, tests and the
example prompt for adding an ability. Existing implementation tests are retained;
cross-pea wire/policy fixtures live in [tests/](../tests/).
