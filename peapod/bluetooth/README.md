# Bluetooth pea

Discover a device by name, review its address, then connect or pair.

- **Requires:** enabled Bluetooth service and available hardware.
- **Inputs:** device-name query resolved to one discovered hardware address.
- **Checks:** reject missing or ambiguous matches and invalid addresses.
- **Effects:** after confirmation, try connection; on failure, try pairing and
  reconnecting through fixed BlueZ commands under existing user permissions.
- **Persistence:** BlueZ may retain pairing. No NixOS rebuild or generation undo.
- **Limits:** no arbitrary commands or privileged system IPC.

`client.rs` owns discovery, validation and execution; shared types define actions.
Example: “Connect to my headphones.” Follow the [pea contract](../README.md).
