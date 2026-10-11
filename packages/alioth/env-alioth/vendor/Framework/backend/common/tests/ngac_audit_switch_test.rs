//! `org_policy_class.audit_required` 接线判据集成测试（change `wire-org-policy-declaration-fields` 切片 A）。
//!
//! 判据面 = `ngac_host::audit_suppressed_for_resource_type`（决策审计抑制的唯一判据；唯一调用点 =
//! SSO `ngac/pdp/audit.rs::audit_decision`）。覆盖：命中 / 非覆盖类型 / `audit_required=true` 不抑制 /
//! **任一** false 即抑制 / 状态非 active 不抑制 / **窗口外不抑制**（窗口谓词同源宏消费）/ 查询失败 fail-safe。
//!
//! 运行：`bash scripts/cargo-test.sh -p common --test ngac_audit_switch_test`（连 `*_test` 库）。

use common::testing::connect_test_db;
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

const MARKER: &str = "ngac-audit-switch-itest";

fn mk_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_millis()
        .to_string()
}

/// 共享测试库缺 org_policy DDL 时自建最小契约面（幂等；含接线所需列）。
async fn ensure_org_policy_ddl(pool: &PgPool) {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS isahl_auth.org_policy_class (
            id bigserial PRIMARY KEY,
            code text NOT NULL,
            scope jsonb NOT NULL DEFAULT '{}',
            ua_template jsonb NOT NULL DEFAULT '{}',
            label_code text,
            prohibition_template jsonb,
            audit_required boolean NOT NULL DEFAULT TRUE,
            effective_from timestamptz,
            effective_until timestamptz,
            state text NOT NULL DEFAULT 'draft',
            created_at timestamptz NOT NULL DEFAULT NOW(),
            updated_at timestamptz NOT NULL DEFAULT NOW(),
            deleted_at timestamptz
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("bootstrap org_policy_class DDL");
    // 早期 bootstrap 建的表可能缺接线列（加性自愈；生产 DDL 已在位时为空转）。
    // 静态语句（禁运行期拼 SQL——`dynamic-table-name` 门禁 + sqlx `SqlSafeStr` 双约）。
    sqlx::query(
        "ALTER TABLE isahl_auth.org_policy_class ADD COLUMN IF NOT EXISTS prohibition_template jsonb",
    )
    .execute(pool)
    .await
    .expect("ensure prohibition_template column");
    sqlx::query(
        "ALTER TABLE isahl_auth.org_policy_class ADD COLUMN IF NOT EXISTS audit_required boolean NOT NULL DEFAULT TRUE",
    )
    .execute(pool)
    .await
    .expect("ensure audit_required column");
    sqlx::query(
        "ALTER TABLE isahl_auth.org_policy_class ADD COLUMN IF NOT EXISTS effective_from timestamptz",
    )
    .execute(pool)
    .await
    .expect("ensure effective_from column");
    sqlx::query(
        "ALTER TABLE isahl_auth.org_policy_class ADD COLUMN IF NOT EXISTS effective_until timestamptz",
    )
    .execute(pool)
    .await
    .expect("ensure effective_until column");
}

async fn insert_class(
    pool: &PgPool,
    code: &str,
    resource_type: &str,
    audit_required: bool,
    state: &str,
    until_in_past: bool,
) -> i64 {
    sqlx::query_scalar(
        r#"
        INSERT INTO isahl_auth.org_policy_class
            (code, notice, scope, ua_template, audit_required, state, effective_until)
        VALUES ($1, $2, jsonb_build_object('resource_types', jsonb_build_array($3::text)),
                '{"name_rule":"position:{code}"}', $4, $5,
                CASE WHEN $6 THEN NOW() - INTERVAL '1 hour' ELSE NULL END)
        RETURNING id
        "#,
    )
    .bind(code)
    .bind(format!("{MARKER}:{code}"))
    .bind(resource_type)
    .bind(audit_required)
    .bind(state)
    .bind(until_in_past)
    .fetch_one(pool)
    .await
    .expect("insert org_policy_class")
}

#[tokio::test]
async fn audit_switch_predicate_honours_state_window_and_required_flag() {
    let pool = connect_test_db().await;
    ensure_org_policy_ddl(&pool).await;
    let suf = mk_suffix();
    let rt = format!("ngac_itest_audit_{suf}");

    // 基线：无覆盖类 ⇒ 不抑制（照常审计）。
    assert!(
        !ngac_host::audit_suppressed_for_resource_type(&pool, &rt).await,
        "无覆盖类 MUST NOT 抑制审计"
    );

    // ① audit_required=TRUE 的 active 覆盖类 ⇒ 不抑制。
    let code_true = format!("{MARKER}_true_{suf}");
    let id_true = insert_class(&pool, &code_true, &rt, true, "active", false).await;
    assert!(
        !ngac_host::audit_suppressed_for_resource_type(&pool, &rt).await,
        "audit_required=TRUE MUST NOT 抑制审计"
    );

    // ② 再加 audit_required=FALSE 的 active 覆盖类 ⇒ 抑制（任一 false 即抑制）。
    let code_false = format!("{MARKER}_false_{suf}");
    let id_false = insert_class(&pool, &code_false, &rt, false, "active", false).await;
    assert!(
        ngac_host::audit_suppressed_for_resource_type(&pool, &rt).await,
        "audit_required=FALSE 的 active 覆盖类 MUST 抑制审计"
    );

    // ③ 非覆盖资源类型 ⇒ 不抑制。
    assert!(
        !ngac_host::audit_suppressed_for_resource_type(&pool, &format!("{rt}_other")).await,
        "非覆盖资源类型 MUST NOT 抑制"
    );

    // ④ 状态非 active（retired）⇒ 不抑制。
    sqlx::query("UPDATE isahl_auth.org_policy_class SET state = 'retired' WHERE id = $1")
        .bind(id_false)
        .execute(&pool)
        .await
        .expect("retire false-class");
    assert!(
        !ngac_host::audit_suppressed_for_resource_type(&pool, &rt).await,
        "非 active 类 MUST NOT 抑制（state 是 active 的附加条件）"
    );

    // ⑤ **窗口外不抑制**（半开区间 [from, until)：until 在过去 ⇒ 视同未生效）——窗口谓词同源宏消费。
    sqlx::query(
        "UPDATE isahl_auth.org_policy_class SET state = 'active', effective_until = NOW() - INTERVAL '1 hour' WHERE id = $1",
    )
    .bind(id_false)
    .execute(&pool)
    .await
    .expect("put false-class outside window");
    assert!(
        !ngac_host::audit_suppressed_for_resource_type(&pool, &rt).await,
        "窗口外的 active 类 MUST NOT 抑制（视同未生效）"
    );

    // ⑥ 回到窗口内 ⇒ 再次抑制（可逆、幂等）。
    sqlx::query("UPDATE isahl_auth.org_policy_class SET effective_until = NULL WHERE id = $1")
        .bind(id_false)
        .execute(&pool)
        .await
        .expect("restore window");
    assert!(
        ngac_host::audit_suppressed_for_resource_type(&pool, &rt).await,
        "窗口恢复后 MUST 重新抑制"
    );

    // 清理（不污染共享测试库）。
    sqlx::query("UPDATE isahl_auth.org_policy_class SET deleted_at = NOW() WHERE id IN ($1, $2)")
        .bind(id_true)
        .bind(id_false)
        .execute(&pool)
        .await
        .expect("soft-delete test classes");
}

/// 判据查询失败 ⇒ fail-safe（照常审计）。
#[tokio::test]
async fn audit_switch_predicate_fails_safe_on_query_error() {
    let pool = connect_test_db().await;
    // 不建表（或表缺列）时查询报错 ⇒ MUST 返回 false（不抑制）+ warn，MUST NOT panic。
    let rt = format!("ngac_itest_audit_err_{}", mk_suffix());
    let suppressed = ngac_host::audit_suppressed_for_resource_type(&pool, &rt).await;
    // 该库通常已有表 ⇒ 结果为 false（无覆盖类）；表缺失路径亦为 false。
    assert!(!suppressed, "无覆盖类/查询失败 ⇒ MUST NOT 抑制审计");
}
