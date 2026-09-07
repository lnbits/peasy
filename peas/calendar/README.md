# Calendar pea

Validate event data, create a private iCalendar file and hand it to the user's default calendar application for review/import.

Example requests: “Set a meeting for 10am tomorrow”.

`types.rs` owns title/date validation and limits. `client.rs` owns current-time context, proposal creation, UTF-8-safe iCalendar rendering, private-file handling and the default-application call.

No calendar account or API credentials are added. A successful handoff does not mean an event was saved by the receiving application. Missing handlers still leave the file available for manual import.

See the [peas contributor guide](../README.md) for shared wiring, tests and the
example prompt for adding an ability. Existing implementation tests are retained;
cross-pea wire/policy fixtures live in [tests/](../tests/).
