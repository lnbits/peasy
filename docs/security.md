# Peasy security model

Peasy treats user text and every byte returned by OpenAI or Ollama as hostile. Model
instructions improve usability; they are not a security boundary.

## Trust boundaries

- A person confirms every changing proposal.
- The selected OpenAI or local Ollama model produces a closed typed value; it has no tools.
- The Wasm engine validates typed decisions with zero ambient capability.
- Unprivileged user programs perform confirmed NetworkManager, BlueZ, and
  calendar handoffs under the user's normal desktop authorization, and perform
  read-only GitHub release discovery for the external-AppImage fallback.
- `peasy-system` performs only package search/verification, deterministic
  managed-module generation, fixed NixOS builds, and fixed activation.
- The Nix daemon, systemd, Nixpkgs store path, and administrator-selected host
  configuration are trusted system components.

## Model threat and data boundary

The model may hallucinate, be prompt-injected, or deliberately return a payload
such as `{"action":"shell","command":"cat /home/user/.ssh/id_ed25519"}`.
Strict JSON Schema and `serde(deny_unknown_fields)` reject undeclared fields.
Unknown action names are rejected. Declared action fields are converted to
bounded Rust strings, enums, dates, or integers before reaching the zero-import
Wasm policy engine.

No action variant represents a command, executable, path, HTTP request,
arbitrary Nix expression, service definition, or general configuration edit.
The model cannot initiate an arbitrary HTTP request: trusted client code alone
constructs fixed `api.github.com` repository/release requests after a package
search intent.

The provider request is assembled from a new JSON value. It may contain:

- the current credential-redacted request;
- current local date/time;
- Peasy's canonical generated managed module, containing only validated package,
  pinned AppImage, and GNOME appearance state;
- a bounded, allowlisted profile generated from the evaluated active NixOS
  configuration: release/platform tokens, closed desktop and Peasy-variant
  enums, and package names only;
- validated package candidates needed for the current request; and
- at most one recent validated package for a follow-up such as “install it”.

It does not contain administrator-authored NixOS source, arbitrary Nix option
values, files, environment variables, process information, logs, nearby Wi-Fi
scan results, Bluetooth addresses, calendar files, or credentials. The only Nix
source supplied is Peasy's own canonical generated module, whose value types are
closed and validated. The OpenAI API key is used only as the HTTPS Authorization
header.
The Ollama request has no key and is restricted to a loopback HTTP origin; the
model cannot choose that origin or any request path.

## Credentials

The OpenAI key is stored per user at `$XDG_CONFIG_HOME/peasy/openai-key` (or
`~/.config/peasy/openai-key`) with mode `0600`; its parent directory is `0700`.
Peasy rejects symlink key files and uses create-new plus atomic rename. The key
is never logged, sent over system IPC, or placed in Nix configuration.
The selected provider and model are stored separately in `provider.json`, also
mode `0600` with symlinks and unknown fields rejected. Replacing or removing a
key is an explicit user action in the settings view.

For Wi-Fi, Peasy recognizes a local `password` marker and removes the following
value before constructing the provider request. If the request contains no recognized
local credential, the review window asks for it separately. The value is passed
to `nmcli --ask` through stdin rather than argv. It is never included in the
preview, panel status, system IPC, or persistent Peasy state.

## Validation and process execution

Package attribute paths are length-bounded dot-separated segments containing
only ASCII letters, digits, `_`, `+`, and `-`. Empty/traversal segments and shell
punctuation are rejected. The daemon still evaluates the attribute against its
pinned Nixpkgs source before proposing it.

SSID, Bluetooth query, calendar title, local timestamp, duration, theme colour,
and colour scheme each have dedicated validators. Wi-Fi names must match a real
NetworkManager scan entry. Bluetooth names must resolve to a discovered device
whose address has the fixed six-byte hexadecimal form. Calendar timestamps must
be real local Gregorian date/time values and durations are 5–1440 minutes.

Native code launches fixed absolute executables with separate argv values. It
never calls `sh -c` or `bash -c`. Nix search text is regex-escaped. Model output
cannot choose an executable or supply an argv sequence.

External AppImage discovery is not a trust decision. Peasy accepts only public
GitHub release assets reached through its fixed API client. It filters out
drafts, prereleases, forks, archived repositories, incompatible architectures,
non-AppImage names, zero-sized assets, and assets above 1 GiB. The typed system
record requires an `https://github.com/OWNER/REPOSITORY/releases/download/...`
URL matching its repository, a closed architecture enum, bounded display
metadata, and a SHA-256 SRI hash. A mutable or replaced release asset therefore
fails the Nix fixed-output hash instead of silently changing. Peasy never runs
repository install scripts or executes an AppImage while discovering it.

## Confirmation boundaries

System IPC is mode `0660` and group-owned by `wheel`. Read-only search has no
system side effect. Propose verifies the change and records a random token,
typed change, complete base state, peer UID, and five-minute expiry. Apply must
present the token from the same UID. Tokens are one-use; a proposal is rejected
if state changed after its diff was displayed.

Only the GTK Apply/Continue button or CLI affirmative response calls an apply
method. Model providers and the Wasm guest cannot call local tools or IPC. Local actions
have a separate typed preview and do not claim NixOS rollback semantics.

## Wasm sandbox

The engine module must have no imports. The host checks that invariant before
instantiation and supplies an empty linker. There are no preopened directories,
WASI contexts, environment variables, sockets, clocks, or process interfaces.
Tests instantiate a hostile module requesting filesystem, environment, socket,
and process imports and prove linking fails.

## System services

The main service runs as root because it stages a NixOS build. Its unit uses
`ProtectHome=tmpfs`, `ProtectSystem=strict`, narrow `ReadWritePaths`,
`PrivateTmp`, `PrivateDevices`, `NoNewPrivileges`, `RestrictSUIDSGID`,
`LockPersonality`, kernel/control-group protections, restricted address
families, IP denial, and a restrictive umask. `/home` and `/root` are hidden.

The service cannot contact a model provider or an Internet host. The separate trusted Nix
daemon may fetch substitutes and a reviewed fixed-output GitHub release asset;
its content must match the proposal's SHA-256 hash. The daemon cannot activate directly under
`ProtectSystem=strict`, so activation uses a fixed one-shot service with no
public IPC and no user-controlled arguments. NixOS activation necessarily needs
full host filesystem, device, user-session, and kernel-setting access, so this
helper is deliberately not filesystem/device sandboxed. It reads a private
root-owned request containing only the already built store result, canonicalizes
and validates that path, sets the system profile, and runs that result's
`switch-to-configuration switch`. The main network-denied service remains
sandboxed throughout package search and build.

The host configuration must be readable for evaluation. By default only
`/etc/nixos` is exposed read-only. Administrator-listed
`configurationReadPaths` become narrow read-only binds when a trusted local
module is below a protected home. Store-backed modules need no exception.

## Filesystem ownership

The root service writes only `/run/peasy` and the dedicated `.peasy` directory
beside the trusted host configuration. Temporary expressions, proposal staging,
and activation requests remain private under `/run`; the only durable file it
can replace is `.peasy/peasy-managed.nix`. It cannot write
`configuration.nix`, `flake.lock`, hardware configuration, or unrelated `/etc`
files. The managed file is replaced atomically, restored after a failed build,
and the built generation must contain its exact validated state before it can
activate.

`/etc/peasy/system-profile.json` is generated by the NixOS module from the
evaluated active configuration. Package derivations are reduced to names; no
option values or source text are copied. The client accepts only a closed JSON
shape, short ASCII version/platform/package tokens, known desktop/variant enums,
at most 256 unique package names, and at most 64 KiB total input. Invalid fields
or oversized data cause the declared profile to be ignored.

The settings export is a local operation and never enters a provider request.
`/etc/peasy/host-configuration-path` contains the trusted absolute module path
selected by the administrator. The UI copies its bounded configuration tree
and the packaged Peasy source into a new mode-`0700` directory, writing regular
files mode `0600`; unsafe absolute symlinks, special files, excessive entries,
and trees over 64 MiB are rejected. The imported
`.peasy/peasy-managed.nix` is already part of that tree; no second state export
is synthesized. API keys and provider settings are excluded. Because
administrator-authored Nix can contain
secrets, users should still review an export before sharing it.

The GNOME extension is only a launcher for the fixed `peasy-ui` executable. It
does not handle request text, proposal data, provider credentials, or Wi-Fi
passwords. `$XDG_RUNTIME_DIR/peasy-user`, mode `0700`, contains only the
non-secret panel-ready marker and mode-`0600` calendar files. The model and Wasm
guest never receive these paths.

Live GNOME appearance synchronization is also unprivileged. The generated
generation contains a non-secret JSON file with only `AccentColor` and
`ColorScheme` enums. `peasy --sync-theme` deserializes it with unknown fields
denied, checks both fixed GNOME keys are writable, and invokes the package's
fixed `gsettings` executable with separate fixed argv fields. A model provider cannot
choose an executable, schema, key, or free-form value. Theme keys are normal
user preferences rather than security lockdown controls.

Hyprland integration is likewise unprivileged and talks only to the invoking
user's compositor socket through that session's `hyprctl`. Model output is
reduced to native enums before review. Peasy can emit only fixed setting paths,
bounded scalar values, and a small harmless dispatcher set; it cannot forward
raw hyprctl arguments or Lua, execute programs, manage plugins, kill processes,
shut down the compositor, or create/remove outputs. Live Hyprland changes are
labelled as session-only and are not represented as NixOS rollback-capable
changes.

## Rollback limits

NixOS generations and rollbacks protect Nixpkgs package, pinned AppImage, and
theme system changes from failed evaluation/build and allow returning to an
earlier generation. Peasy has no persistent state database outside the imported
managed Nix file. Rollbacks do not undo external side effects: Wi-Fi,
Bluetooth, calendar, and live Hyprland changes therefore use their native
controls rather than being described as Nix rollbacks. Peasy never
automatically deletes old NixOS generations.
