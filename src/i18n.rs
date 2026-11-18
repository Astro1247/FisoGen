use std::{collections::BTreeMap, fmt, sync::OnceLock};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Language {
    pub code: &'static str,
    name: &'static str,
}

impl fmt::Display for Language {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        output.write_str(self.name)
    }
}

pub const LANGUAGES: [Language; 3] = [
    Language {
        code: "en",
        name: "English",
    },
    Language {
        code: "uk",
        name: "Українська",
    },
    Language {
        code: "ru",
        name: "Русский",
    },
];

const TRANSLATIONS: [(&str, &str); 2] = [
    ("uk", include_str!("../locales/uk.json")),
    ("ru", include_str!("../locales/ru.json")),
];

type Dictionary = BTreeMap<String, String>;
static DICTIONARIES: OnceLock<BTreeMap<&'static str, Dictionary>> = OnceLock::new();

pub fn language(code: &str) -> Language {
    LANGUAGES
        .into_iter()
        .find(|language| language.code == code)
        .unwrap_or(LANGUAGES[0])
}

pub fn translate(language: Language, key: &'static str) -> &'static str {
    DICTIONARIES
        .get_or_init(|| {
            TRANSLATIONS
                .into_iter()
                .map(|(code, json)| {
                    let dictionary = serde_json::from_str(json).unwrap_or_default();
                    (code, dictionary)
                })
                .collect()
        })
        .get(language.code)
        .and_then(|dictionary| dictionary.get(key))
        .map(String::as_str)
        .unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionaries_include_the_same_keys_and_templates() {
        let mut previous = None;
        for (_, json) in TRANSLATIONS {
            let entries: Dictionary = serde_json::from_str(json).unwrap();
            assert!(entries.contains_key("Create / Edit"));
            assert!(entries.contains_key("Settings"));
            let keys: Vec<_> = entries.keys().cloned().collect();
            if let Some(previous) = &previous {
                assert_eq!(previous, &keys);
            }
            for (key, translated) in entries {
                for field in key
                    .split('{')
                    .skip(1)
                    .filter_map(|part| part.split_once('}'))
                {
                    assert!(translated.contains(&format!("{{{}}}", field.0)));
                }
            }
            previous = Some(keys);
        }
        assert_eq!(translate(language("uk"), "Settings"), "Налаштування");
        assert_eq!(translate(language("ru"), "Settings"), "Настройки");
        assert_eq!(translate(language("en"), "Settings"), "Settings");
    }
}
