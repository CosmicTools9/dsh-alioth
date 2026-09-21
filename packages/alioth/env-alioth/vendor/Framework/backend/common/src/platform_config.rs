//! 平台运行期配置读取 — `isahl.zc_id_prot-env_config`
//!
//! 运行期布尔开关的**单一真相源**读取器。首个消费方 = 自动审批通过开关
//! （[`AUTO_APPROVE_CODE`]）。
//!
//! 为什么放 common：开关同时被 SSO（实名链分支）与 Gateway（审批自动通过 handler）消费，
//! 两容器都依赖 `common` 但不共享 `system-config` crate（SSO 未依赖它）——读取器下沉到
//! 共同依赖，避免两处内联 SQL 形成第二真相源。
//!
//! 语义（fail-closed）：行缺失 / 行软删 / `settings` 无 `enabled` / 值非法 / 查询失败
//! ⇒ `false`。任何情况下 MUST NOT 以「默认开启」兜底——自动审批通过是权限相关开关，
//! 读不到即视为关闭。

use sqlx::PgPool;

/// 自动审批通过开关的配置 code（`zc_id_prot-env_config.code`）。
/// 模型级种子供给：`Framework/seed/seed-auth-approval-flows.sql`，默认 `enabled=false`。
pub const AUTO_APPROVE_CODE: &str = "approval:auto-approve";

/// 读取布尔开关（`settings->>'enabled'`），fail-closed。
///
/// 查询失败不返回 Err——调用方（审批链/实名链）不因配置读取问题改变主干行为，
/// 只降级为「开关关闭」，并留下 warn 供排查。
pub async fn is_enabled(pool: &PgPool, code: &str) -> bool {
    match fetch_enabled(pool, code).await {
        Ok(v) => v.unwrap_or(false),
        Err(e) => {
            crate::telemetry::warn!("平台配置读取失败（code={}）：{}——按关闭处理", code, e);
            false
        }
    }
}

/// 读取原始值：`Ok(None)` = 行缺失/软删/无 `enabled` 键。
async fn fetch_enabled(pool: &PgPool, code: &str) -> Result<Option<bool>, sqlx::Error> {
    let raw: Option<Option<String>> = sqlx::query_scalar(
        r#"SELECT settings->>'enabled'
             FROM isahl."zc_id_prot-env_config"
            WHERE code = $1 AND deleted_at IS NULL
            ORDER BY id DESC
            LIMIT 1"#,
    )
    .bind(code)
    .fetch_optional(pool)
    .await?;
    Ok(raw.flatten().map(|v| parse_enabled(&v)))
}

/// JSONB 文本 → 布尔。仅显式真值视为开启，其余（含 `"false"`/空串/`"0"`）为关闭。
fn parse_enabled(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "true" | "t" | "1" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::parse_enabled;

    #[test]
    fn enabled_true_values() {
        for v in ["true", "TRUE", " true ", "t", "1", "yes", "on"] {
            assert!(parse_enabled(v), "{v} 应判为开启");
        }
    }

    #[test]
    fn enabled_false_values() {
        for v in ["false", "FALSE", "0", "no", "off", "", "  ", "null", "2"] {
            assert!(!parse_enabled(v), "{v} 应判为关闭");
        }
    }
}
