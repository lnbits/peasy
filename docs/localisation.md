# Localisation

Peasy selects its desktop language at startup from the user's session locale.
`LC_ALL`, then `LC_MESSAGES`, then `LANG` selects the message locale. `LANGUAGE`
can supply a colon-separated preference list, except in the `C`/`POSIX` locale.
Unsupported languages and missing translations fall back to English. Restart
Peasy after changing the desktop language.

Included: English (`en`), Simplified Chinese (`zh`), Hindi (`hi`), Spanish (`es`),
Arabic (`ar`), French (`fr`), Bengali (`bn`), Portuguese (`pt`), Russian (`ru`),
and Indonesian (`id`). Regional locales use their base language. Arabic uses a
right-to-left layout; package attributes and Nix diffs keep their original order.

Translations cover desktop controls, settings, progress, backup guidance and tray
menus. Both model providers receive a short instruction to use the selected
language for human-facing replies. Model language quality depends on the model.
Native operation details, package metadata, diagnostics and Nix source retain
their original text. Translation never changes actions, permissions or approvals.

## Add a language

1. Copy `crates/peasy-core/src/locales/en.json` to `<language-code>.json` in the
   same directory. Use a lowercase base language code, such as `it`.
2. Set `name` to the language name used in model instructions, and `direction`
   to `ltr` or `rtl`.
3. Translate the values in `messages`. Keep the English keys and all named
   placeholders, such as `{version}`, unchanged. Do not translate identifiers
   or commands embedded in messages.
4. Run `cargo test -p peasy-core i18n::tests` and check the desktop with that
   language. Have a fluent speaker review the wording.

The build automatically embeds every JSON catalogue; no UI registry or new
dependency is needed. Translation is local and requires no model or network.
For new UI copy, use `tr` or `tr_args` and update each catalogue. Pass user data
as placeholder values; never use translations as protocol values or Nix input.
Catalogue tests check matching keys, nonempty text and preserved placeholders.

## Offline errors

An Internet transport failure displays “Sorry, no internet connection” only
when the OS also reports no running, addressed, non-loopback network interface.
This conservative check leaves ambiguous failures unchanged: active interfaces,
VPNs, captive portals, broken DNS and upstream outages may retain the original
error. Authentication, TLS and HTTP server errors are not proof of being offline.
Local Ollama failures are not Internet failures.

Cached and local operations run normally. No connectivity probe is sent to an
external server. “Copy diagnostics” retains the complete original error.
