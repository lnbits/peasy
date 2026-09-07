# Appearance pea

List supported appearance choices, persist existing theme enums through NixOS, and apply them to supported live desktops.

Example requests: “Which themes can I use?”, “change to a green dark theme”.

`types.rs` owns theme enums; `desktop.rs` owns desktop detection/capabilities; `client.rs` owns review and live theme helpers; `adapters.rs` owns fixed GNOME/Plasma calls; `system.rs` owns declarative proposals.

Supported choices and fallback behaviour are unchanged. Declarative Apply retains administrator authorization. Wallpaper requests are not added; the ISO's wallpaper remains build-time branding.

See the [peas contributor guide](../README.md) for shared wiring, tests and the
example prompt for adding an ability. Existing implementation tests are retained;
cross-pea wire/policy fixtures live in [tests/](../tests/).
