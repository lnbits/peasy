# AppImages pea

Discover a compatible stable GitHub release and install its pinned AppImage.

- **Inputs:** search or repository, optional exact version, selected release asset.
- **Checks:** validate repository, URL, architecture, size and SHA-256; enforce
  administrator hash policy when configured. Prefetch does not execute the asset.
- **Review:** repository, release, download URL, architecture, size, hash and Nix diff.
- **Effects:** authenticated NixOS install/replacement/removal through `fetchurl`
  and `appimageTools.wrapType2` in the managed module.
- **Limits:** a hash pins bytes, not publisher authenticity or software safety.
  Removing the package does not remove application data.

`types.rs` defines records, policy and rendering; `client.rs` handles discovery
and prefetch; `system.rs` checks proposals. Example: “Find the latest AppImage
from owner/project.” Follow the shared [pea contract](../README.md).
