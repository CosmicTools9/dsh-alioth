use crate::ngac_org::{COGNITION_CTE, DELEGATED_CTE};
use crate::AliothError;
use ngac_host::ADMIN_UA_NAME;
use sqlx::{AssertSqlSafe, PgPool};

// 判定链 PIP 段单源：`crate::ngac_org::{COGNITION_CTE, DELEGATED_CTE}`（NGAC_SPEC §2.2.3/§2.2.4
// 消费同源义务）拼装为 DB 函数 `isahl_auth.effective_ua_ids`（本模块，见 `effective_ua_ids_fn_ddl`）；
// 本模块其余 SQL 消费该函数，禁止复制闭包/时效 SQL。`$1` = fk_user（DDL 内替换为 `p_uid`）。

/// 有效 UA 集函数的 SQL 面形式（**判定链 PIP 段唯一实现**：DB 函数 `isahl_auth.effective_ua_ids(bigint)`）。
///
/// 语义（NGAC_SPEC §2.2.3 认知派生 + §2.2.4 委托派生 + §6.2 闭包）= 直派（`deleted_at IS NULL`
/// 且 `expires_at` 未过期）∪ 认知派生 ∪ 委托派生 ∪ 祖先闭包（depth ≤ 10，仅未软删节点）。
/// Rust 面唯一实现 = `SSO/ngac/pip.rs::get_inherited_user_attributes`（取结构化 UA 行供 PDP），
/// 二者同源于 `ngac_org` 的 CTE 常量。消费方 MUST 调用本函数（或经 Rust 唯一实现），MUST NOT 手写闭包/时效。
pub const EFFECTIVE_UA_IDS_FN: &str = "isahl_auth.effective_ua_ids";

/// 生成 `isahl_auth.condition_ts(text)` 的 `CREATE OR REPLACE FUNCTION` DDL（幂等）。
///
/// **NGAC 条件时间解析的安全唯一实现**：非法/空文本 ⇒ `NULL`（调用方据此**失败封闭**：
/// 条件视为不满足）。`p_text::timestamptz` 在异常时返回 NULL，其行为与
/// `ngac_decision::evaluate_conditions` 的 `parse_iso8601` 一致（解析失败 ⇒ 不满足）。
pub const CONDITION_TS_FN: &str = "isahl_auth.condition_ts";

pub fn condition_ts_fn_ddl() -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION {CONDITION_TS_FN}(p_text text)
           RETURNS timestamptz
           LANGUAGE plpgsql IMMUTABLE AS $fn$
           BEGIN
             IF p_text IS NULL OR btrim(p_text) = '' THEN
               RETURN NULL;
             END IF;
             RETURN p_text::timestamptz;
           EXCEPTION WHEN others THEN
             RETURN NULL;
           END
           $fn$"#,
    )
}

/// 生成 `isahl_auth.effective_ua_ids(bigint)` 的 `CREATE OR REPLACE FUNCTION` DDL（幂等；CTE 常量单源）。
///
/// **指派级条件（`ngac_user_rr_attribute.conditions`）接线（2026-10-10 `wire-ngac-zero-consumer-faces`）**：
/// 解析点无资源/动作上下文 ⇒ 仅 §2.4 的**时间窗**（`not_before` / `not_after`）可求值，其余键
/// （`rhai` / `user_attr_in` / `object_attr_in`）在该点**不可求值** ⇒ 失败封闭（该指派不生效 ⇒
/// 该 UA 不进入有效集）；写径按同键集校验（400，`SSO/ngac/pip.rs`）⇒ 两者合并即「键集契约」。
/// 时间语义与 `ngac_decision::evaluate_conditions` 逐条一致（`not_before`：`now >= nb`；
/// `not_after`：`now <= na`；值非法 ⇒ 不满足）。
pub fn effective_ua_ids_fn_ddl() -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION isahl_auth.effective_ua_ids(p_uid bigint)
           RETURNS TABLE(ua_id bigint)
           LANGUAGE sql STABLE AS $fn$
             WITH RECURSIVE {cognition},
             {delegated},
             ua_ids AS (
               SELECT ur.fk_user_attribute AS ua_id, 0 AS depth
               FROM isahl_auth.ngac_user_rr_attribute ur
               WHERE ur.fk_user = p_uid
                 AND ur.deleted_at IS NULL
                 AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
                 AND (
                      COALESCE(ur.conditions, '{{}}'::jsonb) = '{{}}'::jsonb
                   OR (
                        (NOT ur.conditions ? 'not_before'
                         OR (isahl_auth.condition_ts(ur.conditions->>'not_before') IS NOT NULL
                             AND isahl_auth.condition_ts(ur.conditions->>'not_before') <= NOW()))
                    AND (NOT ur.conditions ? 'not_after'
                         OR (isahl_auth.condition_ts(ur.conditions->>'not_after') IS NOT NULL
                             AND isahl_auth.condition_ts(ur.conditions->>'not_after') >= NOW()))
                    AND NOT (ur.conditions ?| ARRAY['rhai','user_attr_in','object_attr_in'])
                      )
                 )
               UNION
               SELECT ua.id, 0
               FROM cognition_ua_names cn
               INNER JOIN isahl_auth.ngac_user_attribute ua
                 ON ua.o_name = cn.o_name AND ua.deleted_at IS NULL
               UNION
               SELECT ua.id, 0
               FROM delegated_ua du
               INNER JOIN isahl_auth.ngac_user_attribute ua
                 ON ua.id = du.id AND ua.deleted_at IS NULL
               UNION ALL
               SELECT unnest(ua.ancestor_ids)::BIGINT AS ua_id, ids.depth + 1
               FROM isahl_auth.ngac_user_attribute ua
               INNER JOIN ua_ids ids ON ua.id = ids.ua_id
               WHERE ids.depth < 10 AND ua.deleted_at IS NULL
             )
             SELECT DISTINCT ua_id FROM ua_ids
           $fn$"#,
        cognition = COGNITION_CTE.replace("$1", "p_uid"),
        delegated = DELEGATED_CTE.replace("$1", "p_uid"),
    )
}

/// admin 判据（**判定链 handler 门 / PDP 豁免的 admin 输入**）：DB 函数 `isahl_auth.is_admin(bigint)`。
///
/// 实现 MUST 建立在 [`EFFECTIVE_UA_IDS_FN`] 之上（同一条链，不重复闭包/时效逻辑）。
/// 为什么是 DB 函数而不是 Rust 侧 SQL 片段：调用方（跨 SSO/Gateway 的内嵌查询）需要
/// 「某用户是 admin」这一**字面**谓词；返回片段再由调用方 `format!` 拼接会被
/// `check-dynamic-table-name.ts` 判为动态列位插值，且违反「判定与执行共用同一实现」。
pub const ADMIN_PREDICATE_FN: &str = "isahl_auth.is_admin";

/// 生成 `isahl_auth.is_admin(bigint)` 的 `CREATE OR REPLACE FUNCTION` DDL（幂等；基于有效 UA 集函数）。
///
/// 注意：函数体 SQL 里 MUST 字面书写 `isahl_auth.effective_ua_ids`（MUST NOT 用 `format!` 插值——
/// 表/函数位插值会被 `check-dynamic-table-name.ts` 判为动态分派）；与 [`EFFECTIVE_UA_IDS_FN`]
/// 的一致性由本文件内相邻声明 + 单测保证。
pub fn admin_predicate_fn_ddl() -> String {
    format!(
        r#"CREATE OR REPLACE FUNCTION isahl_auth.is_admin(p_uid bigint) RETURNS boolean
           LANGUAGE sql STABLE AS $fn$
             SELECT EXISTS (
               SELECT 1
               FROM isahl_auth.effective_ua_ids(p_uid) e
               INNER JOIN isahl_auth.ngac_user_attribute a ON a.id = e.ua_id
               WHERE a.o_name = '{ADMIN_UA_NAME}' AND a.deleted_at IS NULL
             )
           $fn$"#,
    )
}

/// 幂等建立判定链 PIP 段的所有 DB 函数（`effective_ua_ids` → `is_admin`；顺序不可反）。
/// 启动期由 SSO / Gateway `build_server` 各调一次；`isahl_auth` 缺失 ⇒ 静默跳过。
///
/// 并发安全：`CREATE OR REPLACE FUNCTION` 同一函数会在系统目录上撞车，PG 直接报
/// `tuple concurrently updated`（实测：并行测试/多进程首用 ensure 同时进入）。
/// 本函数是 `is_admin_user` 的前置，失败 ⇒ admin 判定 **fail-closed** ⇒ 管理者被误判 403。
/// 故 DDL MUST 在事务内先取 advisory 锁（`NGAC_PREDICATE_FN_LOCK`，事务结束自动释放）。
pub async fn ensure_ngac_predicate_fns(pool: &PgPool) -> Result<(), AliothError> {
    if !sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT FROM information_schema.schemata WHERE schema_name='isahl_auth')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false)
    {
        return Ok(());
    }
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| AliothError::Internal(format!("ensure ngac predicate fns: begin: {}", e)))?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(NGAC_PREDICATE_FN_LOCK)
        .execute(&mut *tx)
        .await
        .map_err(|e| AliothError::Internal(format!("ensure ngac predicate fns: lock: {}", e)))?;
    for (fn_name, ddl) in [
        (CONDITION_TS_FN, condition_ts_fn_ddl()),
        (EFFECTIVE_UA_IDS_FN, effective_ua_ids_fn_ddl()),
        (ADMIN_PREDICATE_FN, admin_predicate_fn_ddl()),
    ] {
        sqlx::query(AssertSqlSafe(ddl.as_str()))
            .execute(&mut *tx)
            .await
            .map_err(|e| AliothError::Internal(format!("ensure {}: {}", fn_name, e)))?;
    }
    tx.commit()
        .await
        .map_err(|e| AliothError::Internal(format!("ensure ngac predicate fns: commit: {}", e)))?;
    Ok(())
}

/// advisory lock 键（全局唯一、跨进程有效）：NGAC 断言函数 ensure 串行化用（见上）。
const NGAC_PREDICATE_FN_LOCK: i64 = 0x4E47_4143_5052_0001_u64 as i64;

/// 本进程内「已确保函数存在」标记（成功才置位；失败下次重试，幂等 DDL 无副作用）。
static ADMIN_FN_ENSURED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// NGAC admin UA 判定（**唯一实现**，形态 = 调 DB 函数 `isahl_auth.is_admin($1)`）。
///
/// 语义见 [`ADMIN_PREDICATE_FN`]（直派 ∪ 认知派生 ∪ 委托派生 ∪ 祖先闭包，含时效过滤），
/// 与 [`require_resource_access`] 的 admin 治理兜底同源（NGAC_SPEC §6.2「遍历后兜底」）。
/// `isahl_auth` schema 缺失 ⇒ `false`（fail-closed：无 NGAC 面即无 admin 身份）。
pub async fn is_admin_user(pool: &PgPool, user_id: i64) -> Result<bool, AliothError> {
    if !sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT FROM information_schema.schemata WHERE schema_name='isahl_auth')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false)
    {
        return Ok(false);
    }
    if ADMIN_FN_ENSURED.get().is_none() {
        // 未确保前先确保（幂等）；失败即 fail-closed（false），不把「函数缺失」当「非 admin」以外的语义
        match ensure_ngac_predicate_fns(pool).await {
            Ok(()) => {
                let _ = ADMIN_FN_ENSURED.set(());
            }
            Err(e) => {
                crate::telemetry::warn!("ensure {} failed: {}", ADMIN_PREDICATE_FN, e);
                return Ok(false);
            }
        }
    }
    sqlx::query_scalar::<_, bool>("SELECT isahl_auth.is_admin($1)")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(|e| AliothError::Internal(format!("Admin check: {}", e)))
}

pub async fn require_resource_access(
    pool: &PgPool,
    user_id: i64,
    resource_type: &str,
    resource_id: i64,
    action: &str,
) -> Result<(), AliothError> {
    // isahl_auth 未启用（未部署 NGAC 面）⇒ 跳过全部 NGAC 检查。
    if !sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT FROM information_schema.schemata WHERE schema_name='isahl_auth')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(false)
    {
        return Ok(());
    }
    // 决策语义 + 输入装配 + 编排唯一实现 = `ngac-host::Evaluator`（openspec
    // extract-ngac-decision-host）：ceiling → bootstrap → UA/OA 闭包 → 合并 →
    // creator 翻转 → 决策序。本函数只做**终态映射**（fail-closed）。
    match ngac_host::Evaluator::global()
        .decide(pool, user_id, resource_type, resource_id, action)
        .await
    {
        Ok(ngac_decision::Decision::Permit) => Ok(()),
        Ok(_) => Err(AliothError::Forbidden(format!(
            "Access denied: user {} lacks '{}' on {}:{}",
            user_id, action, resource_type, resource_id
        ))),
        Err(e) => Err(AliothError::Internal(format!("NGAC decide: {}", e))),
    }
}

/// 行级判定拒绝时回落集合级（fk_resource=0）判定（fix-avic-fd-e2e D7）。
///
/// 背景：`require_resource_access` 的 resource_attrs CTE 在**存在行级 OA 时不再回落
/// 集合 OA**（NGAC_SPEC 全局属性回落语义）。实例创建路径注册的行级 OA 仅关联创建者
/// UA → 持集合 approver 权的审批人被行 OA 存在性遮蔽（403），与 monitor 旧路径
/// （无行 OA，集合权治理实例动作）语义回归。
///
/// 本函数语义：行级命中 → 放行；行级 Forbidden 且集合级命中（approver UA 类权）→
/// 放行；两级皆拒 → 返回行级原始错误。prohibition 治理不回落（集合级 prohibited
/// 同样拒绝——deny-overrides 在两级判定内各自先行）。
pub async fn require_row_or_collection_access(
    pool: &sqlx::PgPool,
    user_id: i64,
    resource_type: &str,
    resource_id: i64,
    action: &str,
) -> Result<(), AliothError> {
    match require_resource_access(pool, user_id, resource_type, resource_id, action).await {
        Ok(()) => Ok(()),
        Err(first @ AliothError::Forbidden(_)) => {
            // 集合级回落双形尝试（历史兜底）：注册面（种子/引擎）的集合 OA `resource_type` 为下划线形
            // （如 `approval_instances`），而调用面历史上出现过连字符形（如 `approval-instances`）。
            // 2026-10-10 清理：`Framework/backend/approval/src/handlers/*` 五处调用点已统一为注册形；
            // 本兜底保留作通用护栏（新调用点若误用连字符仍可命中），两形任一命中即放行。
            let alt: String = resource_type.replace('-', "_");
            let candidates = if alt == resource_type {
                vec![0i64]
            } else {
                vec![0i64, -1i64]
            };
            for flag in candidates {
                let rt = if flag == -1 {
                    alt.as_str()
                } else {
                    resource_type
                };
                if require_resource_access(pool, user_id, rt, 0, action)
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
            }
            Err(first)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod admin_predicate_tests {
    use super::*;
    use ngac_host::attr_names_include_admin;

    /// 名字集恰为 `{admin}`：别名（含 `superadmin`）MUST NOT 构成管理员
    /// （用户裁决 2026-09-29；后端实现在此收敛前从未认过 `superadmin`）。
    #[test]
    fn admin_name_set_is_exactly_admin() {
        assert_eq!(ADMIN_UA_NAME, "admin");
        assert!(attr_names_include_admin(["admin"]));
        assert!(attr_names_include_admin(["employee", "admin"]));
        assert!(!attr_names_include_admin(["superadmin"]));
        assert!(!attr_names_include_admin(["Admin"]));
        assert!(!attr_names_include_admin([] as [&str; 0]));
        let near_miss = String::from("admin-ops");
        assert!(!attr_names_include_admin([near_miss.as_str()]));
    }
}

// 原 `sql_template_invariants`（对「判定 SQL 模板 + format! 片段拼接」的源码级不变量
// 测试）随任务 4.3 的重写一并退役：候选规则装配 SQL 已删除（决策语义收敛
// `ngac-decision` crate，剩余模板只拼接 `{ownership}` 单片段且不再注入策略规则）。
// 运行期兜底断言 `tests/permissions_runtime_test.rs::judgment_sql_never_yields_internal_error`
// 继续守「判定链不产生 Internal」面。
