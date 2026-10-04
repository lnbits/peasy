# Architecture

Peasy separates model interpretation from validated execution. See the
[workflow diagram](workflow-map.md) and [security boundaries](security.md).

| Component | Responsibility |
| --- | --- |
| `peasy-ui`, `peasy`, `peasy-tray` | Unprivileged desktop, CLI and launcher |
| `peasy-client` | Provider requests, bounded chat, task routing and reviewed session actions |
| `peasy-core` | Closed types, validation and deterministic Nix rendering |
| `peasy-engine` / `peasy-engine-host` | Pure Wasm policy with no imports; bounded Wasmtime execution |
| `peasy-system` | Independent authorization, proposals, Nix builds and recovery |
| `peasy-activate.service` | Activate the exact verified generation through a private request |

## Requests

Chat and task routing use the selected OpenAI or local Ollama model. Chat history
is bounded; task prompts load only the selected capability's schema. Scope changes
retain the original request and pea permissions. Unknown actions and fields fail
validation. The model cannot cancel requests or execute reply text.

Ollama disables prompt truncation and context shifting, rejects incomplete output,
and retries explicit context overflow once at a larger bounded context. OpenAI
chat research can use web search; system actions always use the typed host API.

## System changes

1. Resolve packages against the host's effective package set.
2. Create a diff and expiring, single-use proposal bound to caller and base state.
3. Require review and daemon-side Polkit authorization.
4. Journal and write `.peasy/peasy-managed.nix`; build the normal host configuration.
5. Verify reviewed package identities and managed state, then activate.

Host configuration, hardware files and `flake.lock` are preserved. Failed builds
restore the prior managed source. Uncertain activation blocks new mutations until
reviewed recovery. Generation switches reconcile Peasy's managed state; personal
files and service data have separate recovery needs.

Session actions use fixed native adapters and the receiving service's permissions
and persistence. They do not gain generation rollback.

## Extending Peasy

[Peas](../peapod/README.md) group domain instructions and native adapters.
Downloaded peas are data packages composing existing operations; new execution
capabilities require a host update. Keep provider access, authorization and
transactions shared.

[Resource protocol](resources.md) · [Pea packages](pea-packages.md) ·
[Chat](chat.md) · [Release checks](release-validation.md)
