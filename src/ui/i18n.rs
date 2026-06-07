// Lightweight internationalization for the Settings UI.
//
// Rather than a key/lookup table, translations live next to their usage via
// the `Lang::t(en, ru)` helper. This keeps the UI code readable and makes it
// obvious that every string has both variants.

use std::fs;
use std::path::Path;

/// UI language. English is the default; Russian is opt-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Russian,
}

impl Default for Language {
    fn default() -> Self {
        Language::English
    }
}

impl Language {
    /// Pick the English or Russian variant of a string based on this language.
    pub fn t<'a>(&self, en: &'a str, ru: &'a str) -> &'a str {
        match self {
            Language::English => en,
            Language::Russian => ru,
        }
    }

    /// Short code used for persistence (`en` / `ru`).
    pub fn code(&self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Russian => "ru",
        }
    }

    fn from_code(code: &str) -> Language {
        match code.trim().to_lowercase().as_str() {
            "ru" => Language::Russian,
            _ => Language::English,
        }
    }
}

/// File (next to the executable) used to remember the last-chosen language.
const LANG_FILE: &str = "ui_language.txt";

/// Load the saved UI language preference from `base_dir`, defaulting to English.
pub fn load_language(base_dir: &Path) -> Language {
    fs::read_to_string(base_dir.join(LANG_FILE))
        .map(|s| Language::from_code(&s))
        .unwrap_or_default()
}

/// Persist the chosen UI language to `base_dir`. Errors are ignored (best effort).
pub fn save_language(base_dir: &Path, language: Language) {
    let _ = fs::write(base_dir.join(LANG_FILE), language.code());
}
