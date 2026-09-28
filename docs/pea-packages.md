# Pea package protocol

Pea packages contain domain instructions, capability descriptions, permissions,
a version and an exact response schema. They are data; the installed Rust host
owns resource discovery, validation, authorization, execution and recovery. The
existing zero-import Wasm engine provides additional policy containment. Peasy does not
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
policy and pin syntax, then starts the fixed `peasy-pea-fetch.service`. This
unprivileged, short-lived helper independently reads the official `main` ref and
requires an exact catalogue entry at that revision (including version, host API,
hash and permissions). It downloads at most 64 KiB of manifest data, rejects
redirects, and verifies SHA-256, schema and metadata. Catalogue/ref responses are
also bounded; each HTTP request has a 30-second deadline and the service has a
100-second deadline. One verification runs at a time.

The root daemon retains its network-denying sandbox. It checks the helper's bytes
again, copies them to private staging, and imports that file into Nix without a
network fetch. The review includes source, revision, hash, version and permissions.
Apply repeats source verification after normal administrator authorization, then
uses the existing stale-state check, build verification and activation transaction.
If `main` moved since discovery, new installation requires fresh discovery/review.
Existing enabled pins, ordinary rebuilds and rollback do not follow `main`.
The explicit backup-restore flow also accepts a historical pin after the helper
proves its revision is an ancestor of independently fetched official main using
GitHub's bounded comparison response. An exact catalogue entry, artifact hash,
compatible schema and current permission policy are still required. Unpublished
branch/PR commits are rejected. This check runs during restore review and again
after authorization; ordinary discovery continues to require current main.

Manifest release 1.3.0 uses host API 4 for the [resource domains](resources.md).
Each domain has separate read/write permissions; diagnostics is read-only.
API 1–3 retain their exact schemas and cannot request the new operations.

Manifest release 1.2.0 used host API 3 for the expanded setup catalogue and longer
manual instructions. Older hosts will not offer incompatible new manifests.
The new host still accepts exact API 1, API 2 and API 3 schemas and immutable pins,
including disabling and restoring those pins. API 2 added PostgreSQL; API 3 adds
setup options and device groups. Arbitrary schema edits and permission expansion
remain rejected.

Source authentication uses fresh HTTPS rather than trusting a claimed catalogue
hash or a previously populated Nix cache. The helper cannot access the Nix daemon
socket or Peasy's private runtime directory; it receives only the request file
through a read-only bind mount. Its output directory is mode `0750`, with files
mode `0640`; a dedicated group lets the daemon read them without granting it
filesystem permission override capabilities.

After successful activation, CLI and GUI resume the original request. Enabled
pea instructions enter a bounded model turn; every returned action must be within
the pea's declared permissions before dispatch to the native host. The subsequent
network/package/desktop change retains its ordinary review. Enabling an ability
does not automatically approve its proposed changes. Cancelling the second review
leaves the installed pea enabled. Downloaded capability descriptions participate in
a selection-only model turn: only an exact enabled pea id can leave that turn.
They cannot directly dispatch an unrestricted host action. A persistent networking
plan with deferred activation requires both `network.system` and `network.session`.
Before preparing that continuation, the client rechecks its originating pin,
enabled state and administrator policy.

`disable_pea` removes an exact enabled pea through a reviewed system proposal.
Disabling instructions does not delete configuration previously created using
those instructions. Application state and pea-package ownership are separate.

## Nix state, exports and rollback

The canonical `.peasy/peasy-managed.nix` state records enabled pea ids, exact Git
revisions, SHA-256 hashes, versions, host APIs and permissions. The trusted renderer
uses `pkgs.fetchurl` for the data file and exposes it at `/etc/peasy/peas/<id>.json`.
No remote Nix expression is imported or evaluated. Nix stores downloaded files
immutably and the system generation retains their store references. Rebuild fetches
also have byte/time limits and reject redirects. Exact older managed modules are
accepted for migration; the next managed write adds those bounds.

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
Both providers receive the originating pea's response schema. Native checks enforce
its API-specific option, group, PostgreSQL and message limits even if the model
ignores that schema. Package searches and selection follow-ups retain the same
pea contract; they cannot silently switch to the current host's broader schema.

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

## Checks before merging

Pull requests and pushes to `main` run `.github/workflows/ci.yml`. Its Rust job runs
`nix develop --command bash scripts/check-rust.sh`: formatting, the compiled Wasm
guest, all workspace tests including integration tests, Clippy with warnings denied,
and both catalogue generators in verification mode. Its Nix job enumerates and
builds every native flake check in separate processes to release evaluator memory
between checks, then validates with `nix flake check --no-build`. Building first
provides the daemon-generated networking fixture needed during evaluation. This includes
the networking, source-fetch and privileged-sandbox NixOS VMs. A failed check fails
the job, even while other checks continue. Locally, `nix flake check --keep-going -L`
runs the combined suite on machines with enough memory. Tests use fixture-only credentials and a local HTTPS
server; they do not change the workstation's connections or contact GitHub for
fixture downloads. The ISO publication workflow remains separate.
