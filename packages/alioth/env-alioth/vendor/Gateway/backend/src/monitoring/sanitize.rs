use regex::Regex;
use std::sync::LazyLock;

static UUID_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")
        .unwrap()
});
static NUMERIC_ID_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/\d+(/|$)").unwrap());

const NGAC_PREFIXES: &[&str] = &[
    "/ngac/policies",
    "/ngac/hierarchy",
    "/ngac/attributes",
    "/ngac/relations",
];

const API_PATTERNS: &[(&str, &str)] = &[
    ("/api/collection", "__collection__"),
    ("/api/collection/", "__collection__"),
    ("/api/field", "__field__"),
    ("/api/field/", "__field__"),
];

pub fn sanitize_path(path: &str) -> String {
    for prefix in NGAC_PREFIXES {
        if path.starts_with(prefix) {
            return prefix.replace("/", "_").trim_matches('_').to_string();
        }
    }

    for (pattern, replacement) in API_PATTERNS {
        if path.starts_with(pattern) {
            return replacement.to_string();
        }
    }

    let normalized = UUID_REGEX.replace_all(path, "__id__");
    let normalized = NUMERIC_ID_REGEX.replace_all(&normalized, "__id__");

    simplify_path(&normalized)
}

fn simplify_path(path: &str) -> String {
    let parts: Vec<&str> = path
        .split('/')
        .filter(|s| !s.is_empty() && *s != "__id__")
        .collect();

    if parts.is_empty() {
        return "__root__".to_string();
    }

    let simplified = parts.join("_");
    format!("_{}_", simplified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ngac_paths_sanitized() {
        assert_eq!(sanitize_path("/ngac/policies/123"), "ngac_policies");
        assert_eq!(sanitize_path("/ngac/hierarchy/456"), "ngac_hierarchy");
        assert_eq!(sanitize_path("/ngac/attributes/789"), "ngac_attributes");
        assert_eq!(sanitize_path("/ngac/relations/abc"), "ngac_relations");
    }

    #[test]
    fn test_api_paths_normalized() {
        assert_eq!(sanitize_path("/api/collection"), "__collection__");
        assert_eq!(sanitize_path("/api/collection/123"), "__collection__");
        assert_eq!(sanitize_path("/api/field"), "__field__");
    }

    #[test]
    fn test_uuids_replaced() {
        let path = "/api/resource/550e8400-e29b-41d4-a716-446655440000";
        let result = sanitize_path(path);
        assert!(!result.contains("550e8400"));
        assert_eq!(result, "_api_resource_");
    }

    #[test]
    fn test_numeric_ids_replaced() {
        let path = "/api/resource/12345";
        let result = sanitize_path(path);
        assert!(!result.contains("12345"));
        assert!(result.contains("__id__"));
    }

    #[test]
    fn test_simplify_path() {
        assert_eq!(simplify_path("/api/test/__id__"), "_api_test_");
        assert_eq!(simplify_path("/__id__/__id__"), "__root__");
        assert_eq!(simplify_path("/api/test"), "_api_test_");
    }
}
