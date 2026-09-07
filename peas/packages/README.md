# Packages pea

Search and check Nixpkgs software, select an exact returned candidate, and propose installation or removal. The bounded package-agent loop can still hand off to the AppImages pea.

Example requests: “Install hello”, “is VLC available?”, “remove hello”.

`types.rs` owns package attributes/versions and validation; `client.rs` owns selection and review; `system.rs` owns pinned Nixpkgs lookup, caches and proposals; `policy.rs` owns Wasm candidate/managed-membership checks.

Read-only queries go through the existing daemon. Applying a system proposal still requires its existing administrator authorization and transaction path.

See the [peas contributor guide](../README.md) for shared wiring, tests and the
example prompt for adding an ability. Existing implementation tests are retained;
cross-pea wire/policy fixtures live in [tests/](../tests/).
