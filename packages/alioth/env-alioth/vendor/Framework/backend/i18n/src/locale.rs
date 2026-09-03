use std::fmt;

/// Supported locale representation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Locale(pub String);

impl Locale {
    pub const ZH_CN: &'static str = "zh-CN";
    pub const EN: &'static str = "en";

    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self(Self::ZH_CN.to_string())
    }
}

/// Parse an Accept-Language header into a list of (locale, quality) pairs,
/// sorted by descending quality.
pub fn parse_accept_language(header: &str) -> Vec<(Locale, f32)> {
    let mut locales = Vec::new();
    for part in header.split(',') {
        let mut it = part.split(';');
        let lang = it.next().unwrap_or("").trim();
        let mut q = 1.0f32;
        for param in it {
            let param = param.trim();
            if let Some(val) = param.strip_prefix("q=") {
                if let Ok(v) = val.parse::<f32>() {
                    q = v.clamp(0.0, 1.0);
                }
            }
        }
        if !lang.is_empty() {
            locales.push((Locale::new(lang), q));
        }
    }
    locales.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    locales
}

/// Resolve the best matching locale from an Accept-Language header
/// against a list of supported locales.
pub fn resolve_locale(header: &str, supported: &[&str], default: &str) -> Locale {
    let parsed = parse_accept_language(header);
    for (loc, _q) in parsed {
        let loc_str = loc.as_str();
        if supported.iter().any(|&s| s.eq_ignore_ascii_case(loc_str)) {
            return loc;
        }
        // Fallback: match primary language tag (e.g., "en-US" -> "en")
        if let Some(primary) = loc_str.split('-').next() {
            if let Some(matched) = supported
                .iter()
                .find(|&&s| s.eq_ignore_ascii_case(primary) || s.split('-').next() == Some(primary))
            {
                return Locale::new(*matched);
            }
        }
    }
    Locale::new(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_accept_language() {
        let result = parse_accept_language("zh-CN,zh;q=0.9,en;q=0.8");
        assert_eq!(result[0].0.as_str(), "zh-CN");
        assert!((result[0].1 - 1.0).abs() < f32::EPSILON);
        assert_eq!(result[1].0.as_str(), "zh");
        assert!((result[1].1 - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_resolve_locale() {
        let supported = &["zh-CN", "en"];
        assert_eq!(
            resolve_locale("en-US,en;q=0.9", supported, "zh-CN").as_str(),
            "en"
        );
        assert_eq!(
            resolve_locale("ja-JP", supported, "zh-CN").as_str(),
            "zh-CN"
        );
        assert_eq!(resolve_locale("zh", supported, "en").as_str(), "zh-CN");
    }
}
