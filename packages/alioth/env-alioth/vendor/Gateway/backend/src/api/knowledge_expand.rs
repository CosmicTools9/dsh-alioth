//! 知识检索查询扩展（add-knowledge-query-expansion，阶段一）
//!
//! 动机：`/api/knowledge/search` 原为纯关键词 `ILIKE`，同义表述漏召回
//! （「赔多少」对不上「赔偿标准」）。本模块把用户关键词经 LLM 改写为同义/领域
//! 扩展词，与原词并集检索。
//!
//! 纪律（design D2/D3/D4）：
//! - **有界**：单查询扩展词 ≤`EXPAND_MAX_TERMS`，单请求参与检索词总数 ≤`QUERY_MAX_TERMS`；
//!   扩展词单条 ≤`TERM_MAX_CHARS`、去重、剔除原词与整句长文本。
//! - **缓存**：进程内按 `locale + 原词` 缓存（TTL `CACHE_TTL_SECS`、容量 `CACHE_MAX`），
//!   前端静态域注入器（当前 2 个走 `/api/knowledge/search`）命中同一关键词时只付
//!   一次 LLM 调用；负结果（降级空）不缓存。
//! - **降级**：LLM 未配置/超时/解析失败 → `warn` + 空扩展 ⇒ 原词检索，语义同现状，
//!   MUST NOT 阻断请求；日志不打印关键词原文。
//!
//! 纯函数（`build_expansion_prompt` / `parse_expansion` / `merge_terms`）可单测；
//! LLM 交互只在 `expand_keywords` 内，失败一律降级。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use llm::LlmService;

/// 单查询取前 N 个扩展词。
const EXPAND_MAX_TERMS: usize = 6;
/// 单请求参与检索的词总数（原词 + 扩展词）。
pub const QUERY_MAX_TERMS: usize = 8;
/// 扩展词单条字符上限（防整句/提示词残留进入词表）。
const TERM_MAX_CHARS: usize = 16;
/// 缓存 TTL 与容量。
const CACHE_TTL_SECS: u64 = 300;
const CACHE_MAX: usize = 256;

type CacheMap = HashMap<String, (Instant, Vec<String>)>;

static CACHE: LazyLock<Mutex<CacheMap>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// 扩展提示词（纯函数）：要求输出 JSON 数组、短词、不含原词。
fn build_expansion_prompt(query: &str, locale: &str) -> String {
    format!(
        "你是企业知识库检索助手。请把用户查询改写为用于「关键词 LIKE 检索」的同义/领域词。\n\
         要求：\n\
         1. 只输出 JSON 字符串数组，形如 [\"赔偿\",\"赔付\",\"限额\"]，不要任何解释；\n\
         2. 每个词为 2-8 个字的业务词（如：赔偿标准、运单、破损、结算），不要整句；\n\
         3. 不包含原查询本身；最多 {EXPAND_MAX_TERMS} 个；\n\
         4. 使用与查询相同的语言（locale={locale}）。\n\
         用户查询：{query}"
    )
}

/// 解析 LLM 输出（纯函数）：接受 JSON 数组或「逗号/顿号/换行」分隔的兜底形态。
/// 去重、剔空、剔原词、剔超长，取前 `EXPAND_MAX_TERMS` 个。
fn parse_expansion(raw: &str, original: &str) -> Vec<String> {
    let text = raw.trim();
    let mut terms: Vec<String> = Vec::new();

    if let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(text) {
        for item in items {
            if let Some(s) = item.as_str() {
                terms.push(s.to_string());
            }
        }
    }
    if terms.is_empty() {
        // 兜底：从 ```json 代码块或裸文本中按分隔符切词
        terms = text
            .split(|c: char| {
                c == ',' || c == '，' || c == '、' || c == '\n' || c == ';' || c == '；'
            })
            .map(str::to_string)
            .collect();
    }

    let original = original.trim();
    let mut out: Vec<String> = Vec::new();
    for term in terms {
        let term = term
            .trim()
            .trim_matches(|c| c == '"' || c == '\'' || c == '[' || c == ']' || c == '`')
            .trim()
            .to_string();
        if term.is_empty()
            || term == original
            || term.chars().count() > TERM_MAX_CHARS
            || term.chars().any(char::is_whitespace)
        {
            continue;
        }
        if !out.contains(&term) {
            out.push(term);
        }
        if out.len() >= EXPAND_MAX_TERMS {
            break;
        }
    }
    out
}

/// 合并原词与扩展词（纯函数）：去重后截断到 `QUERY_MAX_TERMS`。
pub fn merge_terms(keywords: &[String], expansions: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for term in keywords.iter().chain(expansions.iter()) {
        let term = term.trim();
        if term.is_empty() {
            continue;
        }
        if !out.iter().any(|t| t == term) {
            out.push(term.to_string());
        }
        if out.len() >= QUERY_MAX_TERMS {
            break;
        }
    }
    out
}

/// 缓存读取（TTL 内命中的扩展词）。
fn cache_get(key: &str) -> Option<Vec<String>> {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let (at, terms) = guard.get(key)?;
    if at.elapsed() > Duration::from_secs(CACHE_TTL_SECS) {
        guard.remove(key);
        return None;
    }
    Some(terms.clone())
}

/// 缓存写入（容量超限 → 清空整表：低价值缓存，无需 LRU 精度）。
fn cache_put(key: String, terms: Vec<String>) {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.len() >= CACHE_MAX {
        guard.clear();
    }
    guard.insert(key, (Instant::now(), terms));
}

/// 查询扩展（唯一 I/O 入口）：命中缓存直接返回；任何失败 → 空扩展（降级）。
pub async fn expand_keywords(llm: &LlmService, query: &str, locale: &str) -> Vec<String> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let cache_key = format!("{locale}\u{1}{query}");
    if let Some(hit) = cache_get(&cache_key) {
        return hit;
    }

    let prompt = build_expansion_prompt(query, locale);
    let call = llm.generate_detailed("", &prompt, Some(0.0), Some(64), None, None, None);
    let terms = match tokio::time::timeout(Duration::from_secs(8), call).await {
        Ok(Ok((text, _usage))) => {
            let parsed = parse_expansion(&text, query);
            if parsed.is_empty() {
                // 失败可见（design D4）：空/不可解析输出也须留痕（不打原文）
                common::telemetry::warn!(
                    "knowledge expand degraded: unparsable/empty output (query_len={}, raw_len={})",
                    query.chars().count(),
                    text.chars().count()
                );
            }
            parsed
        }
        Ok(Err(e)) => {
            common::telemetry::warn!(
                "knowledge expand degraded: llm error (query_len={}): {}",
                query.chars().count(),
                e
            );
            Vec::new()
        }
        Err(_) => {
            common::telemetry::warn!(
                "knowledge expand degraded: llm timeout (query_len={})",
                query.chars().count()
            );
            Vec::new()
        }
    };

    // 负结果不缓存（LLM 恢复后应即时生效）
    if !terms.is_empty() {
        cache_put(cache_key, terms.clone());
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_expansion_reads_json_array() {
        let raw = r#"["赔偿","赔付","限额","赔偿标准"]"#;
        let terms = parse_expansion(raw, "赔多少");
        assert_eq!(terms, vec!["赔偿", "赔付", "限额", "赔偿标准"]);
    }

    #[test]
    fn parse_expansion_falls_back_to_delimited_text() {
        let raw = "赔偿、赔付，限额\n标准";
        let terms = parse_expansion(raw, "赔多少");
        assert_eq!(terms, vec!["赔偿", "赔付", "限额", "标准"]);
    }

    #[test]
    fn parse_expansion_drops_original_blank_and_overlong() {
        let raw = r#"["赔多少", "", "   ", "这是一个非常长的整句话不应该进入词表", "赔偿"]"#;
        let terms = parse_expansion(raw, "赔多少");
        assert_eq!(terms, vec!["赔偿"], "原词/空串/整句须剔除");
    }

    #[test]
    fn parse_expansion_is_bounded_and_deduped() {
        let raw = r#"["a1","a2","a3","a4","a5","a6","a7","a8","a2","a9"]"#;
        let terms = parse_expansion(raw, "x");
        assert_eq!(terms.len(), EXPAND_MAX_TERMS, "扩展词须按上限截断");
        assert_eq!(terms.iter().filter(|t| *t == "a2").count(), 1, "须去重");
    }

    #[test]
    fn merge_terms_dedupes_and_caps() {
        let keywords = vec!["赔多少".to_string()];
        let expansions = vec![
            "赔偿".to_string(),
            "赔多少".to_string(), // 与原词重复
            "赔付".to_string(),
            "限额".to_string(),
            "标准".to_string(),
            "破损".to_string(),
            "运费".to_string(),
            "结算".to_string(),
            "多余".to_string(),
        ];
        let merged = merge_terms(&keywords, &expansions);
        assert_eq!(merged[0], "赔多少", "原词恒在首位");
        assert!(merged.len() <= QUERY_MAX_TERMS, "总词数须受限");
        assert_eq!(merged.iter().filter(|t| *t == "赔多少").count(), 1);
    }

    #[tokio::test]
    async fn expand_keywords_requests_are_cached_and_failures_not_cached() {
        // 判别性：命中缓存时即便 LLM 不可达也返回缓存值（证明短路，不依赖网络）
        let llm = llm::LlmService::new(llm::LlmServiceConfig {
            provider: llm::LlmProvider::DeepSeek,
            api_key: "sk-test-dummy".to_string(),
            model: "dummy".to_string(),
            flash_model: "dummy".to_string(),
            base_url: Some("http://127.0.0.1:9".to_string()),
            timeout_seconds: 1,
            max_retries: 0,
            generation_params: Default::default(),
            roles: Default::default(),
        })
        .expect("dummy llm");

        cache_put(
            "zh-CN\u{1}缓存命中词".to_string(),
            vec!["赔偿".to_string(), "赔付".to_string()],
        );
        assert_eq!(
            expand_keywords(&llm, "缓存命中词", "zh-CN").await,
            vec!["赔偿".to_string(), "赔付".to_string()],
            "命中缓存 MUST 短路（LLM 不可达也返回缓存值）"
        );

        // 负结果不缓存：全新查询 + 不可达 LLM ⇒ 空结果且缓存仍无该键（恢复后可立即重试）
        let fresh = "未被缓存的查询词";
        assert!(expand_keywords(&llm, fresh, "zh-CN").await.is_empty());
        assert!(
            cache_get(&format!("zh-CN\u{1}{fresh}")).is_none(),
            "降级空结果 MUST NOT 入缓存"
        );
    }

    #[test]
    fn cache_roundtrip_and_negative_not_cached() {
        cache_put("zh-CN\u{1}缓存键".to_string(), vec!["赔偿".to_string()]);
        assert_eq!(
            cache_get("zh-CN\u{1}缓存键"),
            Some(vec!["赔偿".to_string()])
        );
        assert_eq!(cache_get("zh-CN\u{1}未写入"), None);
    }

    #[tokio::test]
    async fn expand_keywords_degrades_on_unreachable_llm() {
        // LLM 指向不可达端口 ⇒ 必须降级为空扩展，不 panic、不 Err
        let llm = llm::LlmService::new(llm::LlmServiceConfig {
            provider: llm::LlmProvider::DeepSeek,
            api_key: "sk-test-dummy".to_string(),
            model: "dummy".to_string(),
            flash_model: "dummy".to_string(),
            base_url: Some("http://127.0.0.1:9".to_string()),
            timeout_seconds: 1,
            max_retries: 0,
            generation_params: Default::default(),
            roles: Default::default(),
        })
        .expect("dummy llm");
        let terms = expand_keywords(&llm, "赔多少", "zh-CN").await;
        assert!(terms.is_empty(), "LLM 不可达时须降级为空扩展");
    }
}
