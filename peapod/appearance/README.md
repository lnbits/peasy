# Appearance pea

List supported theme choices and apply a reviewed declarative theme.

- **Inputs:** supported accent colour and colour-scheme enums.
- **Checks:** detect desktop capabilities and reject unsupported choices.
- **Effects:** authenticated NixOS configuration plus a local desktop update.
  Login and generation changes synchronize the configured theme.
- **Support:** GNOME and Plasma accents and light/dark modes; GNOME also supports
  system-default mode. Other desktops have no appearance adapter.
- **Limits:** no wallpaper changes or arbitrary desktop settings. ISO wallpaper
  is build-time configuration.

`types.rs` defines values; `desktop.rs` detects capabilities; `adapters.rs` owns
fixed desktop calls; `client.rs` and `system.rs` prepare/apply changes.
Example: “Change to a green dark theme.” Follow the [pea contract](../README.md).
