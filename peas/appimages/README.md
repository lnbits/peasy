# AppImages pea

Discover compatible stable GitHub releases, prefetch a selected asset to obtain its hash, and propose reviewed installation, replacement or removal.

Example requests: “Find the latest AppImage from owner/project on GitHub”.

`types.rs` owns pinned package records, URL/hash validation, administrator policy and Nix bindings. `client.rs` owns unprivileged GitHub discovery/prefetch. `system.rs` owns review details and policy-checked proposals.

The repository, release, URL and hash remain visible for review. Downloading does not execute an AppImage. The optional administrator hash policy and normal Apply authorization remain enforced.

See the [peas contributor guide](../README.md) for shared wiring, tests and the
example prompt for adding an ability. Existing implementation tests are retained;
cross-pea wire/policy fixtures live in [tests/](../tests/).
