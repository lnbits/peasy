# Peasy architecture

Start with the [visual workflow and AI access map](workflow-map.md) for a
diagram-led overview of the trust boundaries.

Peasy exposes a deliberately closed set of typed NixOS, GNOME, and Hyprland
capabilities.
It is not a shell, an agent framework, or an arbitrary NixOS configuration
editor. Package and appearance changes use a declarative system-generation
flow. Wi-Fi, Bluetooth, and calendar operations use reviewed per-user desktop
flows because they are live session data rather than NixOS generations.

## Processes

```text
person
  |             constructed, credential-guarded request data
  +-- peasy / peasy-ui ------------------------------+--> OpenAI Responses API
  |                                                   +--> local Ollama /api/chat
  |       |
  |       +-- fixed GitHub API discovery --> public repositories/releases
  |       |
  |       +-- Wasmtime -- peasy-engine.wasm (zero imports)
  |       |
  |       +-- local typed actions --> NetworkManager / BlueZ / default calendar
  |       |                       +--> running Hyprland session via hyprctl
  |       |
  |       +-- typed system JSON over /run/peasy/peasy.sock
  |                         |
  |                    peasy-system
  |                         |
  |          nix search / nix eval / fixed NixOS build
  |
  +-- GNOME/Hyprland mint launcher ---------------------------> peasy-ui
```

`peasy`, `peasy-ui`, and the panel extension are unprivileged session programs.
`peasy-system` is a system service. It has no model-provider client and never
receives an API key, Wi-Fi password, calendar event, or Bluetooth request.

The CLI and GUI share `peasy-client`, which owns request interpretation,
candidate selection, guarded external-release discovery, read-only answers,
local action proposals, and confirmation flow. Nixpkgs search, package
verification, system-state rendering, NixOS building, and activation exist only
in `peasy-system`.

## Desktop and headless packages

The full `peasy` derivation adds the GTK/libadwaita UI, tray helper, desktop
metadata, GNOME Shell extension, and wrappers for desktop integration tools.
The separate `peasy-core` derivation builds and installs only the CLI,
`peasy-system`, and `peasy-engine.wasm`. Its source and build exclude the UI and
tray crates as well as all graphical assets, and its runtime wrapper references
only Nix and coreutils.

`services.peasy.desktop.enable = false` selects `peasy-core`, defaults the tray
off, omits GNOME user units and autostart data, and does not enable
NetworkManager or Bluetooth. Both variants use the identical typed IPC,
zero-import policy engine, proposal validation, system sandbox, build, and
activation paths.

## Model-provider flow

For OpenAI, the user process uses `POST /v1/responses`, `store: false`, no tools,
and a strict JSON Schema response format. For Ollama, it uses the native local
`POST /api/chat` endpoint with `stream: false`, temperature zero, and the same
schema as the `format` value. Peasy discovers installed models through
`GET /api/tags`; only an exact returned model can be selected in the GUI. The
Ollama origin is restricted to localhost or a loopback IP and defaults to
`http://127.0.0.1:11434`.

The constructed boundary includes the credential-guarded request, current
local time, Peasy's canonical generated managed module, at most one recent
validated package, and a locally generated system profile. The profile contains
only the active NixOS release and Nix system, runtime/configured desktop enums,
desktop package version where available, desktop/headless Peasy variant, and a
bounded list of package names evaluated into the active generation. Package
search is a bounded agent loop: each search returns only candidate attributes,
display names, versions, and descriptions to the model so it can select a real
match, search for a better alternative, or request AppImage discovery.
Administrator-authored Nix source and arbitrary option values never cross the
model boundary.

Responses deserialize into a closed Rust enum. Unknown actions or fields are
rejected before they reach the Wasm engine or IPC. The known actions cover
package search/check/install/remove, theme listing/change, Wi-Fi listing/connect,
Bluetooth connect, calendar-event creation, explanation, and cancellation.

Read-only decisions can list the static supported GNOME appearance values, ask
NetworkManager locally for visible SSIDs, or request a real Nixpkgs search. The
Wi-Fi scan itself is never added to a later model request. The recent-package
slot permits a follow-up such as “install it”, but the Wasm engine still accepts
only that exact validated attribute.

The model cannot apply a proposal. Native code independently validates enum
values, lengths, dates, package membership, nearby SSIDs, and discovered
Bluetooth addresses. Hyprland setting names, values, and dispatchers are also
closed enums; arbitrary Lua or hyprctl commands never cross this boundary.

## Wasmtime boundary

`peasy-engine.wasm` is built for `wasm32-unknown-unknown`. It exports a small
typed-value ABI described by `wit/peasy-engine.wit` and imports nothing. The
host creates a Wasmtime linker with no WASI and no host functions. It does not
add filesystem, environment, socket, clock, or process interfaces. The engine
accepts serialized typed data and returns a typed decision; it never sees a
path, command, credential, or IPC connection.

The core-module ABI makes the zero-import property mechanically inspectable.
The WIT contract remains the canonical interface for a future component-model
encoding.

## System IPC

IPC is newline-delimited JSON on a mode-`0660`, `wheel`-owned Unix socket. The
request enum contains only:

- `SearchPackages`
- `GetPackages`
- `GetTheme`
- `ProposeInstall`
- `ProposeAppImageInstall`
- `ProposeRemove`
- `ProposeTheme`
- `Apply`
- `Status`

There is no stringly command, path, networking, Bluetooth, calendar, or
credential method. Apply uses a random, short-lived proposal token bound to the
peer UID that requested it. The pending record contains the exact reviewed
change and base state; stale proposals are rejected. Search and proposal strings
have small limits, and package attributes pass a conservative parser. The daemon
independently checks Polkit administrator authorization for every system apply,
bound to the peer's UID, PID, and process start time. Connections, request rates,
and pending proposals are bounded, and heavy Nix operations are serialized.

## Package lookup

The module passes its evaluated `pkgs.path` to `peasy-system`. Search invokes
the trusted `nix` executable directly with an argv array and consumes `nix
search --json` output. The query is regex-escaped before it becomes an argument.
No shell is involved. Search results are normalized to Nixpkgs attribute paths.

Before a proposal is returned, `peasy-system` evaluates the exact attribute
against the same Nixpkgs source. Install proposals identify a real derivation;
removal proposals identify an attribute present in Peasy's own state.
Verification imports that immutable store source directly rather than
snapshotting it as a path flake. Successful verification is cached for later
apply; search retains Nix's native traversal of the package set.

When a request names a GitHub URL, `owner/repository`, or a repository and
organisation in natural language, the unprivileged client queries it directly
and follows GitHub's canonical repository-rename redirect. Otherwise, when
Nixpkgs has no suitable result or the user selects the explicit external
fallback, it searches GitHub's structured repository and release APIs. It
ignores forks, archived repositories, drafts, prereleases, non-AppImage assets,
incompatible architectures, and assets over 1 GiB. An exact requested version
must match the release tag after an optional leading `v`; `latest` means the
first stable release containing a compatible AppImage.

Discovery results are suggestions, not trusted packages. When an administrator
configures an optional hash allowlist, discovery is limited to its repositories.
Otherwise the user reviews the repository and release. Selecting one first
downloads it through `nix store prefetch-file` to calculate a SHA-256 hash. The
system proposal then contains a closed `AppImagePackage` record: identifier,
display name, `owner/repository`, release tag, version, asset name, fixed GitHub
release URL, hash, architecture, and size. The review shows the publisher,
release, asset, architecture, size, hash, and generated Nix diff before Apply.
The review also shows the download URL. Installation requires administrator
authentication. If an optional hash allowlist is configured, the daemon checks
the exact repository/hash pair both when proposing and when applying. A downloaded
hash is an integrity pin, not proof of publisher authenticity. The default policy
is `null` (review without preapproval); an empty map disables external installs.

## State, diff, and deterministic Nix

`.peasy/peasy-managed.nix`, beside the host configuration, is Peasy's only
durable state. It contains a canonical embedded record with a sorted,
duplicate-free package attribute list, validated pinned external AppImage
records, and optional typed GNOME appearance enums. The same file is the NixOS
module that implements that state. There is no parallel mutable database.

External records render to `fetchurl` plus `appimageTools.wrapType2` and a fixed
desktop entry; no repository script or AppImage is executed during discovery
or evaluation. Appearance is a declarative dconf default. Each generation also
exposes the complete record as `/etc/peasy/state.json` and its appearance subset
as `/etc/peasy/theme.json`. The unprivileged client applies only those closed
appearance values to the live session. A user path unit re-synchronizes them at
login and when switching between generations.

The NixOS module also renders `/etc/peasy/system-profile.json` from evaluated,
typed configuration values. It contains no Nix source or option values: package
derivations are reduced to bounded names, desktops to closed enums, and system
identity to short version/platform tokens. The user client validates and bounds
the file again before adding it to a provider request.

The renderer emits fixed syntax and validated values; it never accepts Nix
source from a user, model, or Wasm guest. It also renders a fixed evaluation
expression containing only administrator-configured absolute paths. Before
confirmation, trusted code renders the current and proposed modules and computes
a compact line diff. The proposal token binds that displayed diff to the base
state and typed change.

Temporary proposals, build links, and activation requests live only below
`/run/peasy` and disappear on reboot.

## NixOS integration and rebuild

The host configuration imports Peasy's module, enables the service, and
optionally imports `.peasy/peasy-managed.nix` when it exists. No host flake is
required. On first service start Peasy creates the empty managed module. At
apply time it atomically writes the reviewed version and evaluates the normal
host configuration using the same Nixpkgs store path that supplied Peasy:

```text
nix build --file \
  /run/peasy/transactions/PROPOSAL/system.nix \
  --out-link /run/peasy/transactions/PROPOSAL/result
```

The small `system.nix` calls Nixpkgs' `nixos/lib/eval-config.nix` with only the
administrator-selected host configuration (by default
`/etc/nixos/configuration.nix`). That configuration imports Peasy's durable
module normally, so manual rebuilds include the same state. Relative imports
retain their normal meaning. There is no channel lookup or expression supplied
through IPC.

For hosts assembled in `flake.nix`, optional `services.peasy.hostFlake` builds
the normal local flake through an explicit `path:` reference so a newly created
managed file is included even before it is committed. Flakes are supported,
not required.

Before activation, Peasy verifies that the result contains exactly the reviewed
state at `/etc/peasy/state.json`. A build failure or missing managed import
restores the preceding managed file. The result must resolve inside
`/nix/store`. Trusted native code writes that
exact result to a private activation request, then asks systemd to run the fixed
`peasy-activate.service`. That helper revalidates the store path, installs it in
the system profile, and calls `switch-to-configuration switch`. Failed builds
are discarded and cannot activate. The helper has the host access required by
NixOS activation but no public IPC or request parameters. `peasy-system` is not
restarted during its own apply, allowing it to return the activation result to
the requesting UI.

Peasy never rewrites `configuration.nix`, hardware configuration, or
`flake.lock`; it owns only `.peasy/peasy-managed.nix`. NixOS generation and boot
rollback behaviour therefore remains standard. Each generation built with
Peasy's rollback reconciliation also restores that managed module from its
validated `/etc/peasy/state.json` when activated. Selecting one of those older
generations consequently rolls back both the live packages and Peasy's durable
desired state, so a later rebuild does not silently reapply the newer package
set. Generations built before this support still roll back the live system but
cannot update the managed source automatically.

## Live desktop actions

Live actions execute only after their typed preview is confirmed:

- Wi-Fi SSIDs are resolved against `nmcli` scan output. Credential-looking inline
  requests are rejected before contacting the model. A password entered in the
  separate local field is written to `nmcli --ask`
  through stdin; it is never an argv value or system IPC field.
- Bluetooth names are resolved against `bluetoothctl devices` output and reduced
  to a validated hardware address before connect/pair is offered.
- Calendar title, local start, and bounded duration are validated, written to a
  mode-`0600` iCalendar file below the private runtime directory, and opened in
  the default calendar for final import.
- Hyprland queries use its JSON IPC output. Changes support fixed scalar
  settings and harmless workspace/window dispatchers. Peasy detects the running
  compositor API and emits either legacy `keyword`/dispatcher argv or current
  `hl.config`/typed Lua dispatcher expressions constructed entirely from native
  enums. It never passes model text as code or exposes exec, plugin, kill, exit,
  output removal, or arbitrary Lua.

These actions do not claim NixOS rollback semantics. NetworkManager, BlueZ, and
the calendar application retain their normal authorization and undo behavior.

## GNOME integration

A native GNOME Shell `PanelMenu` contributes only a compact mint-circle
launcher. Selecting it starts the fixed `peasy-ui` executable. Request entry,
progress, package choices, diff review, confirmation, Wi-Fi password entry, and
provider settings all remain inside the unprivileged GTK4/libadwaita process;
the shell extension neither receives secrets nor participates in proposals.

The extension is installed in the package's standard GNOME Shell extension
directory. The NixOS module seeds it in `enabled-extensions` and installs a fixed
login-time `gnome-extensions enable` action to cover existing profiles.

The settings view exports a private, portable system directory through a native
GTK folder dialog. It contains the administrator's complete configuration tree,
the packaged Peasy source, a wrapper `configuration.nix`, restore instructions,
and the imported `.peasy/peasy-managed.nix` already present in the configuration
tree. That generated module includes Peasy-managed packages, AppImages, and
appearance state. The exporter rewrites Peasy's original checkout/store
path to the bundled `/etc/nixos/peasy` location. Provider credentials and API
keys remain outside the export; hardware configuration is included for faithful
backup but must be regenerated when restoring to different hardware.
