use std::collections::HashMap;

use crate::Locale;

/// A loaded dictionary: nested JSON object flattened into dotted keys.
#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    messages: HashMap<String, String>,
}

impl Dictionary {
    pub fn from_json(value: serde_json::Value) -> Self {
        let mut messages = HashMap::new();
        if let Some(obj) = value.as_object() {
            Self::flatten_obj("", obj, &mut messages);
        }
        Self { messages }
    }

    pub fn load(json_str: &str) -> Result<Self, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(json_str)?;
        Ok(Self::from_json(value))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.messages.get(key).map(|s| s.as_str())
    }

    pub fn merge(&mut self, other: Dictionary) {
        self.messages.extend(other.messages);
    }

    fn flatten_obj(
        prefix: &str,
        obj: &serde_json::Map<String, serde_json::Value>,
        out: &mut HashMap<String, String>,
    ) {
        for (k, v) in obj {
            let key = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{}.{}", prefix, k)
            };
            match v {
                serde_json::Value::Object(nested) => {
                    Self::flatten_obj(&key, nested, out);
                }
                serde_json::Value::String(s) => {
                    out.insert(key, s.clone());
                }
                serde_json::Value::Number(n) => {
                    out.insert(key, n.to_string());
                }
                _ => {}
            }
        }
    }
}

/// i18n message manager.
#[derive(Debug, Clone, Default)]
pub struct I18nManager {
    dictionaries: HashMap<String, Dictionary>,
    default_locale: String,
}

impl I18nManager {
    pub fn new(default_locale: impl Into<String>) -> Self {
        Self {
            dictionaries: HashMap::new(),
            default_locale: default_locale.into(),
        }
    }

    pub fn load_locale(&mut self, locale: &Locale, dict: Dictionary) {
        self.dictionaries
            .insert(locale.as_str().to_lowercase(), dict);
    }

    pub fn load_locale_from_json(
        &mut self,
        locale: &Locale,
        json: &str,
    ) -> Result<(), serde_json::Error> {
        let dict = Dictionary::load(json)?;
        self.load_locale(locale, dict);
        Ok(())
    }

    /// Get a message by key for the given locale, falling back to the default locale.
    pub fn get(&self, locale: &Locale, key: &str) -> Option<&str> {
        let loc_str = locale.as_str().to_lowercase();
        self.dictionaries
            .get(&loc_str)
            .and_then(|d| d.get(key))
            .or_else(|| {
                self.dictionaries
                    .get(&self.default_locale.to_lowercase())
                    .and_then(|d| d.get(key))
            })
    }

    /// Format a message with simple `{param}` interpolation.
    pub fn format(&self, locale: &Locale, key: &str, params: &HashMap<String, String>) -> String {
        let template = self.get(locale, key).unwrap_or(key);
        let mut result = template.to_string();
        for (k, v) in params {
            result = result.replace(&format!("{{{}}}", k), v);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dictionary_flatten() {
        let json = r#"{"wizard":{"title":"Hello","count":"{n} items"}}"#;
        let dict = Dictionary::load(json).unwrap();
        assert_eq!(dict.get("wizard.title"), Some("Hello"));
        assert_eq!(dict.get("wizard.count"), Some("{n} items"));
    }

    #[test]
    fn test_manager_format() {
        let mut mgr = I18nManager::new("zh-cn");
        mgr.load_locale_from_json(&Locale::new("zh-CN"), r#"{"greeting":"你好，{name}"}"#)
            .unwrap();
        let mut params = HashMap::new();
        params.insert("name".to_string(), "World".to_string());
        assert_eq!(
            mgr.format(&Locale::new("zh-CN"), "greeting", &params),
            "你好，World"
        );
    }
}
