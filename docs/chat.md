# Conversation and tasks

Peasy accepts questions and computer tasks in one conversation. The input stays
below the replies. Enter sends; Shift+Enter inserts a line. **New conversation**
starts a fresh conversation after saving the current one. **Chat history**, to its
left, opens the last 50 saved conversations. Select one to read it or ask a follow-up.
History also offers individual deletion and a confirmed **Delete all chats** action.

- **Auto** selects an answer, supported computer task, app launch or inspection.
- **Ask** answers without directly launching apps or starting system tasks.
- **Task** uses Peasy's existing typed system-task planner directly, without a chat-routing call. Tray task requests use this path too.
- Suggested changes use **Review suggested task**. Send the suggested request,
  inspect the resulting proposal, then approve it. A chat answer never executes.
- Follow-up questions retain complete recent turns. A notice identifies answers
  for which older turns were omitted to fit the selected model.

Auto and Ask classify the latest message with recent conversation context, including
reopened chats, so follow-up questions can retain research or diagnosis intent.
The latest topic takes precedence. Routing includes text only: at most 2,048 bytes
of recent context for Ollama or 8,192 for OpenAI, including message overhead and
within the overall input budget. Long past messages are marked as excerpts;
attachments are excluded. Answers still receive complete recent turns that fit.

Ordinary answers use the current local date/time and reasonable low-risk assumptions
instead of repeatedly asking for details. Material assumptions are stated, and
missing information still prompts a focused question when it matters for accuracy
or safety. This does not authorize actions or invent computer-change parameters.
If contextual routing selects a task or app launch, a second classification of
the current message alone must support that action. Context-dependent changes
remain suggestions for review. Ask and attachment guards still apply; explicit
Task mode passes the original request directly to the existing planner.

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

The active conversation is bounded to 32 turns and 32 MiB (including attachment
contents, names, suggestions and source links). The history browser loads only
small titles and timestamps; opening a chat loads one transcript. Disk work runs
on one worker with a queue capped at two compact snapshots, keeping disk stalls
off the UI thread and preventing a growing write backlog.

Transcripts are private local files under `$XDG_STATE_HOME/peasy/chats`, or
`~/.local/state/peasy/chats` by default (directory mode `0700`, files `0600`).
Only text, source links, suggestions and attachment names are saved. Attached
files, images and PDFs are not copied into the archive; executable task state is
never archived or replayed. Answers may still quote text from attached files.
Reattach files for follow-ups that need their content. Saved chats keep at most
32 recent turns and 1 MiB per transcript, removing the oldest complete turns if
necessary; reopening a shortened chat displays a notice. Only the latest 50
conversations by last activity are retained, approximately 50 MiB plus small
headers and an atomic-write temporary file. New conversations are saved after
completed answers/tasks and before navigating away. Older chats from before
this feature cannot be recovered. The application opens with a fresh conversation
after restart; saved chats remain available through history.

Files stay in memory, but choosing
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
