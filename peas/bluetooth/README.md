# Bluetooth pea

Discover matching devices, reject ambiguous matches, connect, and attempt pairing/reconnection when necessary.

Example requests: “Connect to my headphones”.

`client.rs` owns discovery, address validation, review and the existing fixed BlueZ command sequence. Query and action types remain in the shared closed protocol.

This remains a reviewed session action using the existing desktop/device permissions. No daemon IPC, arbitrary address execution or additional privilege is introduced.

See the [peas contributor guide](../README.md) for shared wiring, tests and the
example prompt for adding an ability. Existing implementation tests are retained;
cross-pea wire/policy fixtures live in [tests/](../tests/).
