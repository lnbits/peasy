# Calendar pea

Prepare an iCalendar event and hand it to the default calendar application.

- **Inputs:** title, local start date/time and duration of 5–1440 minutes.
- **Checks:** validate title and date; escape and fold iCalendar text.
- **Effects:** after confirmation, write a mode-0600 `.ics` file in a private
  local directory and open the default `text/calendar` handler.
- **Completion:** successful handoff is not proof that the event was saved.
  If opening fails, report the file path for manual import.
- **Persistence:** the receiving calendar controls saved events; no NixOS rollback.
- **Limits:** no account integration or calendar API credentials.

`types.rs` defines validation; `client.rs` handles review, files and opening.
Example: “Set a meeting for 10am tomorrow.” Follow the [pea contract](../README.md).
