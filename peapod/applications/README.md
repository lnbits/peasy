# Applications pea

Discover and open installed desktop applications in the current user's session.
Requires host API 6; permissions are `applications.read` and `applications.write`.

- Discovery returns desktop-entry IDs and display names from XDG application directories.
- Opening accepts one discovered `.desktop` ID. Paths, commands, arguments and URLs are not model inputs.
- Hidden entries mask lower-priority entries. Generic typo matching preserves ambiguous choices.
- Peasy rechecks the desktop entry before calling the fixed GIO launcher, as the current user, never root.
- An explicit conversational open request with one match opens immediately. Multiple matches require a choice. Requests through the system-task planner retain its review step.
- No Nix configuration is changed. The launched application's normal persistence applies; there is no rollback and cancellation cannot close an already-open application.

Downloaded peas remain data-only. This adapter cannot launch files supplied as chat attachments.
