# Languages

Peasy follows the OS message locale, with English fallback. Restart after changing
language. Included: English, Chinese, Hindi, Spanish, Arabic, French, Bengali,
Portuguese, Russian and Indonesian. Arabic uses right-to-left layout.

## Add a language

1. Copy `crates/peasy-core/src/locales/en.json` to `<language-code>.json` beside it.
2. Set `name` and `direction` (`ltr` or `rtl`).
3. Translate message values; preserve English keys, placeholders and commands.
4. Run `cargo test -p peasy-core i18n::tests` and have a fluent speaker check the UI.

Catalogues are embedded automatically. New UI text must use `tr`/`tr_args` and be
added to every catalogue. Never translate protocol values or Nix input.

Model replies follow the chosen language; package metadata, diagnostics and source
code may retain their original text.
