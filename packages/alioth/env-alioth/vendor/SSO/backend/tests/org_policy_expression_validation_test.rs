//! 权限规范表达式写径强校验 + 编辑门 + 审计 集成测试
//! （route-ngac-policy-authoring-through-ai-assist 2.7）。
//!
//! 覆盖（#[tokio::test] + connect_test_db）：
//! (a) 非法 Rhai（未知标识符）/ 结构化占位 ⇒ 拒绝且不落库（fail-closed）；
//! (b) 合法 Rhai ⇒ 落库并原样往返；
//! (c) 编辑门：active 类内容更新与 rule 挂接被拒（状态机冻结）；
//! (d) 每次成功写入留下 isahl_audit.audit_events 记录（操作者 = 发起者）。
//!
//! 使用测试库；用例自建数据自清（code 前缀独占）。

mod common;

use gateway_sso::ngac::org_policy::{
    activate_org_policy_class, create_org_policy_class, create_org_policy_rule,
    submit_review_org_policy_class, update_org_policy_class, CreateOrgPolicyClass,
    CreateOrgPolicyRule, UpdateOrgPolicyClass,
};
use serde_json::json;
use sqlx::PgPool;

async fn connect() -> PgPool {
    ::common::testing::connect_test_db().await
}

fn uniq(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{nanos}")
}

/// 自建正 ID 发起者（审计 `user_id` 必须为正；测试库既有行的 id 符号不可依赖）。
async fn actor(pool: &PgPool) -> (i64, String) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let uid = 980_000_000 + (nanos % 1_000_000) as i64;
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, status, is_active, created_at, updated_at) \
         VALUES ($1, $2, 'active', true, NOW(), NOW()) ON CONFLICT (id) DO UPDATE SET is_active = true",
    )
    .bind(uid)
    .bind(format!("rtngac_actor_{uid}"))
    .execute(pool)
    .await
    .expect("insert actor");
    (uid, format!("rtngac_actor_{uid}@test.local"))
}

async fn cleanup_actor(pool: &PgPool, uid: i64) {
    let _ = sqlx::query(
        "DELETE FROM isahl_audit.audit_events WHERE user_id = $1 AND operation LIKE 'policy.%'",
    )
    .bind(uid)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(uid)
        .execute(pool)
        .await;
}

async fn cleanup(pool: &PgPool, code_prefix: &str) {
    let _ = sqlx::query(
        "DELETE FROM isahl_auth.org_policy_rule WHERE policy_class_id IN \
         (SELECT id FROM isahl_auth.org_policy_class WHERE code LIKE $1)",
    )
    .bind(format!("{code_prefix}%"))
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM isahl_auth.org_policy_class WHERE code LIKE $1")
        .bind(format!("{code_prefix}%"))
        .execute(pool)
        .await;
}

fn class_input(
    code: &str,
    prohibition_template: Option<serde_json::Value>,
) -> CreateOrgPolicyClass {
    CreateOrgPolicyClass {
        code: code.to_string(),
        notice: "集成测试规范".to_string(),
        scope: None,
        ua_template: None,
        label_code: None,
        prohibition_template,
        audit_required: true,
        effective_from: None,
        effective_until: None,
        comments: None,
    }
}

#[tokio::test]
async fn invalid_rhai_rejected_and_not_persisted_then_valid_roundtrips_with_audit() {
    let pool = connect().await;
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(&pool).await;
    let (uid, email) = actor(&pool).await;
    let code = uniq("rtngac-expr");
    cleanup(&pool, &code).await;

    // (a) 未知标识符 ⇒ 拒绝 + 不落库
    let err = create_org_policy_class(
        &pool,
        uid,
        &email,
        class_input(&code, Some(json!("amount > 100"))),
    )
    .await
    .expect_err("未知标识符表达式必须拒绝");
    assert!(err.to_string().contains("fail-closed"), "{err}");
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM isahl_auth.org_policy_class WHERE code = $1")
            .bind(&code)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0, "非法表达式 MUST NOT 落库");

    // 结构化占位 ⇒ 拒绝
    let err = create_org_policy_class(
        &pool,
        uid,
        &email,
        class_input(&code, Some(json!({"and": []}))),
    )
    .await
    .expect_err("结构化占位必须拒绝");
    assert!(err.to_string().contains("Rhai 源码字符串"), "{err}");

    // (b) 合法 Rhai ⇒ 落库并原样往返（键集 = 决策期求值器变量集；判决期变量 `permitted` 见下）
    let legal = json!("action == \"read\" && user_id > 0");
    let row = create_org_policy_class(&pool, uid, &email, class_input(&code, Some(legal.clone())))
        .await
        .expect("合法表达式必须通过");
    assert_eq!(row.prohibition_template, Some(legal));

    // (c) 判决期变量（`permitted`）在 `prohibition_template` 上 ⇒ 拒绝（写径键集收窄，tasks 1.6）
    let err = create_org_policy_class(
        &pool,
        uid,
        &email,
        class_input(&code, Some(json!("action == \"read\" && permitted"))),
    )
    .await
    .expect_err("prohibition_template 引用判决期变量必须拒绝");
    assert!(err.to_string().contains("fail-closed"), "{err}");

    // (d) 审计落行（操作者 = 发起者）
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_audit.audit_events \
         WHERE operation = 'policy.class.create' AND object_path LIKE $1 AND user_id = $2",
    )
    .bind(format!("org_policy_class/{}%", row.id))
    .bind(uid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(n >= 1, "成功写入 MUST 留审计记录");

    // 更新路径同判据：patch 非法表达式 ⇒ 拒绝且原值不变
    let err = update_org_policy_class(
        &pool,
        uid,
        &email,
        row.id,
        UpdateOrgPolicyClass {
            prohibition_template: Some(json!("bogus_key > 1")),
            ..Default::default()
        },
    )
    .await
    .expect_err("patch 非法表达式必须拒绝");
    assert!(err.to_string().contains("fail-closed"), "{err}");
    let kept: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT prohibition_template FROM isahl_auth.org_policy_class WHERE id = $1",
    )
    .bind(row.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        kept,
        Some(json!("action == \"read\" && user_id > 0")),
        "拒绝时原值不变"
    );

    cleanup(&pool, &code).await;
    cleanup_actor(&pool, uid).await;
}

#[tokio::test]
async fn active_class_frozen_for_content_write_and_rule_attach() {
    let pool = connect().await;
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(&pool).await;
    let (uid, email) = actor(&pool).await;
    let code = uniq("rtngac-gate");
    cleanup(&pool, &code).await;

    let row = create_org_policy_class(&pool, uid, &email, class_input(&code, None))
        .await
        .expect("draft 新建");
    submit_review_org_policy_class(&pool, uid, &email, row.id)
        .await
        .expect("draft → in_review");
    let active = activate_org_policy_class(&pool, uid, &email, row.id)
        .await
        .expect("in_review → active");
    assert_eq!(active.state, "active");

    // (c) active 冻结：内容更新被编辑门拒
    let err = update_org_policy_class(
        &pool,
        uid,
        &email,
        row.id,
        UpdateOrgPolicyClass {
            notice: Some("改说明".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect_err("active 类内容更新必须被编辑门拒绝");
    assert!(err.to_string().contains("冻结"), "{err}");

    // (c2) active 冻结：rule 挂接被拒
    let err = create_org_policy_rule(
        &pool,
        uid,
        &email,
        CreateOrgPolicyRule {
            policy_class_id: row.id,
            subject_code: "position:tester".to_string(),
            resource_type: "engineers".to_string(),
            actions: vec!["read".to_string()],
            condition: Some(json!("action == \"read\"")),
            obligation: None,
            label_code: None,
        },
    )
    .await
    .expect_err("active 类不可挂接 rule");
    assert!(err.to_string().contains("冻结"), "{err}");

    cleanup(&pool, &code).await;
    cleanup_actor(&pool, uid).await;
}

#[tokio::test]
async fn rule_expression_validated_before_persist() {
    let pool = connect().await;
    gateway_sso::ngac::ensure::ensure_ngac_extension_tables(&pool).await;
    let (uid, email) = actor(&pool).await;
    let code = uniq("rtngac-rule");
    cleanup(&pool, &code).await;

    let cls = create_org_policy_class(&pool, uid, &email, class_input(&code, None))
        .await
        .expect("draft 新建");

    // 非法：未注册自由函数（R8 盲区由白名单层拦截）
    let err = create_org_policy_rule(
        &pool,
        uid,
        &email,
        CreateOrgPolicyRule {
            policy_class_id: cls.id,
            subject_code: "position:tester".to_string(),
            resource_type: "engineers".to_string(),
            actions: vec!["read".to_string()],
            condition: Some(json!("not_a_builtin(1)")),
            obligation: None,
            label_code: None,
        },
    )
    .await
    .expect_err("未注册自由函数必须拒绝");
    assert!(err.to_string().contains("fail-closed"), "{err}");

    // 判决期变量（`permitted`）在 `condition` 上 ⇒ 拒绝（写径键集收窄：条件在判决终态前求值，
    // 接受即「写入成功但永不满足」的静默面；tasks 1.6）
    let err = create_org_policy_rule(
        &pool,
        uid,
        &email,
        CreateOrgPolicyRule {
            policy_class_id: cls.id,
            subject_code: "position:tester".to_string(),
            resource_type: "engineers".to_string(),
            actions: vec!["read".to_string()],
            condition: Some(json!("action == \"read\" && permitted")),
            obligation: None,
            label_code: None,
        },
    )
    .await
    .expect_err("condition 引用判决期变量必须拒绝");
    assert!(err.to_string().contains("fail-closed"), "{err}");

    // 合法：条件表达式落库 + 审计（policy.rule.create；键集 = 决策期求值器变量集）
    let rule = create_org_policy_rule(
        &pool,
        uid,
        &email,
        CreateOrgPolicyRule {
            policy_class_id: cls.id,
            subject_code: "position:tester".to_string(),
            resource_type: "engineers".to_string(),
            actions: vec!["read".to_string()],
            condition: Some(json!("action == \"read\" && user_id > 0")),
            obligation: None,
            label_code: None,
        },
    )
    .await
    .expect("合法 rule");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_audit.audit_events \
         WHERE operation = 'policy.rule.create' AND object_path LIKE $1",
    )
    .bind(format!("org_policy_rule/{}%", rule.id))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(n >= 1, "rule 成功写入 MUST 留审计");

    cleanup(&pool, &code).await;
    cleanup_actor(&pool, uid).await;
}
