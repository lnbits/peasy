# Hyprland pea

Inspect the live session and apply the existing closed setting and dispatcher choices, preserving both legacy and modern hyprctl support.

Example requests: “What is my current workspace?”, “set the inner gaps to 8”.

`types.rs` owns setting/dispatcher enums, bounds and normalisation. `client.rs` owns status queries, review, compatibility detection and fixed command generation.

These are reviewed live-session changes, not persistent NixOS configuration. Arbitrary Lua, dispatchers, paths or compositor options are not accepted; reload retains the existing restore behaviour.

See the [peas contributor guide](../README.md) for shared wiring, tests and the
example prompt for adding an ability. Existing implementation tests are retained;
cross-pea wire/policy fixtures live in [tests/](../tests/).
