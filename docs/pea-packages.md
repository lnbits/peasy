# Pea packages

A pea is a versioned data manifest: instructions, capabilities, permissions,
host API and exact response schema. It composes installed host operations;
new executable capabilities require a Peasy update.

## Discovery and policy

The official catalogue is `peapod/catalogue.json`; each package has
`peapod/<id>/pea.json`. Build one with `nix build .#pea-networking`.

Discovery pins the official repository's `main` revision. A separate unprivileged
helper verifies catalogue membership, metadata, schema and SHA-256. The daemon
rechecks before review and after authorization. Downloads are bounded; redirects
and arbitrary repositories are rejected.

Review shows source, version, hash and permissions. Enabling a pea does not approve
its subsequent actions. Disabling it removes instructions, not earlier effects.
Pins travel with managed state, rebuilds and rollback. Restore may use historical
pins only after verifying published ancestry and current policy.

```nix
services.peasy.peas = {
  allowOfficial = true;
  allowedPermissions = [ "network.read" "network.session" "network.system" ];
};
```

Missing policy fails closed. Tightening permissions does not undo earlier changes.

## Authoring and compatibility

Follow the [pea contract](../peapod/README.md). The response schema must match the
host API projected onto declared permissions; instructions cannot widen it.
Preserve frozen API schemas and version intentional contract changes. Current API 6
adds app opening; API 5 added signed display positions and partial power updates.
Older compatible pins retain their original limits.

Regenerate and verify:

```sh
cargo run --locked -p peasy-core --example pea_catalogue
python3 scripts/pea-catalogue.py
cargo run --locked -p peasy-core --example pea_catalogue -- --check
python3 scripts/pea-catalogue.py --check
```

Publish manifests and catalogue together to the official repository. Test accepted
and hostile inputs, permission boundaries and pin round trips. Run the shared
[Rust and release checks](release-validation.md).
