//! Desktop language preferences for presentation only. Protocol identifiers,
//! Nix source, user data and diagnostics must never pass through translation.
use std::{collections::BTreeMap, sync::OnceLock};

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

#[derive(serde::Deserialize)]
struct Catalogue {
    name: String,
    direction: String,
    messages: BTreeMap<String, String>,
}

pub fn language() -> &'static str {
    static LANGUAGE: OnceLock<String> = OnceLock::new();
    LANGUAGE.get_or_init(|| select_language(|key| std::env::var(key).ok()))
}

fn select_language(get: impl Fn(&str) -> Option<String>) -> String {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| get(key).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "en".into());
    if matches!(locale.split('.').next(), Some("C" | "POSIX")) {
        return "en".into();
    }
    let preferences = get("LANGUAGE")
        .filter(|value| !value.is_empty())
        .unwrap_or(locale);
    for locale in preferences.split(':') {
        let base = locale
            .split(['_', '-', '.', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if CATALOGUES.iter().any(|(code, _)| *code == base) {
            return base;
        }
    }
    "en".into()
}

fn catalogue(language: &str) -> Catalogue {
    let json = CATALOGUES
        .iter()
        .find(|(code, _)| *code == language)
        .or_else(|| CATALOGUES.iter().find(|(code, _)| *code == "en"))
        .expect("English catalogue")
        .1;
    serde_json::from_str(json).expect("bundled translation catalogue")
}

fn selected_catalogue() -> &'static Catalogue {
    static SELECTED: OnceLock<Catalogue> = OnceLock::new();
    SELECTED.get_or_init(|| catalogue(language()))
}

pub fn is_rtl() -> bool {
    selected_catalogue().direction == "rtl"
}

/// Only affects human-readable replies; identifiers remain protocol values.
pub fn model_language_instruction() -> String {
    if language() == "en" {
        return String::new();
    }
    format!(
        " Write human-facing messages and clarification questions in {}. Keep all schema keys, enum values and package identifiers unchanged. ",
        selected_catalogue().name
    )
}

pub fn tr(message: &str) -> String {
    selected_catalogue()
        .messages
        .get(message)
        .map(String::as_str)
        .unwrap_or(message)
        .to_owned()
}

/// Replace named placeholders without interpreting or translating their values.
pub fn tr_args(message: &str, values: &[(&str, &str)]) -> String {
    interpolate(&tr(message), values)
}

fn interpolate(template: &str, values: &[(&str, &str)]) -> String {
    let mut result = String::new();
    let mut rest = template;
    while let Some((before, after)) = rest.split_once('{') {
        result.push_str(before);
        let Some((name, tail)) = after.split_once('}') else {
            result.push('{');
            result.push_str(after);
            return result;
        };
        if let Some((_, value)) = values.iter().find(|(key, _)| *key == name) {
            result.push_str(value);
        } else {
            result.push('{');
            result.push_str(name);
            result.push('}');
        }
        rest = tail;
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn select(values: &[(&str, &str)]) -> String {
        select_language(|key| {
            values
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
    }
    #[test]
    fn desktop_language_precedence_and_fallback() {
        assert_eq!(select(&[("LANG", "pt_BR.UTF-8")]), "pt");
        assert_eq!(
            select(&[("LC_MESSAGES", "es_ES.UTF-8"), ("LANG", "fr_FR")]),
            "es"
        );
        assert_eq!(
            select(&[("LC_ALL", "es_ES"), ("LC_MESSAGES", "de_DE")]),
            "es"
        );
        assert_eq!(select(&[("LANGUAGE", "xx:fr:en"), ("LANG", "de_DE")]), "fr");
        assert_eq!(select(&[("LANGUAGE", "fr"), ("LC_ALL", "C.UTF-8")]), "en");
        assert_eq!(select(&[("LANG", "ja_JP")]), "en");
        assert_eq!(select(&[]), "en");
        assert_eq!(
            interpolate(
                "{first} {second}",
                &[("first", "{second}"), ("second", "✓")]
            ),
            "{second} ✓"
        );
    }
    #[test]
    fn catalogues_have_matching_keys_and_placeholders() {
        let reference = catalogue("en").messages;
        for &(language, _) in CATALOGUES {
            let catalog = catalogue(language);
            assert!(matches!(catalog.direction.as_str(), "ltr" | "rtl"));
            assert!(!catalog.name.is_empty());
            let entries = catalog.messages;
            assert_eq!(
                entries.keys().collect::<Vec<_>>(),
                reference.keys().collect::<Vec<_>>()
            );
            for (original, translated) in entries {
                assert!(!translated.is_empty(), "{language}: {original}");
                let placeholders = |s: &str| {
                    let mut names: Vec<_> = s
                        .split('{')
                        .skip(1)
                        .filter_map(|s| s.split_once('}').map(|(name, _)| name.to_owned()))
                        .collect();
                    names.sort();
                    names
                };
                assert_eq!(
                    placeholders(&original),
                    placeholders(&translated),
                    "{language}: {original}"
                );
            }
        }
    }
}
