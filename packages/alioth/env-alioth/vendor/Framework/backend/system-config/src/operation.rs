//! 系统设置操作形状 + 审计口径（change `delegate-system-config-write-to-service`，候选 C2）。
//!
//! 背景：同一个「配置行」形状此前走在三条写径上，且各有独立实现——
//! REST handler 用 serde DTO，AI 动作层自带一份 `parse_operation`，LLM 工具层再自带一份
//! `validate_system_config_operation`；审计写手更是在 `handlers.rs` 与 Gateway 动作层
//! **逐字重复两份**（仅签名不同）。于是「一行形状」= 三个校验器 + 两个审计写手 +
//! 两处主体解析，改一处即漂移。
//!
//! 本 module 收敛**服务端**一侧：解析 + 形状复校验 + 审计写手唯一实现，由 REST 写径
//! （[`crate::handlers`]）与 AI 动作写径（Gateway `system_config_action`）共同消费。
//!
//! **工具层（ai-agent）保留自己的早失败校验**——它跑在 DB 之前、面向 LLM 修正重试，
//! 与「服务端权威解析」职责不同；但其**操作名集合** MUST 与本模块 [`OPERATION_NAMES`]
//! 一致（门禁 `scripts/check/check-action-catalogue.ts` 断言，MUST NOT 静默漂移）。

use common::audit::{record_audit_event_with_metadata, Decision};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::models::{CreateSystemConfigRequest, UpdateSystemConfigRequest};

/// 操作白名单（唯一真相源：REST 写径、AI 动作写径、门禁同源消费）。
pub const OPERATION_NAMES: &[&str] = &["create_config", "update_config", "delete_config"];

/// 形状校验后的单操作。
#[derive(Debug)]
pub enum ParsedOperation {
    Create(CreateSystemConfigRequest),
    Update {
        id: i64,
        input: UpdateSystemConfigRequest,
    },
    Delete {
        id: i64,
    },
}

/// 权威解析 + 形状复校验（不信任上游；服务端 fail-closed）。
///
/// 错误文本保留可行动语义（`缺少 category` / `MUST 为 JSON 对象` / 白名单提示），
/// 供两条写径共用（此前 Gateway 动作层与工具层各有一份等价文本）。
pub fn parse_operation(op: &Value) -> Result<(String, ParsedOperation), String> {
    let o = op
        .as_object()
        .ok_or_else(|| "operation 必须为 JSON 对象".to_string())?;
    let name = o
        .get("op")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "operation 缺少 op 字段".to_string())?
        .to_string();
    let s = |f: &str| -> Option<String> { o.get(f).and_then(|v| v.as_str()).map(str::to_string) };
    let req_id = |f: &str| -> Result<i64, String> {
        o.get(f)
            .and_then(|v| v.as_i64())
            .ok_or_else(|| format!("{name}: 缺少整数字段 {f}"))
    };
    // object 型可选字段（非对象拒绝：凭证专属键约束）
    let obj = |f: &str| -> Result<Option<Value>, String> {
        match o.get(f) {
            None | Some(Value::Null) => Ok(None),
            Some(v @ Value::Object(_)) => Ok(Some(v.clone())),
            Some(_) => Err(format!("{name}: {f} MUST 为 JSON 对象")),
        }
    };

    let parsed = match name.as_str() {
        "create_config" => ParsedOperation::Create(CreateSystemConfigRequest {
            notice: s("notice"),
            code: s("code"),
            _f_: Some(s("category").ok_or_else(|| format!("{name}: 缺少 category"))?),
            _t_: Some(s("provider").ok_or_else(|| format!("{name}: 缺少 provider"))?),
            comments: s("comments"),
            credentials: obj("credentials")?,
            settings: obj("settings")?,
            enabled: o.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
            is_default: o
                .get("is_default")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            domain_: s("domain_"),
        }),
        "update_config" => ParsedOperation::Update {
            id: req_id("id")?,
            input: UpdateSystemConfigRequest {
                notice: s("notice"),
                code: s("code"),
                _f_: s("category"),
                _t_: s("provider"),
                comments: s("comments"),
                credentials: obj("credentials")?,
                settings: obj("settings")?,
                enabled: o.get("enabled").and_then(|v| v.as_bool()),
                is_default: o.get("is_default").and_then(|v| v.as_bool()),
                domain_: s("domain_"),
            },
        },
        "delete_config" => ParsedOperation::Delete { id: req_id("id")? },
        other => {
            return Err(format!(
                "未知操作 '{other}'（白名单：{}）",
                OPERATION_NAMES.join("/")
            ));
        }
    };
    Ok((name, parsed))
}

/// 变更审计（**唯一实现**）。成功写后调用；失败仅内部记 error 不阻断已成功的写
/// （SECURITY_SPEC §11.2 容忍度）——REST 写径与 AI 动作写径同口径。
pub async fn audit_write(
    pool: &PgPool,
    uid: i64,
    subject: &str,
    operation: &str,
    object_path: &str,
    metadata: Value,
) {
    let _ = record_audit_event_with_metadata(
        pool,
        uid,
        subject,
        object_path,
        operation,
        &Decision::Permit,
        metadata,
    )
    .await;
}

/// `object_path` 构造（唯一实现）：`system_config/{category}/{id}`；
/// delete 无 category 时 `system_config/{id}`。
pub fn object_path(category: Option<&str>, id: i64) -> String {
    match category {
        Some(c) if !c.is_empty() => format!("system_config/{c}/{id}"),
        _ => format!("system_config/{id}"),
    }
}

/// 概要 metadata（唯一实现）：仅 category/code/provider（**MUST NOT 含凭证值**）。
pub fn summary_metadata(
    category: Option<&str>,
    code: Option<&str>,
    provider: Option<&str>,
    extra: Value,
) -> Value {
    json!({
        "category": category,
        "code": code,
        "provider": provider,
        "summary": extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_create_defaults_and_split_channels() {
        let (name, parsed) = parse_operation(&json!({
            "op": "create_config", "category": "llm", "provider": "deepseek",
            "settings": { "model": "deepseek-v4" },
            "credentials": { "api_key": "sk-x" }
        }))
        .unwrap();
        assert_eq!(name, "create_config");
        match parsed {
            ParsedOperation::Create(input) => {
                assert_eq!(input._f_.as_deref(), Some("llm"));
                assert_eq!(input._t_.as_deref(), Some("deepseek"));
                assert!(input.enabled, "缺省 enabled = true");
                assert!(!input.is_default);
                assert!(input.credentials.is_some());
            }
            other => panic!("应为 Create：{other:?}"),
        }
    }

    /// 凭证 MUST 只走专属键：非对象拒绝（errors 可行动）。
    #[test]
    fn non_object_credentials_rejected() {
        let err = parse_operation(&json!({
            "op": "create_config", "category": "llm", "provider": "x",
            "credentials": "sk-leaked-in-settings"
        }))
        .unwrap_err();
        assert!(err.contains("credentials"), "{err}");
    }

    #[test]
    fn unknown_operation_lists_whitelist() {
        let err = parse_operation(&json!({ "op": "nope" })).unwrap_err();
        assert!(
            err.contains("create_config/update_config/delete_config"),
            "{err}"
        );
    }

    #[test]
    fn delete_requires_integer_id() {
        assert!(parse_operation(&json!({ "op": "delete_config" })).is_err());
        assert!(parse_operation(&json!({ "op": "delete_config", "id": 3 })).is_ok());
    }

    #[test]
    fn object_path_shapes() {
        assert_eq!(object_path(Some("llm"), 9), "system_config/llm/9");
        assert_eq!(object_path(None, 9), "system_config/9");
        assert_eq!(object_path(Some(""), 9), "system_config/9");
    }
}
