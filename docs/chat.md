# Conversation and tasks

Peasy accepts questions and computer tasks in one conversation. The input stays
below the replies. Enter sends; Shift+Enter inserts a line. **New conversation**
clears the in-memory history. Peasy does not save a chat transcript to disk.

- **Auto** selects an answer, supported computer task, app launch or inspection.
- **Ask** answers without directly launching apps or starting system tasks.
- **Task** uses Peasy's existing typed system-task planner directly, without a chat-routing call. Tray task requests use this path too.
- Suggested changes use **Review suggested task**. Send the suggested request,
  inspect the resulting proposal, then approve it. A chat answer never executes.
- Follow-up questions retain complete recent turns. A notice identifies answers
  for which older turns were omitted to fit the selected model.

Auto classifies the latest message with a small prompt. Answers retain conversation
context. Context-dependent changes can be offered as a suggested task for review;
explicit Task mode always passes the original request to the existing planner.

Chat, routing and tasks use the same selected provider and model. Peasy never
loads a second local model or silently switches providers.

## Provider capabilities

| Feature | Local Ollama | OpenAI |
| --- | --- | --- |
| Questions, writing, code and follow-ups | Yes; quality depends on model | Yes |
| UTF-8 text attachments | Yes, within context budget | Yes |
| PNG/JPEG image analysis | Models reporting vision support | Image-capable models |
| PDF attachments | Use a text excerpt instead | Models supporting PDF input |
| Web research with linked sources | Unsupported; choose OpenAI | Native web search tool |
| Image creation and editing | Unavailable: requires a separate image model | Unavailable under the single-model policy |
| Reviewed Peasy tasks and installed app opening | Yes | Yes |

The selected OpenAI model must support the requested tool. Provider account
access and API billing also apply. Peasy reports unsupported requests and limits;
it does not silently switch providers, download a model, or invent tool results.
Image-generation requests explain the limitation and offer help writing a prompt.
OpenAI's image-generation tool invokes a separate image model internally, so Peasy
does not request it under the single-model policy. Links open only when clicked;
replies never load remote images automatically.

## Attachments and limits

Select files explicitly with the attachment button. At most four attachments,
8 MiB each and 16 MiB total; UTF-8 text files are limited to 48,000 bytes and images
to 16 million pixels. Unsupported binary formats and special files are rejected.
PDFs go directly to OpenAI as file input. Attached content is data, never authority
to run a task. Requests containing attachments stay conversational.

Local chat budgets 5,500 bytes for current content and recent conversation, with
additional room reserved for instructions, schema and output. OpenAI uses a
48,000-byte conversation budget. Image/file token costs vary; the provider may
reject a request that passes byte limits. The existing Ollama transport disables
prompt truncation and context shifting, checks server support, and rejects
incomplete output. A larger request must be shortened or moved to a suitable
model; Peasy never silently cuts the current message or attachment.
Chat string lengths are checked by Peasy after decoding; large string limits are
omitted from Ollama's wire schema to avoid excessive sampler grammar expansion.

History is bounded to 32 turns and 32 MiB. Files stay in memory, but choosing
OpenAI sends included messages, attachments and requested diagnostic observations
to that provider. The provider label makes this visible before sending. Ordinary
requests retain Peasy's credential guard; intentionally selected file contents
are sent as supplied, so do not attach secrets you do not want the provider to see.

## Computer diagnosis

A diagnosis request collects a short read-only sample: load, memory and pressure,
the current user's busiest processes, and allowlisted literal settings from
`/etc/nixos/configuration.nix`, `hardware-configuration.nix` and `flake.nix`.
Process arguments, environment values and whole configuration files are not sent.
Unreadable files are reported as unavailable. Imports, module overrides and flakes
elsewhere are not evaluated. Observations are evidence, not a guaranteed cause;
Peasy can suggest a supported task but must still obtain normal review and approval.

## Architecture

The conversation router and answer schema are separate from the system action
schema. Native web search belongs to the selected provider transport;
there is no generic shell, filesystem or network-fetch tool. Task handoffs go
through the existing validator, pea permissions, proposal and activation paths.
App opening uses the [Applications pea](../peapod/applications/README.md).

## Optional live-model smoke check

With an existing local Ollama server and a built Wasm engine, run:

```sh
PEASY_TEST_ENGINE="$PWD/target/wasm32-unknown-unknown/release/peasy_engine.wasm" \
  cargo run -p peasy-client --example chat_smoke -- http://127.0.0.1:11434 qwen3:0.6b
```

This exercises real answers, follow-ups and task routing with one selected model.
It never executes returned tasks or opens applications. Automated CI uses local
HTTP fixtures and a virtual display; it does not download or run model weights.
