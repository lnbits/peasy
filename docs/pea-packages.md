# Pea packages and host API v1

Pea packages contain domain instructions, capability descriptions, permissions,
a version and an exact response schema. They are data; the installed Rust host
owns resource discovery, validation, authorization, execution and recovery. The
existing zero-import Wasm engine remains the policy boundary. Version 1 does not
load Rust libraries, scripts, remote Nix modules, or additional Wasm guests.

A new package can teach the AI to compose existing host operations without
rebuilding Peasy. Adding a new resource API or privileged effect requires a host
update. The same boundary applies to built-in and downloaded domain instructions.

## Official catalogue and request flow

`peas/catalogue.json` lists package ids, versions, capabilities, host API versions,
permissions and SHA-256 hashes. Each `peas/<id>/pea.json` is independently packaged;
`nix build .#pea-networking` builds only that data package. Other outputs use the
same `pea-<id>` convention. An installed application cannot load new Rust by finding
a folder in the repository.

The model can request `discover_peas` when installed capabilities cannot satisfy
the request. The client resolves `lnbits/peasy`'s `main` Git ref once, then reads
the catalogue at that immutable commit. Only compatible entries permitted by the
administrator are presented to the model. The user's request and local resources
are not sent to GitHub. Requests use fixed HTTPS origins, no redirects, timeouts
and bounded responses. An unavailable catalogue produces an error; there is no
fallback to arbitrary repositories or code.

The model selects `use_pea` with a returned id. The daemon independently checks
policy and pin syntax, asks Nix to fetch the fixed official manifest URL with
`--expected-hash`, validates its schema/metadata, and prepares a system proposal.
The review includes source, revision, hash, version and permissions. Apply rechecks
policy and the artifact, then uses the ordinary administrator authorization,
stale-state check, build verification and activation transaction.

After successful activation, CLI and GUI resume the original request. Enabled
pea instructions enter a bounded model turn; every returned action must be within
the pea's declared permissions before dispatch to the native host. The subsequent
network/package/desktop change retains its ordinary review. Enabling an ability
does not automatically approve its proposed changes. Cancelling the second review
leaves the installed pea enabled.

`disable_pea` removes an exact enabled pea through a reviewed system proposal.
Disabling instructions does not delete configuration previously created using
those instructions. Application state and pea-package ownership are separate.

## Nix state, exports and rollback

The canonical `.peasy/peasy-managed.nix` state records enabled pea ids, exact Git
revisions, SHA-256 hashes, versions, host APIs and permissions. The trusted renderer
uses `pkgs.fetchurl` for the data file and exposes it at `/etc/peasy/peas/<id>.json`.
No remote Nix expression is imported or evaluated. Nix stores downloaded files
immutably and the system generation retains their store references.

Normal builds, portable configuration exports and generation rollback carry these
pins through the existing managed-state mechanism. Exports retain reproducible
source references; a fresh machine still needs network access or a cache containing
the artifacts. Source availability is not guaranteed forever by a hash. The host
flake and `flake.lock` are not rewritten by on-demand installation.

Policy is declarative:

```nix
services.peasy.peas = {
  allowOfficial = true;
  allowedPermissions = [ "network.read" "network.session" "network.system" ];
};
```

By default, the module permits official discovery and all reviewed host permission
groups, with normal confirmation for installation and use. Missing policy files
fail closed. Tightening policy prevents loading or applying disallowed packages;
it does not undo their earlier effects. A hash pins content integrity; accepting
the fixed official repository is the publisher trust policy, not independent
cryptographic proof of authorship. System-level package authorization still applies.

## Authoring and compatibility

Start with the [domain design contract](../peas/README.md#design-a-domain-not-a-recipe).
Write instructions describing resources, constraints and supported intent, not
request-specific scripts. Existing native peas supply the reusable host operations.

`PeaManifest` denies unknown fields and limits metadata, instructions, capabilities
and permissions. Its `response_schema` must equal the host schema projected onto
the declared permission groups. Host-side validation also distinguishes networking
session permission from persistent system permission even though both use the
`configure_network` action. Packs cannot grant themselves permission to install
other packs. Unknown host APIs, schema changes and unsupported permissions fail
closed with a host-update requirement. Version the host API when changing this
contract; do not silently change schema semantics under the same API version.

For repository-maintained peas, regenerate and check the artifacts:

```console
cargo run --locked -p peasy-core --example pea_catalogue
python3 scripts/pea-catalogue.py
cargo run --locked -p peasy-core --example pea_catalogue -- --check
python3 scripts/pea-catalogue.py --check
```

The generator's small metadata list declares existing official packs. A new pack
using existing primitives needs instructions and metadata plus catalogue/package
publication; it does not need a new host action or Rust dispatcher. Tests check
manifest schemas, catalogue metadata/hashes, permission boundaries, closed action
fixtures and canonical pin round trips. Nix package checks run the generators in
verification mode. Publishing the catalogue and manifests to the official repository
is required before already-installed hosts can discover these new files.
