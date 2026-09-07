# Peas: Peasy's abilities

A pea is a built-in, reviewed ability, organised in one folder. This is a source
layout, not a downloadable plugin system: adding a folder does not enable code
automatically. Peas are explicitly wired into Peasy's existing Rust crates and
compiled with the application. No new process, permissions, configuration file
or runtime registration mechanism is introduced.

The initial split moves existing functionality without changing prompts, action
names, JSON formats, review messages, command arguments, defaults or persistence.

## Existing peas

| Folder | Abilities | Owned code |
| --- | --- | --- |
| [packages](packages/) | Nixpkgs search, availability checks, exact-version selection, install/remove, follow-up selection | `types.rs`, `client.rs`, `system.rs`, `policy.rs` |
| [system_configuration](system_configuration/) | Generic package setup and uninstall: supporting packages, reviewed NixOS enable options and caller-bound groups | `types.rs`, `client.rs`, `system.rs` |
| [appimages](appimages/) | GitHub discovery, release/architecture selection, prefetch/hash, reviewed install/update/remove, administrator policy | `types.rs`, `client.rs`, `system.rs` |
| [appearance](appearance/) | Supported theme choices, declarative accents/light/dark settings, live GNOME/Plasma application | `types.rs`, `desktop.rs`, `client.rs`, `adapters.rs`, `system.rs` |
| [wifi](wifi/) | List nearby networks and connect using a separately supplied local password | `types.rs`, `client.rs` |
| [bluetooth](bluetooth/) | Discover matching devices, connect and pair when needed | `client.rs` |
| [calendar](calendar/) | Validate local dates, prepare private iCalendar files, open the default calendar handler | `types.rs`, `client.rs` |
| [hyprland](hyprland/) | Inspect a running session and apply the existing bounded live settings/dispatchers, with legacy/modern CLI support | `types.rs`, `client.rs` |

`tests/` holds cross-pea compatibility fixtures. Ability-specific tests live next
to their implementation; shared state, authorization and transaction tests stay
in the crates that enforce those contracts.

## How the files fit together

The same pea folder can contribute source to different compilation boundaries:

| File | Compiled by | Responsibility |
| --- | --- | --- |
| `types.rs` | `peasy-core` | Existing closed types, constants and validation; re-exported under the same `peasy_core` names |
| `client.rs` | `peasy-client` | Unprivileged discovery, review proposals and session execution; shared by CLI and GUI |
| `system.rs` | `peasy-system::nix_backend` | Privileged-side verification and declarative proposals, using the existing backend |
| `policy.rs` | `peasy-engine` | Pure ability-specific policy, compiled into the zero-import Wasm guest |

Not every pea needs every file. Bluetooth uses the shared query/action types and
validates discovered addresses in its client. Only packages currently need
additional Wasm membership rules; the engine's other existing arms forward
already-typed values. Appearance also owns desktop detection and adapters.

Rust `#[path = "../../../peas/<name>/<layer>.rs"]` declarations in the owning
crates make these connections explicit. A `system.rs` file is a child of the
Nix backend, not part of the unprivileged client. Client code and its networking
dependencies are never pulled into Wasm by registering a pea.

This avoids putting privileged execution, HTTP clients and pure policy in one
plugin crate. It also preserves existing public APIs used by the CLI, GUI and
tray. Helpers remain private to their owning crate.

## What stays shared

- `crates/peasy-client/src/lib.rs`: provider/key storage, credential checks before
  contacting the model, exact model prompts/schema, bounded context, orchestration,
  public proposal types, progress messages and exhaustive dispatch.
- `crates/peasy-core/src/lib.rs`: the closed model/IPC enums and envelope decoding,
  shared validation, canonical managed state, migration, Nix rendering and diffs.
- `crates/peasy-engine/src/lib.rs`: Wasm ABI and exhaustive action routing.
- `crates/peasy-engine-host/`: import rejection, memory/fuel limits and typed calls.
- `crates/peasy-system/`: peer identity, authorization, proposal expiry/replay
  checks, locking, atomic state handling, build validation and activation.
- `nix/`: trusted packaging, fixed tool paths, system sandbox and installer wiring.

Package search can still hand off to AppImage discovery. Appearance still uses
the normal declarative apply path followed by its unprivileged desktop adapter.
Wi-Fi credential guarding remains a shared pre-model check, not an optional
step that another pea can accidentally omit.

## Security and compatibility rules

Peas are trusted application code, **not security sandboxes for third-party Rust**.
A malicious source contribution could misuse its owning process's authority;
it requires normal review and a rebuild. There is no claim that sibling Rust
modules isolate credentials from malicious contributors.

The AI still receives only constructed data and returns a closed typed action.
It cannot choose a pea file, executable, shell command, Nix expression or an
arbitrary setting/path. Wasm has no imports, WASI, filesystem or network access.
Per-user operations keep their existing review/confirmation flow. System Apply
still requires daemon-side administrator authorization and a UID-bound proposal.
Do not add a new direct activation or configuration-writing route inside a pea.

Do not expose provider credentials through a new context field, log or error.
Secrets must use separate local input and must not go into model text or command
arguments. External discovery is data, never instructions or trusted code.

`.peasy/peasy-managed.nix` remains the source of truth for Peasy-managed system
state. Existing NixOS builds, generations and rollback handling are unchanged.
Session changes retain their current semantics: calendar import is not proof an
event was saved, and live Hyprland changes are not persistent NixOS settings.

## Adding a pea

1. Define a small capability, supported desktops, read/write effects, confirmation
   requirements and failure behaviour. Start with the least authority needed.
2. Add `peas/<name>/client.rs` and tests. Add `types.rs`, `system.rs` or `policy.rs`
   only when that responsibility is actually needed. Use explicit imports and
   existing shared helpers; do not copy a second transaction or provider stack.
3. Register source modules in their owning crates. Extend the closed
   `ModelAction`, `ModelEnvelope` validation and `EngineDecision` as necessary.
   Wire exhaustive matches in the engine and client. Do not add a catch-all
   command action or dynamic dispatcher.
4. Add model instructions/schema fields deliberately. Keep existing meanings and
   names; retain `additionalProperties: false`, bounded data and local validation.
   Update `tests/model-schema.json`, the prompt fixtures and `tests/actions.json`
   only for intentional additions. Existing action fixtures must still pass.
5. For a session change, add a typed `LocalAction`, reviewable `LocalProposal`,
   and fixed execution handler reached only through the existing confirmation
   path. Add fixed tools through `LocalTools` and trusted Nix wrappers, never from
   model-selected executable paths. Report unsupported desktops explicitly.
6. For a system change, extend the closed IPC/proposal/state types only as needed.
   Prepare the reviewed change in `system.rs`; keep Apply authorization, stale
   proposal checks, rendering, build verification and activation in the shared
   backend. Preserve old managed-state parsing/migration and rollback behaviour.
   Such changes need transaction and installed-system tests, not just a unit test.
7. Test valid inputs, malformed/hostile values, missing tools, command failures,
   confirmation/cancellation, and any desktop/version fallbacks. Use temporary
   fixtures and mock tools; tests must not modify the developer's real desktop.
8. Update this table, the capability/security documentation and examples. Check
   packaging if adding dependencies: both full and headless source snapshots
   include `peas/`, and the full package carries it into ISO-installed systems.

### Checks

From the repository root, use the development shell for the pinned toolchain
and desktop libraries (`nix develop`). After adding files, make sure they are
tracked before testing a Git-backed flake; Nix otherwise omits untracked files.

```console
cargo fmt --all -- --check
cargo test --locked -p peasy-core -p peasy-engine -p peasy-client -p peasy-system -p peasy-engine-host -- --test-threads=1
cargo build --locked --release -p peasy-engine --target wasm32-unknown-unknown
PEASY_TEST_ENGINE="$PWD/target/wasm32-unknown-unknown/release/peasy_engine.wasm" \
  cargo test --locked -p peasy-engine-host compiled_wasm_preserves_all_pea_decisions -- --ignored
PEASY_TEST_ENGINE="$PWD/target/wasm32-unknown-unknown/release/peasy_engine.wasm" \
  cargo test --locked -p peasy-client setup_selection_retains_the_exact_candidate_for_follow_up_uninstall -- --ignored
nix build .#peasy .#peasy-core
```

The localhost provider tests use mock servers, not real AI credentials. The Wasm
and setup-selection integration tests are explicitly ignored in ordinary local
Cargo runs because they need a built guest; Nix package checks supply that guest
and run them automatically.
For changes touching desktop execution or installation, also follow the
[desktop](../docs/desktop-compatibility.md) and [ISO](../docs/iso.md#verification)
test procedures. A new pea does not automatically require rebuilding every ISO
while iterating, but the release still has its normal acceptance gates.

## Example prompt for adding a pea

This is a contributor prompt, not a capability already implemented by this split:

> Work in the existing Peasy repository. Read `peas/README.md` and add an `audio`
> pea that answers “what is my current output volume?” on PipeWire desktops. Keep
> this addition read-only: use a fixed, trusted `wpctl get-volume
> @DEFAULT_AUDIO_SINK@` call, validate and bound its output, and report a clear
> unsupported/unavailable result when appropriate. Do not add volume changes,
> muting, arbitrary commands, model-chosen paths or new privileges. Put the
> implementation and tests in `peas/audio/`, wire it into the existing closed
> model validation, Wasm decision and client routing, and package its fixed tool
> dependency through Nix. Preserve every existing action, prompt meaning, public
> API, credential boundary, confirmation flow and NixOS transaction/rollback
> behaviour. Add mock-command success/failure/malformed-output tests and extend
> the compatibility corpus for the new action without weakening existing cases.
> Update the pea table and user documentation, run the relevant Rust and Wasm
> checks, and report any checks you could not run. Do not switch my host system,
> contact a real AI provider, commit, tag or publish.
