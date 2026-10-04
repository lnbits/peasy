# Chat

Ask a question or describe a task. Enter sends; Shift+Enter adds a line.

- **Auto:** chooses chat or a computer task.
- **Ask:** answers without directly starting tasks or opening apps.
- **Task:** goes straight to the existing task planner.
- **Review suggested task:** reviews a proposed action before applying it.

Follow-ups use recent context, including reopened chats. Peasy makes reasonable
assumptions for ordinary questions and asks when a missing detail matters.
Task validation and approval still apply.

## Models and attachments

Peasy uses your selected model throughout. Images need a vision-capable model;
PDFs need a compatible OpenAI model. Web research requires OpenAI. Image generation
is currently unavailable.

Attach up to four files, 8 MiB each and 16 MiB total. Smaller models fit less
context; older messages may be omitted with a notice. OpenAI receives the included
messages and attachments, so do not attach secrets.

## History

The top-left history button opens your last **50 chats**. Reopen one to continue,
or delete individual chats or all history. New chat saves the current conversation.

History is local, capped at 1 MiB and 32 turns per chat; older chats are removed
automatically. Only the selected chat is loaded. Files and images are not archived;
reattach them when needed. Storage: `~/.local/state/peasy/chats`, or under
`$XDG_STATE_HOME` when set.
