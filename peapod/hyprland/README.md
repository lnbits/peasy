# Hyprland pea

Inspect the running compositor and apply supported live settings or dispatchers.

- **Requires:** an accessible Hyprland session.
- **Inputs:** the closed setting/dispatcher enums and bounded values in `types.rs`.
- **Checks:** normalize values and detect the installed compositor API.
- **Effects:** after review, issue fixed `hyprctl` calls using the appropriate
  legacy or modern interface.
- **Persistence:** current session only; no configuration-file edits or NixOS
  rollback. Compositor reload/restart may restore configured values.
- **Limits:** no arbitrary Lua, commands, paths, plugins or dispatchers.

`types.rs` defines the accepted operations; `client.rs` handles discovery,
compatibility and execution. Example: “Set the inner gaps to 8.”
Follow the shared [pea contract](../README.md).
