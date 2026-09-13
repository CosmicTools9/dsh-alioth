//! dict 条目 `notice` 的 locale 覆盖（数据值 i18n，见 DATA_I18N_SPEC）
//!
//! 字典显示名以 `notice`（中文单语言）存储于 DB。多语言切换时由本模块
//! 在 API 装配层按请求 Accept-Language 覆盖：
//!
//! - zh（默认）：原样返回 DB notice，零回归；
//! - en：依次尝试 ① en.json 精确翻译键（`dict.<table>.<code>`）
//!   ② code ASCII 可读化（`BOM-CAT-MANDATORY` → `BOM CAT MANDATORY`，
//!   动态字典行自动获得近似英文显示）③ 原 notice 回退。
//!
//! 翻译键数据：`locales/en.json`（编译期嵌入，公开标准——国家/时区/币种/政体；
//! 其余类目按域渐进补键，缺键走可读化/回退，不报错）。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 编译期嵌入的 en 精确翻译：table → (code → value)
static EN_DICT: OnceLock<HashMap<String, HashMap<String, String>>> = OnceLock::new();

fn en_dict() -> &'static HashMap<String, HashMap<String, String>> {
    EN_DICT.get_or_init(|| {
        let raw: serde_json::Value = serde_json::from_str(include_str!("../locales/en.json"))
            .expect("dict locales/en.json 必须为有效 JSON");
        raw.get("dict")
            .and_then(|d| d.as_object())
            .map(|tables| {
                tables
                    .iter()
                    .map(|(table, codes)| {
                        let map = codes
                            .as_object()
                            .map(|m| {
                                m.iter()
                                    .map(|(k, v)| {
                                        (k.clone(), v.as_str().unwrap_or_default().to_string())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        (table.clone(), map)
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// 请求 Accept-Language 头是否解析为 en（en/en-US/en-GB 等前缀匹配）。
/// 空头/其他语言一律 false（zh 默认）。
pub fn accept_is_en(header: Option<&str>) -> bool {
    let Some(header) = header else { return false };
    header.split(',').any(|part| {
        let lang = part.split(';').next().unwrap_or("").trim();
        lang.eq_ignore_ascii_case("en") || lang.to_ascii_lowercase().starts_with("en-")
    })
}

/// code ASCII 可读化：`-`/`_` → 空格（`BOM-CAT-MANDATORY` → `BOM CAT MANDATORY`）。
/// 仅作用于纯 ASCII code（中文 code 原样返回，走 notice 回退）。
pub fn humanize_code(code: &str) -> Option<String> {
    if code.is_empty() || !code.is_ascii() {
        return None;
    }
    let readable = code.replace(['-', '_'], " ");
    if readable.chars().any(|c| c.is_ascii_alphabetic()) {
        Some(readable)
    } else {
        None // 纯数字/符号 code 无可读化价值
    }
}

/// 按 locale 覆盖 notice。
///
/// - `en == false`：原样返回 notice（zh 默认零回归）；
/// - `en == true`：精确键 → code 可读化 → notice 回退。
pub fn localize_notice(
    table: &str,
    code: Option<&str>,
    notice: Option<&str>,
    en: bool,
) -> Option<String> {
    if !en {
        return notice.map(str::to_string);
    }
    if let Some(code) = code {
        if let Some(exact) = en_dict()
            .get(table)
            .and_then(|m| m.get(code))
            .filter(|v| !v.is_empty())
        {
            return Some(exact.clone());
        }
        if let Some(readable) = humanize_code(code) {
            return Some(readable);
        }
    }
    notice.map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zh_default_returns_notice_unchanged() {
        assert_eq!(
            localize_notice("zc_id_subj-country", Some("CHN"), Some("中国"), false),
            Some("中国".to_string())
        );
        assert_eq!(
            localize_notice("zc_id_cons-timezone-cate", None, Some("阿比让时间"), false),
            Some("阿比让时间".to_string())
        );
    }

    #[test]
    fn en_exact_key_wins() {
        assert_eq!(
            localize_notice("zc_id_subj-country", Some("CHN"), Some("中国"), true),
            Some("China".to_string())
        );
        assert_eq!(
            localize_notice("zc_id_unit-currency", Some("USD"), Some("美元"), true),
            Some("US Dollar".to_string())
        );
        assert_eq!(
            localize_notice(
                "zc_id_cons-timezone-cate",
                Some("Africa/Abidjan"),
                Some("阿比让时间"),
                true
            ),
            Some("Abidjan Time".to_string())
        );
    }

    #[test]
    fn en_dynamic_row_falls_back_to_humanized_code() {
        // 未收精确键的 ASCII code（如 ns seed 新增类目）→ 可读化
        assert_eq!(
            localize_notice(
                "zc_id_cate-file",
                Some("PROOF-SEAL"),
                Some("铅封照片"),
                true
            ),
            Some("PROOF SEAL".to_string())
        );
        // 无 code → notice 原样
        assert_eq!(
            localize_notice("zc_id_subj-country", None, Some("中国"), true),
            Some("中国".to_string())
        );
        // 中文 code → 不可读化 → notice
        assert_eq!(
            localize_notice("zc_id_cate-x", Some("自定义类"), Some("自定义类"), true),
            Some("自定义类".to_string())
        );
    }

    #[test]
    fn accept_header_parsing() {
        assert!(accept_is_en(Some("en")));
        assert!(accept_is_en(Some("en-US,en;q=0.9")));
        assert!(accept_is_en(Some("zh-CN,en;q=0.8"))); // 含 en 即 en 优先
        assert!(!accept_is_en(Some("zh-CN,zh;q=0.9")));
        assert!(!accept_is_en(None));
        assert!(!accept_is_en(Some("fr-FR")));
    }

    #[test]
    fn humanize_rules() {
        assert_eq!(
            humanize_code("BOM-CAT-MANDATORY").as_deref(),
            Some("BOM CAT MANDATORY")
        );
        assert_eq!(
            humanize_code("CERT-ID-CARD").as_deref(),
            Some("CERT ID CARD")
        );
        assert_eq!(humanize_code("中文码"), None);
        assert_eq!(humanize_code("123456"), None);
        assert_eq!(humanize_code(""), None);
    }
}
