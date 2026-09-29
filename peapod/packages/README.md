# Packages pea

Verify an exact attribute or search host Nixpkgs, then propose install/remove.

- **Inputs:** known package attribute, search query with optional version, or
  managed package attribute for removal.
- **Checks:** install must select a validated candidate; removal must name a
  Peasy-managed package. The daemon verifies package identity against host Nixpkgs.
  Missing exact attributes fall back to search; requested versions search first.
- **Effects:** reviewed NixOS change with administrator authentication. Optional
  [system setup](../system_configuration/README.md) adds dependencies and integration.
- **Removal:** withdraw the owned contribution; retain shared dependencies.
- **Limits:** no arbitrary expressions or package versions absent from host Nixpkgs.
  Search may offer the explicit [AppImage fallback](../appimages/README.md).

`types.rs` defines values; `client.rs` handles selection; `system.rs` handles
lookup/proposals; `policy.rs` checks candidate and managed membership in Wasm.
Example: “Install hello.” Follow the shared [pea contract](../README.md).
