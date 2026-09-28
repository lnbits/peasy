# Pea contract

A **pea** describes a domain in which the LLM can propose changes through Peasy's
host API. The LLM chooses the configuration; the user approves its application.
The host validates inputs, enforces permissions, executes approved operations and
handles recovery. These rules apply to built-in and downloaded peas.

## Package and host adapter

| Part | Contents | Authority |
| --- | --- | --- |
| Pea package | `pea.json`: domain instructions, capabilities, permissions, version, host API and response schema | Can request existing host operations only |
| Native host adapter | Reviewed Rust source compiled into Peasy | Implements discovery, validation and effects within its owning process |

Downloaded packages are data. They cannot load Rust, Python, shell, Nix modules
or additional Wasm guests. Native adapters require source review and a host
rebuild; Rust module boundaries do not sandbox malicious native code.

The host API and generated schemas define accepted inputs. Package instructions
cannot add operations, widen permissions or override validation. Model responses
and package-selection continuations must retain the originating pea's API limits. See
[package format, compatibility and publication](../docs/pea-packages.md).

## Design a domain, not a recipe

- Expose reusable resources and structured changes. Let the LLM compose them
  from the user's request and discovered machine state.
- Do not dispatch by request keywords or add an installer workflow per application.
  Examples are test cases, not routing rules.
- Prefer a data package when existing operations suffice. Add native code only
  for missing host operations or required validation and lifecycle behaviour.
- Discover missing facts through bounded reads. Report unsupported requirements;
  do not invent operations or bypass the host with commands.
- Every operation must define inputs, preconditions, effects, ownership,
  persistence, review, failure and recovery. Secrets use separate local input.

Current system setup uses an allowlisted option catalogue. A broader structured
NixOS interface is a possible host API extension, not an existing capability.
It must retain review, authorization, ownership and recovery; it should avoid
requiring a separate application recipe. Changing accepted values or permissions
requires an explicit compatibility decision and updated schemas and tests.

## Execution contract

| Stage | Required behaviour |
| --- | --- |
| Interpretation | LLM returns a supported structured action; explanation text is never executed |
| Validation | Host checks types, bounds, permissions and discovered resource identities |
| Review | Show the proposed effects and, for system changes, generated Nix diff |
| System Apply | Daemon authenticates the caller and checks the expiring UID-bound proposal and current state; persistent configuration then builds, verifies and activates, while privileged live operations recheck and apply directly |
| Session Apply | Client executes the reviewed local action under the session's existing permissions |
| Recovery | Report partial failure; preserve unrelated state and use shared recovery handling |

System configuration belongs in `.peasy/peasy-managed.nix` and NixOS generations.
Session operations use their service's persistence and undo semantics; they do not
claim NixOS rollback. Generations do not undo user files, database writes or pairing.
Installing a pea does not approve its later actions. Disabling a pea removes its
instructions, not configuration previously created with them.

Keep provider access, credential guarding, review, authorization, cancellation,
transactions and activation in the shared host. Secrets must not enter model
context, command arguments, logs, system IPC or managed Nix state.

Wasm provides additional containment for pure policy. Keep consistent engine
routing; add policy logic only where needed. Resource checks belong in native
adapters, and system changes require independent daemon validation. No WASI,
host imports, filesystem, network or process access may be added to the guest.

## Native source layout

| File | Owning crate | Responsibility |
| --- | --- | --- |
| `types.rs` | `peasy-core` | Types, validation and declarative rendering |
| `client.rs` | `peasy-client` | Discovery, proposals and local execution |
| `system.rs` | `peasy-system` | Privileged-side checks and system proposals |
| `native.rs` | Shared native host | Bounded resource probes and fixed service adapters |
| `policy.rs` | `peasy-engine` | Pure policy when required |

Files are optional and included through explicit `#[path]` declarations.
Appearance also has desktop detection and adapters. Keep this layout until a
concrete dependency or maintenance problem justifies changing it.

## Domains

| Pea | Operations | State |
| --- | --- | --- |
| [Packages](packages/README.md) | Search, check, install, remove | NixOS |
| [System configuration](system_configuration/README.md) | Compose packages, options and caller access | NixOS; service data separate |
| [AppImages](appimages/README.md) | Discover and install pinned external releases | NixOS |
| [Appearance](appearance/README.md) | List and apply supported theme values | NixOS and live desktop |
| [Networking](networking/README.md) | Inspect, configure and activate profiles | Explicit system or session scope |
| [Wi-Fi](wifi/README.md) | Scan and connect | NetworkManager |
| [Bluetooth](bluetooth/README.md) | Discover, connect and pair | BlueZ |
| [Calendar](calendar/README.md) | Prepare an event for import | Local file and calendar application |
| [Hyprland](hyprland/README.md) | Inspect and change supported live settings | Current compositor session |

The resource peas share the [resource protocol](../docs/resources.md):

| Pea | Operations | State |
| --- | --- | --- |
| [Diagnostics](diagnostics/README.md) | Bounded health and failure inspection | Read-only |
| [Services](services/README.md) | Inspect/control services; declare supported enablement | Live and NixOS |
| [Storage](storage/README.md) | Removable filesystems and UUID mounts | UDisks and NixOS |
| [Nix maintenance](nix_maintenance/README.md) | Generations, collection and optimisation | Nix store and system profile |
| [Users](users/README.md) | Local accounts and caller groups | NixOS; home data separate |
| [Firewall](firewall/README.md) | Inspect policy and manage port/interface contributions | NixOS |
| [Printing](printing/README.md) | Driverless printers, defaults and test page | CUPS |
| [Displays](displays/README.md) | Discover and configure outputs | Desktop session |
| [Audio](audio/README.md) | Devices, defaults, volume and mute | WirePlumber |
| [Power](power/README.md) | Battery, profiles and lid/idle settings | Session and NixOS |

## Adding or extending a pea

1. Write its domain contract: operations, effects, persistence, recovery and limits.
2. If the host already supports it, add instructions and metadata using the
   [package authoring procedure](../docs/pea-packages.md#authoring-and-compatibility).
3. Otherwise add reusable native operations in the owning layers. Update model
   decoding, schemas, engine/client dispatch and daemon IPC only as required.
   Keep fixed tools in trusted packaging and preserve managed-state migrations.
4. Regenerate schema, catalogue and prompt fixtures for intentional changes.
   Preserve existing meanings or version the host API.
5. Test accepted and hostile inputs, permission boundaries, review/cancellation,
   failure and recovery. System changes also need ownership, uninstall, shared
   dependency and generation-reconciliation coverage. Use mocked local services.
6. Update the domain README and relevant capability documentation. Package
   dependencies for both desktop and headless builds.

Preserve regression coverage for Nix escaping, privileged activation, managed-source
reconciliation, UID-bound proposal expiry/replay and package permission enforcement.
New operations must use the reviewed host API and IPC, never an alternate executor.

Use shared cancellable reads and HTTP transport. Do not detach work from its
cancellation scope. Once activation or a local mutation is protected, closing
Peasy is not an undo request.

## Verification

```console
nix develop --command bash scripts/check-rust.sh
nix build .#peasy .#peasy-core
```

The Rust script checks formatting, builds the Wasm guest, runs workspace and
integration tests, checks Clippy, and verifies generated package artifacts.
[Cross-domain fixtures](tests/) cover model decoding and native/Wasm decisions.
Run relevant [desktop](../docs/desktop-compatibility.md) and
[installed-system checks](../docs/iso.md#verification) for affected behaviour;
[release validation](../docs/release-validation.md) defines the release gates.
