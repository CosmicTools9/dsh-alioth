//! NGAC 义务求值（`isahl_auth.ngac_obligation`，NGAC_SPEC §3.3）。
//!
//! ## 语义边界（不得漂移）
//!
//! - **义务不改变判决**：本模块只产出 `obligations_triggered` 字符串列表（EPP `AccessEvent`
//!   字段 → `isahl_audit.audit_events.obligations_triggered` 列）。判决（Permit/Deny）由
//!   PDP 给出，本模块 MUST NOT 改写之；响应动作的**执行**归消费方。
//! - **动态计算走平台唯一表达式引擎**：`runtime_engine::RhaiExpressionEngine`
//!   （2026-09-22 用户裁决：平台所有表达式 MUST 经该引擎，MUST NOT 并存第二套文法）。
//! - **失败封闭**：`conditions` 不满足（§2.4 同一实现 `ngac_decision::evaluate_conditions`）或
//!   `event_pattern` 求值失败 ⇒ 该义务**不触发**；`response_ops` 求值失败 ⇒ 该义务产出空
//!   （仅告警，不阻断决策、不静默改判）。
//! - **序**：按 `priority` 升序（NULL 最后），同优先级按 `id`；输出首现去重。

use std::collections::HashMap;

use ngac_decision::{evaluate_conditions, ConditionContext};
use runtime_engine::RhaiExpressionEngine;
use serde_json::Value;
use sqlx::PgPool;

/// 一次判决事件的义务求值上下文。
pub struct ObligationEvalContext<'a> {
    pub user_id: i64,
    /// 资源标识 `{type}:{id}`（与 PEP 资源标识同构）。
    pub resource: &'a str,
    pub action: &'a str,
    /// `Permit` / `Deny`（`AccessEvent.decision` 的字符串形）。
    pub decision: &'a str,
    /// 主体有效 UA 名集合（conditions v2 `user_attr_in` 与脚本 `ctx` 可见）。
    pub subject_attributes: &'a [String],
    /// 目标 OA 闭包名集合（conditions v2 `object_attr_in` 与脚本 `ctx` 可见）。
    pub object_attributes: &'a [String],
}

/// `ngac_obligation` 取数行（具名结构体：避免元组类型复杂度过高，clippy::type_complexity）。
#[derive(sqlx::FromRow)]
struct ObligationRow {
    o_name: String,
    event_pattern: Option<String>,
    response_ops: Option<Value>,
    conditions: Option<Value>,
}

/// 求值全部 active 义务；返回按序拼接的 `response_ops`（首现去重）。
pub async fn evaluate_obligations(pool: &PgPool, ctx: &ObligationEvalContext<'_>) -> Vec<String> {
    let rows = load_active_rows(pool, "post").await;

    let variables = ctx_variables(ctx);
    let engine = RhaiExpressionEngine::new();
    let mut out: Vec<String> = Vec::new();
    for ObligationRow {
        o_name,
        event_pattern,
        response_ops,
        conditions,
    } in rows
    {
        // §2.4 条件（与 association/prohibition 同一求值实现，失败封闭）
        let cond_ctx = ConditionContext {
            now: chrono::Utc::now(),
            user_ua_names: ctx.subject_attributes.to_vec(),
            oa_closure_names: ctx.object_attributes.to_vec(),
            // 条件含 `{"rhai": …}` 时与真决策面**同源注入**（缺口 A 收口）：此前为 `None`
            // ⇒ 写径接受的 rhai 条件永不满足 = 「写入成功但永不触发」的静默面。
            rhai_eval: Some(ngac_host::rhai_condition_evaluator(
                ctx.user_id,
                ctx.resource.to_string(),
                ctx.action.to_string(),
            )),
        };
        if !evaluate_conditions(&conditions, &cond_ctx) {
            continue;
        }
        // 事件匹配：空/缺省 ⇒ 全匹配；否则按 Rhai 布尔表达式（求值失败 ⇒ 不匹配）
        if let Some(p) = event_pattern
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            match engine.evaluate_bool(p, &variables) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    log::warn!(
                        "ngac obligation '{o_name}': event_pattern 求值失败（按不匹配）：{e}"
                    );
                    continue;
                }
            }
        }
        for op in ops_from_value(&engine, response_ops.as_ref(), &variables, &o_name) {
            if !out.contains(&op) {
                out.push(op);
            }
        }
    }
    out
}

/// 保留算子：`phase='pre'` 义务命中它 ⇒ 判决翻转为 Deny（add-ngac-pre-obligations）。
pub const DENY_OP: &str = "deny";

/// 取某相位的 active 义务行（读失败先自愈一次；仍失败 ⇒ 空，fail-safe）。
async fn load_active_rows(pool: &PgPool, phase: &str) -> Vec<ObligationRow> {
    let sql = "SELECT o_name, event_pattern, response_ops, conditions \
                 FROM isahl_auth.ngac_obligation \
                WHERE is_active = TRUE AND deleted_at IS NULL AND phase = $1 \
                ORDER BY priority ASC NULLS LAST, id ASC";
    match sqlx::query_as(sql).bind(phase).fetch_all(pool).await {
        Ok(rows) => rows,
        Err(e) => {
            // 自愈一次：fresh 库可能尚无该表（`ensure_ngac_extension_tables` 负责建）。
            // 仅在读失败时触发（不进判定热路径的常规分支）。
            crate::ngac::ensure::ensure_ngac_extension_tables(pool).await;
            match sqlx::query_as(sql).bind(phase).fetch_all(pool).await {
                Ok(rows) => rows,
                Err(e2) => {
                    log::warn!(
                        "ngac obligation: 读取 ngac_obligation(phase={phase}) 失败（首次 {e}；自愈后 {e2}），按无义务处理"
                    );
                    Vec::new()
                }
            }
        }
    }
}

/// **pre 相判决参与**（add-ngac-pre-obligations）：对 `phase='pre'` 义务求值；任一匹配
/// （`event_pattern` 真 + `conditions` 满足）的义务其算子含保留算子 [`DENY_OP`] ⇒ `true`
/// （判决翻转为 Deny）。**fail-safe**：读/解析/求值失败 ⇒ 该义务不触发（不返回 true）。
pub async fn evaluate_pre_obligation_denies(
    pool: &PgPool,
    ctx: &ObligationEvalContext<'_>,
) -> bool {
    let rows = load_active_rows(pool, "pre").await;
    if rows.is_empty() {
        return false;
    }
    let variables = ctx_variables(ctx);
    let engine = RhaiExpressionEngine::new();
    for row in rows {
        let cond_ctx = ConditionContext {
            now: chrono::Utc::now(),
            user_ua_names: ctx.subject_attributes.to_vec(),
            oa_closure_names: ctx.object_attributes.to_vec(),
            // 同 `evaluate_obligations`：pre 义务条件亦与真决策同源注入（缺口 A 收口）。
            rhai_eval: Some(ngac_host::rhai_condition_evaluator(
                ctx.user_id,
                ctx.resource.to_string(),
                ctx.action.to_string(),
            )),
        };
        if !evaluate_conditions(&row.conditions, &cond_ctx) {
            continue;
        }
        if let Some(p) = row
            .event_pattern
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            match engine.evaluate_bool(p, &variables) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(e) => {
                    log::warn!(
                        "ngac pre-obligation '{}': event_pattern 求值失败（按不匹配）：{e}",
                        row.o_name
                    );
                    continue;
                }
            }
        }
        if ops_from_value(&engine, row.response_ops.as_ref(), &variables, &row.o_name)
            .iter()
            .any(|op| op == DENY_OP)
        {
            return true;
        }
    }
    false
}

/// 脚本上下文（`RhaiExpressionEngine` D2 键契约：可作标识符的键以裸名注入，整体另注入 `ctx`）。
fn ctx_variables(ctx: &ObligationEvalContext<'_>) -> HashMap<String, Value> {
    let mut m: HashMap<String, Value> = HashMap::new();
    m.insert("user_id".to_string(), Value::from(ctx.user_id));
    m.insert("resource".to_string(), Value::from(ctx.resource));
    m.insert("action".to_string(), Value::from(ctx.action));
    m.insert("decision".to_string(), Value::from(ctx.decision));
    m.insert(
        "permitted".to_string(),
        Value::from(ctx.decision.eq_ignore_ascii_case("permit")),
    );
    m.insert(
        "subject_attributes".to_string(),
        Value::from(ctx.subject_attributes.to_vec()),
    );
    m.insert(
        "object_attributes".to_string(),
        Value::from(ctx.object_attributes.to_vec()),
    );
    m
}

/// `response_ops` 取值（二形态）：
///
/// - **JSON 字符串数组**：静态算子清单，原序返回（元素非字符串者忽略）；
/// - **JSON 字符串**：Rhai 表达式，求值为 `String`（单算子）或字符串数组（算子清单）；
/// - `null` / 缺省 / 其他类型：空。
///
/// 表达式求值失败 ⇒ 该义务产出空并行告警（不阻断决策）。
fn ops_from_value(
    engine: &RhaiExpressionEngine,
    raw: Option<&Value>,
    variables: &HashMap<String, Value>,
    o_name: &str,
) -> Vec<String> {
    match raw {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(script)) => {
            let script = script.trim();
            if script.is_empty() {
                return Vec::new();
            }
            match engine.evaluate(script, variables) {
                Ok(Value::String(s)) if !s.is_empty() => vec![s],
                Ok(Value::Array(items)) => items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
                Ok(_) => Vec::new(),
                Err(e) => {
                    log::warn!("ngac obligation '{o_name}': response_ops 求值失败（产出空）：{e}");
                    Vec::new()
                }
            }
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> ObligationEvalContext<'static> {
        static SUBS: &[String] = &[];
        static OAS: &[String] = &[];
        ObligationEvalContext {
            user_id: 7,
            resource: "engineers:42",
            action: "read",
            decision: "Permit",
            subject_attributes: SUBS,
            object_attributes: OAS,
        }
    }

    #[test]
    fn static_array_is_returned_in_order() {
        let engine = RhaiExpressionEngine::new();
        let vars = ctx_variables(&ctx());
        let ops = ops_from_value(
            &engine,
            Some(&json!(["notify:admin", "audit:log", 42])),
            &vars,
            "ob",
        );
        assert_eq!(
            ops,
            vec!["notify:admin".to_string(), "audit:log".to_string()]
        );
    }

    #[test]
    fn dynamic_script_can_emit_scalar_or_array() {
        let engine = RhaiExpressionEngine::new();
        let vars = ctx_variables(&ctx());
        let scalar = ops_from_value(
            &engine,
            Some(&json!(
                "if permitted { \"audit:log\" } else { \"notify:admin\" }"
            )),
            &vars,
            "ob",
        );
        assert_eq!(scalar, vec!["audit:log".to_string()]);

        let array = ops_from_value(&engine, Some(&json!("[\"a\", \"b\"]")), &vars, "ob");
        assert_eq!(array, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn dynamic_script_failure_yields_empty_and_null_yields_empty() {
        let engine = RhaiExpressionEngine::new();
        let vars = ctx_variables(&ctx());
        assert!(ops_from_value(&engine, Some(&json!("1 +")), &vars, "ob").is_empty());
        assert!(ops_from_value(&engine, None, &vars, "ob").is_empty());
        assert!(ops_from_value(&engine, Some(&Value::Null), &vars, "ob").is_empty());
    }

    #[test]
    fn event_pattern_context_exposes_decision_and_resource() {
        let engine = RhaiExpressionEngine::new();
        let vars = ctx_variables(&ctx());
        assert!(engine
            .evaluate_bool(
                "permitted && action == \"read\" && resource == \"engineers:42\"",
                &vars
            )
            .unwrap_or(false));
        assert!(!engine
            .evaluate_bool("decision == \"Deny\"", &vars)
            .unwrap_or(true));
    }

    // ── DB 集成（`--ignored`；口径见 common::testing）──
    //
    // 夹具自建自清（前缀 `zz-ob-`）。注意：`isahl_auth.ngac_obligation` 的 DDL **不在本仓**
    // （仅由历史迁移/人工建表引入），故本测试在建表缺失时跳过；`evaluate_obligations` 自身
    // 在表缺失时按「无义务」处理（fail-safe，见模块文档）。

    async fn open_test_pool() -> Option<PgPool> {
        let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
        if url.is_none() {
            eprintln!("skipped: DATABASE_URL 未设置");
            return None;
        }
        let pool = common::testing::connect_test_db().await;
        let exists: bool =
            sqlx::query_scalar("SELECT to_regclass('isahl_auth.ngac_obligation') IS NOT NULL")
                .fetch_one(&pool)
                .await
                .unwrap_or(false);
        if !exists {
            eprintln!("skipped: isahl_auth.ngac_obligation 不存在（本仓无其 DDL）");
            return None;
        }
        Some(pool)
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM isahl_auth.ngac_obligation WHERE o_name LIKE 'zz-ob-%'")
            .execute(pool)
            .await
            .expect("cleanup obligations");
    }

    async fn insert(
        pool: &PgPool,
        o_name: &str,
        pattern: Option<&str>,
        ops: Value,
        conditions: Option<Value>,
        is_active: bool,
        priority: i32,
    ) {
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_obligation \
             (o_name, event_pattern, response_ops, conditions, is_active, priority, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())",
        )
        .bind(o_name)
        .bind(pattern)
        .bind(ops)
        .bind(conditions)
        .bind(is_active)
        .bind(priority)
        .execute(pool)
        .await
        .expect("insert obligation fixture");
    }

    /// 插入带相位的义务（add-ngac-pre-obligations）。
    async fn insert_phase(
        pool: &PgPool,
        o_name: &str,
        phase: &str,
        pattern: Option<&str>,
        ops: Value,
    ) {
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_obligation \
             (o_name, phase, event_pattern, response_ops, is_active, priority, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, TRUE, 0, NOW(), NOW())",
        )
        .bind(o_name)
        .bind(phase)
        .bind(pattern)
        .bind(ops)
        .execute(pool)
        .await
        .expect("insert phased obligation fixture");
    }

    #[tokio::test]
    #[ignore = "需 DATABASE_URL 测试库（并存在 ngac_obligation 表 + phase 列）"]
    async fn pre_obligation_deny_op_forces_deny_but_post_does_not() {
        let Some(pool) = open_test_pool().await else {
            return;
        };
        // 先 ensure（幂等；为其追加 phase 列），再判列存在
        crate::ngac::ensure::ensure_ngac_extension_tables(&pool).await;
        // phase 列缺失（存量库未跑 ensure）⇒ 跳过
        let has_phase: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM information_schema.columns \
             WHERE table_schema='isahl_auth' AND table_name='ngac_obligation' AND column_name='phase')",
        )
        .fetch_one(&pool)
        .await
        .unwrap_or(false);
        if !has_phase {
            eprintln!("skipped: ngac_obligation.phase 列缺失（未跑 ensure）");
            return;
        }

        // pre 相：事件命中 + 含保留算子 deny ⇒ 否决
        cleanup(&pool).await;
        insert_phase(
            &pool,
            "zz-ob-pre-deny",
            "pre",
            Some("permitted"),
            json!(["deny"]),
        )
        .await;
        assert!(
            evaluate_pre_obligation_denies(&pool, &ctx()).await,
            "pre 义务含 deny 算子 ⇒ 否决"
        );

        // post 相同算子 MUST NOT 参与 pre 判定
        cleanup(&pool).await;
        insert_phase(
            &pool,
            "zz-ob-post-deny",
            "post",
            Some("permitted"),
            json!(["deny"]),
        )
        .await;
        assert!(
            !evaluate_pre_obligation_denies(&pool, &ctx()).await,
            "post 义务不参与 pre 判定"
        );

        // pre 义务无 deny 算子 ⇒ 不否决
        cleanup(&pool).await;
        insert_phase(
            &pool,
            "zz-ob-pre-audit",
            "pre",
            Some("permitted"),
            json!(["audit:log"]),
        )
        .await;
        assert!(
            !evaluate_pre_obligation_denies(&pool, &ctx()).await,
            "pre 义务无 deny 算子 ⇒ 不否决"
        );
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[ignore = "需 DATABASE_URL 测试库（并存在 ngac_obligation 表）"]
    async fn evaluate_obligations_orders_filters_and_dedupes() {
        let Some(pool) = open_test_pool().await else {
            return;
        };
        cleanup(&pool).await;

        // 1) 命中：事件匹配 + 静态算子数组
        insert(
            &pool,
            "zz-ob-1-hit",
            Some("permitted"),
            json!(["audit:log"]),
            None,
            true,
            1,
        )
        .await;
        // 2) 不命中：事件不匹配（!permitted）
        insert(
            &pool,
            "zz-ob-2-nomatch",
            Some("!permitted"),
            json!(["never"]),
            None,
            true,
            0,
        )
        .await;
        // 3) 不命中：conditions 未满足（时间窗已过；§2.4 失败封闭）
        insert(
            &pool,
            "zz-ob-3-cond",
            None,
            json!(["cond"]),
            Some(json!({"not_after": "2000-01-01T00:00:00Z"})),
            true,
            2,
        )
        .await;
        // 4) 命中：pattern 缺省（全匹配）+ 动态脚本产出数组
        insert(
            &pool,
            "zz-ob-4-dyn",
            None,
            json!("[\"a\", \"b\"]"),
            None,
            true,
            3,
        )
        .await;
        // 5) 命中但重复算子（首现去重）
        insert(
            &pool,
            "zz-ob-5-dup",
            None,
            json!(["audit:log"]),
            None,
            true,
            4,
        )
        .await;
        // 6) 停用：忽略
        insert(
            &pool,
            "zz-ob-6-inactive",
            None,
            json!(["inactive"]),
            None,
            false,
            5,
        )
        .await;
        // 7) 命中：动态脚本按 decision 分支
        insert(
            &pool,
            "zz-ob-7-branch",
            Some("permitted"),
            json!("if permitted { \"notify:admin\" } else { \"never\" }"),
            None,
            true,
            6,
        )
        .await;
        // 8) 命中：`conditions.rhai` 满足（action == read，与 ctx() 同源注入）——缺口 A 回归
        insert(
            &pool,
            "zz-ob-8-rhai-hit",
            None,
            json!(["rhai:hit"]),
            Some(json!({ "rhai": "action == \"read\"" })),
            true,
            7,
        )
        .await;
        // 9) 不命中：`conditions.rhai` 不满足（action 不符）
        insert(
            &pool,
            "zz-ob-9-rhai-miss",
            None,
            json!(["rhai:miss"]),
            Some(json!({ "rhai": "action == \"delete\"" })),
            true,
            8,
        )
        .await;

        let ops = evaluate_obligations(&pool, &ctx()).await;
        assert_eq!(
            ops,
            vec![
                "audit:log".to_string(),
                "a".to_string(),
                "b".to_string(),
                "notify:admin".to_string(),
                "rhai:hit".to_string(),
            ],
            "按 priority 升序拼接、命中过滤（含 rhai 条件同源求值）、首现去重"
        );

        cleanup(&pool).await;
    }
}
