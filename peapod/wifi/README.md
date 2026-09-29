# Wi-Fi pea

List nearby networks and connect to an exact discovered SSID.

- **Requires:** NetworkManager running and managing the wireless device.
- **Inputs:** discovered SSID and, when required, a separate local password.
- **Checks:** SSID validation and discovery matching before local review.
- **Effects:** confirmed connection through fixed `nmcli` arguments under the
  user's NetworkManager permissions. No NixOS rebuild.
- **Secrets:** password goes to `nmcli --ask` over stdin, never model context,
  command arguments or system IPC.
- **Persistence:** NetworkManager controls the connection; this action does not
  create Peasy-managed Nix state or claim generation rollback.

`types.rs` validates SSIDs; `client.rs` handles scanning and connection.
For declarative profiles, use [networking](../networking/README.md).
Example: “Connect to Cafe.” Follow the [pea contract](../README.md).
