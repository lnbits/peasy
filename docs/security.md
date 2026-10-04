# Security

Model output and attached content are untrusted. Prompts improve behaviour;
native checks enforce permissions.

## Execution boundaries

- Actions use closed, bounded types. Unknown fields, shell commands and arbitrary
  Nix expressions are rejected. Native adapters use fixed executables and arguments.
- System changes require a reviewed, expiring, single-use proposal tied to the
  caller and current state, plus daemon-side Polkit administrator authorization.
- Packages are verified against the host package set. Generated Nix escapes
  interpolation; the built generation must match the reviewed state.
- Wasm policy has zero imports: no filesystem, network, processes or WASI.
  Its memory, fuel and output are bounded.
- The root daemon has restricted filesystem/device access and no Internet access.
  Separate fixed helpers handle source verification, limited inspection and
  activation. The activation helper necessarily has host privileges.
- Explicit app-opening requests with one discovered match may launch immediately.
  Ambiguous matches require selection; this grants no system-change permission.

Nix, Nixpkgs, administrator configuration, native Peasy code, systemd and Polkit
remain trusted. Installed applications run under their normal permissions, outside
Peasy's policy sandbox.

## Data and credentials

OpenAI receives included requests, recent chat, attachments and task-specific
context. Ollama uses a loopback endpoint with proxies and redirects disabled.
Chat research can use OpenAI web search. Requested diagnosis provides bounded
read-only observations and allowlisted configuration excerpts, not arbitrary
filesystem access. Network context may reveal SSIDs, addresses and routes.

API keys stay in private user files and authenticate provider requests; they are
not prompt text or system IPC. Administrator passwords go through Polkit. Wi-Fi
passwords belong in the separate local field and reach NetworkManager via stdin.
Do not paste secrets into chat: detection cannot catch every secret. Root or a
compromised same-user process can access user data.

[Chat history](chat.md#history) is private local text. [Backups](backups.md) may
contain inline secrets in archived host source; review before sharing.

## Sources and permissions

AppImage review shows repository, release, URL and hash. Hashes pin bytes, not
publisher trust or software safety. `services.peasy.appImages.trustedHashes` is
`null` for source review, `{ }` to block new installs, or an exact repository/hash
allowlist. Removal remains possible after approval is withdrawn.

[Downloaded peas](pea-packages.md) require official source verification, pinned
hashes, compatible schemas and allowed permissions. They cannot load executable
code. Installing a pea does not approve its later changes.

## Failure and recovery

Closing Peasy cancels pending requests and pre-activation builds. Activation and
already-started mutations finish; cancellation cannot undo effects. Provider-side
computation or billing may continue after a disconnected request.

Failed builds restore managed configuration. Activation can partially apply;
use **System status and recovery** or `peasy --recover`. Rollback does not restore
personal files, database writes or live connections. Peasy preserves concurrent
administrator edits instead of silently overwriting them.

Read [resource limits and destructive operations](resources.md) before extending
adapters. Run the [release checks](release-validation.md); automated checks do not
replace real hardware and interactive authentication tests.
