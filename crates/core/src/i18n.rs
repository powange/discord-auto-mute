//! User interface language (logs stay in English). Texts are written in English in the
//! code and passed through `tr`; each other language is a catalog in i18n/<code>.json
//! mapping the English text to its translation. Adding a language = adding its file
//! to CATALOGS.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// (code, native name, catalog). English needs no catalog.
const CATALOGS: &[(&str, &str, &str)] = &[("fr", "Français", include_str!("../i18n/fr.json"))];

/// Languages offered in the settings, "auto" first.
pub fn languages() -> Vec<(&'static str, &'static str)> {
    let mut out = vec![("auto", "Automatic"), ("en", "English")];
    out.extend(CATALOGS.iter().map(|(code, name, _)| (*code, *name)));
    out
}

fn catalogs() -> &'static HashMap<&'static str, HashMap<String, String>> {
    static PARSED: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();
    PARSED.get_or_init(|| {
        CATALOGS
            .iter()
            .map(|(code, _, json)| {
                let catalog = serde_json::from_str(json)
                    .unwrap_or_else(|e| panic!("invalid i18n/{code}.json: {e}"));
                (*code, catalog)
            })
            .collect()
    })
}

static CURRENT: RwLock<&'static str> = RwLock::new("en");

/// The supported language closest to a locale such as "fr-FR", "fr_CA.UTF-8" or "de".
pub fn supported(locale: &str) -> &'static str {
    let primary = locale
        .split(['-', '_', '.', '@'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    CATALOGS
        .iter()
        .map(|(code, _, _)| *code)
        .find(|code| *code == primary)
        .unwrap_or("en")
}

pub fn detect() -> &'static str {
    sys_locale::get_locale().map_or("en", |locale| supported(&locale))
}

/// "auto" or a language code; returns the language now in use.
pub fn set_language(code: &str) -> &'static str {
    let lang = if code == "en" {
        "en"
    } else if code == "auto" {
        detect()
    } else {
        supported(code)
    };
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = lang;
    lang
}

pub fn current() -> &'static str {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// The catalog of a language, for the settings window (empty for English).
pub fn catalog(lang: &str) -> HashMap<String, String> {
    catalogs().get(lang).cloned().unwrap_or_default()
}

/// Translates `text` and replaces each `{name}` with its value.
pub fn tr(text: &str, args: &[(&str, &str)]) -> String {
    let translated = catalogs()
        .get(current())
        .and_then(|c| c.get(text))
        .map_or(text, String::as_str);
    let mut out = translated.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_matching() {
        assert_eq!(supported("fr-FR"), "fr");
        assert_eq!(supported("fr_CA.UTF-8"), "fr");
        assert_eq!(supported("FR"), "fr");
        assert_eq!(supported("de-DE"), "en");
        assert_eq!(supported(""), "en");
    }

    #[test]
    fn catalogs_keep_the_placeholders() {
        for (code, catalog) in catalogs() {
            for (english, translated) in catalog {
                let names = |s: &str| {
                    let mut v: Vec<String> = s
                        .split('{')
                        .skip(1)
                        .filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string()))
                        .collect();
                    v.sort();
                    v
                };
                assert_eq!(names(english), names(translated), "{code}: {english}");
            }
        }
    }

    #[test]
    fn translation_and_arguments() {
        set_language("fr");
        assert_eq!(current(), "fr");
        assert_eq!(tr("Paused", &[]), "En pause");
        assert_eq!(tr("Profile: {name}", &[("name", "Rust")]), "Profil : Rust");
        assert_eq!(tr("Not translated", &[]), "Not translated");
        set_language("en");
        assert_eq!(tr("Paused", &[]), "Paused");
    }
}
