//! 入站 chat 上下文的**类型化信封**（C6 backend）。
//!
//! 取代此前散落在三处的防御式形状发现（`orchestrator.rs` 的 `knowledgeContext`、
//! `adapters/agent_dispatch.rs` 的 `suggestedAgent` 顶层 OR 嵌套探测）——形状发现
//! 是**单一实现**（[`ChatContextEnvelope::from_raw`]），读取方拿到已解析的声明字段。
//!
//! wire 契约（与前端 M53 约定）：`context` 保持既有 `{ pageContext: {...} }` 嵌套；
//! `knowledge_context` / `suggested_agent` 为**顶层兄弟字段**（snake_case）。
//! 旧前端形态（`knowledgeContext` / `suggestedAgent` 直接嵌在 context 内、
//! 或页面对象扁平）保留一版兼容读。

use serde_json::Value;

/// 会话 context 内的**规范键**（存储形态，单一深度）。
pub const KEY_PAGE_CONTEXT: &str = "pageContext";
pub const KEY_KNOWLEDGE_CONTEXT: &str = "knowledgeContext";
pub const KEY_SUGGESTED_AGENT: &str = "suggestedAgent";
/// 宿主表单契约键（change `add-form-contract-seam`）——wire 名 = `pageContext.formContract`，
/// 即**页面对象内**与 `currentData`/`pageState`/`rowSummary` 同层的兄弟键（随 `page_context`
/// 一起进「## 当前页面上下文」prompt 段）。形状/解析口径 = 既有 DTO 元数据通道
/// （`parseDtoMeta`），本信封只做**载体与尺寸判定**，MUST NOT 另建描述语言。
pub const KEY_FORM_CONTRACT: &str = "formContract";

/// 一次 turn 的入站上下文信封。
#[derive(Debug, Clone, Default)]
pub struct ChatContextEnvelope {
    /// 页面/实体上下文对象（「## 当前页面上下文」段的载荷）。
    pub page_context: Option<Value>,
    /// 知识注入结果（「## 知识参考」段的载荷）。
    pub knowledge_context: Option<String>,
    /// Agent 路由提示。
    pub suggested_agent: Option<String>,
    /// 宿主表单契约（`pageContext.formContract` 抽出的**只读视图**，change
    /// `add-form-contract-seam`）——承载仍在 `page_context` 内（同层兄弟键），此处
    /// 抽出供 page_context 组装的**独立尺寸判定**消费。缺失 = 该页无可用契约
    /// （助手校验 MUST fail-closed，MUST NOT 以空契约冒充；见 `form-contract` 能力）。
    pub form_contract: Option<Value>,
}

impl ChatContextEnvelope {
    /// wire 声明的顶层兄弟字段 → 规范化信封（**唯一**形状发现实现）。
    pub fn from_parts(
        context: Option<Value>,
        knowledge_context: Option<String>,
        suggested_agent: Option<String>,
    ) -> Self {
        let mut env = Self::from_raw(context.as_ref());
        if knowledge_context.is_some() {
            env.knowledge_context = knowledge_context;
        }
        if suggested_agent.is_some() {
            env.suggested_agent = suggested_agent;
        }
        env
    }

    /// 从任意历史/当前形态解析（单实现）：
    /// - 页面对象：优先 `pageContext` 嵌套；否则「去掉声明键后的对象」；
    /// - `knowledgeContext` / `suggestedAgent`：顶层（规范）优先，字符串取用；
    ///   顶层缺失时回退到 `pageContext.<key>`——旧前端（M53 过渡期）把页面注册
    ///   字段（含 `suggestedAgent`）嵌在 `pageContext` 内，兼容读保住页面建议路由。
    pub fn from_raw(raw: Option<&Value>) -> Self {
        let Some(obj) = raw.and_then(|v| v.as_object()) else {
            return Self::default();
        };
        let page_context = obj.get(KEY_PAGE_CONTEXT).cloned().or_else(|| {
            let mut m = obj.clone();
            m.remove(KEY_KNOWLEDGE_CONTEXT);
            m.remove(KEY_SUGGESTED_AGENT);
            if m.is_empty() {
                None
            } else {
                Some(Value::Object(m))
            }
        });
        // 顶层兄弟键 = 规范形态（权威）；pageContext 内旧键 = 兼容读（仅在顶层缺失时）。
        // 表单契约（`formContract`）例外：其 wire 名声明在 `pageContext` **内**
        // （与 currentData 同层，随 page_context 进 prompt 段）⇒ 嵌套权威、顶层兜底。
        let form_contract = page_context
            .as_ref()
            .and_then(|pc| pc.get(KEY_FORM_CONTRACT))
            .cloned()
            .or_else(|| obj.get(KEY_FORM_CONTRACT).cloned());
        let nested_str = |key: &str| {
            page_context
                .as_ref()
                .and_then(|pc| pc.get(key))
                .and_then(|v| v.as_str())
        };
        let knowledge_context = obj
            .get(KEY_KNOWLEDGE_CONTEXT)
            .and_then(|v| v.as_str())
            .or_else(|| nested_str(KEY_KNOWLEDGE_CONTEXT))
            .map(String::from);
        let suggested_agent = obj
            .get(KEY_SUGGESTED_AGENT)
            .and_then(|v| v.as_str())
            .or_else(|| nested_str(KEY_SUGGESTED_AGENT))
            .map(String::from);
        Self {
            page_context,
            knowledge_context,
            suggested_agent,
            form_contract,
        }
    }

    /// 会话 context 的**规范存储形态**（单一深度）。四者皆空 → None。
    pub fn to_session_context(&self) -> Option<Value> {
        let mut m = serde_json::Map::new();
        if let Some(pc) = &self.page_context {
            m.insert(KEY_PAGE_CONTEXT.to_string(), pc.clone());
        }
        if let Some(k) = &self.knowledge_context {
            m.insert(KEY_KNOWLEDGE_CONTEXT.to_string(), Value::String(k.clone()));
        }
        if let Some(s) = &self.suggested_agent {
            m.insert(KEY_SUGGESTED_AGENT.to_string(), Value::String(s.clone()));
        }
        // 契约随 `page_context` 承载（规范单点，无需另存）；仅当 page_context 未内嵌
        // 该键（顶层兄弟键兜底读入的形态）时补顶层兄弟键，保证存储往返不丢契约。
        if let Some(fc) = &self.form_contract {
            let nested = self
                .page_context
                .as_ref()
                .and_then(|pc| pc.get(KEY_FORM_CONTRACT))
                .is_some();
            if !nested {
                m.insert(KEY_FORM_CONTRACT.to_string(), fc.clone());
            }
        }
        if m.is_empty() {
            None
        } else {
            Some(Value::Object(m))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_raw_reads_canonical_keys() {
        let raw = json!({
            "pageContext": {"page": "合同列表"},
            "knowledgeContext": "LAB-44 赔偿标准",
            "suggestedAgent": "form_filling"
        });
        let env = ChatContextEnvelope::from_raw(Some(&raw));
        assert_eq!(env.page_context.unwrap()["page"], "合同列表");
        assert_eq!(env.knowledge_context.as_deref(), Some("LAB-44 赔偿标准"));
        assert_eq!(env.suggested_agent.as_deref(), Some("form_filling"));
    }

    #[test]
    fn from_raw_reads_nested_page_registered_hint() {
        // 旧前端形态：页面注册字段（含 suggestedAgent）嵌在 pageContext 内，顶层无兄弟键。
        // 回归：顶层读取恒 None 曾令页面建议路由失效（填单场景被路由到 general）。
        let raw = json!({
            "pageContext": {
                "page": "合同列表",
                "suggestedAgent": "form_filling",
                "knowledgeContext": "NESTED-K"
            }
        });
        let env = ChatContextEnvelope::from_raw(Some(&raw));
        assert_eq!(
            env.suggested_agent.as_deref(),
            Some("form_filling"),
            "pageContext 内 suggestedAgent 必须被兼容读取出"
        );
        assert_eq!(env.knowledge_context.as_deref(), Some("NESTED-K"));
        assert_eq!(env.page_context.unwrap()["page"], "合同列表");
    }

    #[test]
    fn from_raw_top_level_hint_wins_over_nested() {
        // 顶层（规范）与 pageContext 内旧键并存 → 顶层权威。
        let raw = json!({
            "pageContext": {"page": "A", "suggestedAgent": "nested", "knowledgeContext": "nested-K"},
            "suggestedAgent": "top",
            "knowledgeContext": "top-K"
        });
        let env = ChatContextEnvelope::from_raw(Some(&raw));
        assert_eq!(env.suggested_agent.as_deref(), Some("top"));
        assert_eq!(env.knowledge_context.as_deref(), Some("top-K"));
    }

    #[test]
    fn from_raw_tolerates_legacy_flat_page_object() {
        // 旧形态：无 pageContext 包裹，页面对象扁平且与声明键同层
        let raw = json!({
            "page": "运单跟踪",
            "module": "wz",
            "knowledgeContext": "K"
        });
        let env = ChatContextEnvelope::from_raw(Some(&raw));
        let page = env.page_context.unwrap();
        assert_eq!(page["page"], "运单跟踪");
        assert_eq!(page["module"], "wz");
        assert!(page.get("knowledgeContext").is_none(), "声明键不入页面对象");
        assert_eq!(env.knowledge_context.as_deref(), Some("K"));
    }

    #[test]
    fn from_raw_none_and_scalar_are_empty() {
        assert!(ChatContextEnvelope::from_raw(None).page_context.is_none());
        assert!(ChatContextEnvelope::from_raw(Some(&json!("x")))
            .page_context
            .is_none());
    }

    #[test]
    fn from_parts_top_level_siblings_win() {
        // 顶层兄弟字段（wire 声明）与 context 内旧键并存 → 顶层优先
        let ctx = json!({"pageContext": {"page": "A"}, "knowledgeContext": "OLD", "suggestedAgent": "old"});
        let env = ChatContextEnvelope::from_parts(
            Some(ctx),
            Some("NEW".to_string()),
            Some("new".to_string()),
        );
        assert_eq!(env.knowledge_context.as_deref(), Some("NEW"));
        assert_eq!(env.suggested_agent.as_deref(), Some("new"));
        assert_eq!(env.page_context.unwrap()["page"], "A");
    }

    #[test]
    fn to_session_context_roundtrips() {
        let env = ChatContextEnvelope {
            page_context: Some(json!({"page": "P"})),
            knowledge_context: Some("K".to_string()),
            suggested_agent: Some("a".to_string()),
            form_contract: None,
        };
        let stored = env.to_session_context().unwrap();
        let back = ChatContextEnvelope::from_raw(Some(&stored));
        assert_eq!(back.page_context.unwrap()["page"], "P");
        assert_eq!(back.knowledge_context.as_deref(), Some("K"));
        assert_eq!(back.suggested_agent.as_deref(), Some("a"));
    }

    #[test]
    fn to_session_context_empty_is_none() {
        assert!(ChatContextEnvelope::default()
            .to_session_context()
            .is_none());
    }

    #[test]
    fn from_raw_reads_page_context_form_contract() {
        // 契约 wire 名 = `pageContext.formContract`（页面对象内兄弟键）。
        let raw = json!({
            "pageContext": {
                "page": "收据录入",
                "formContract": {"fields": [{"name": "customer"}]}
            }
        });
        let env = ChatContextEnvelope::from_raw(Some(&raw));
        assert_eq!(
            env.form_contract.as_ref().unwrap()["fields"][0]["name"],
            "customer"
        );
        // 契约仍留在 page_context 内（承载单点，供 prompt 段序列化）
        assert_eq!(
            env.page_context.as_ref().unwrap()["formContract"]["fields"][0]["name"],
            "customer"
        );
    }

    #[test]
    fn from_raw_reads_top_level_form_contract_sibling() {
        // 顶层兄弟键兜底读（容错；嵌套缺失时）。
        let raw = json!({
            "pageContext": {"page": "P"},
            "formContract": {"fields": []}
        });
        let env = ChatContextEnvelope::from_raw(Some(&raw));
        assert!(env.form_contract.is_some());
    }

    #[test]
    fn to_session_context_preserves_nested_form_contract() {
        let raw = json!({"pageContext": {"page": "P", "formContract": {"fields": []}}});
        let env = ChatContextEnvelope::from_raw(Some(&raw));
        let stored = env.to_session_context().unwrap();
        // 契约随 pageContext 存储，且 MUST NOT 被顶层复制（单一承载点）
        assert_eq!(stored["pageContext"]["formContract"]["fields"], json!([]));
        assert!(stored.get(KEY_FORM_CONTRACT).is_none());
        let back = ChatContextEnvelope::from_raw(Some(&stored));
        assert!(back.form_contract.is_some());
    }
}
