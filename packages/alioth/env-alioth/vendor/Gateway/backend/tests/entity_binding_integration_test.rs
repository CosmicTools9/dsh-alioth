//! Entity Binding 集成测试（refactor-wz-trade-chain-ownership Group 1）
//!
//! 覆盖：
//! - status.operator_org_id：组织类叶表（非银行法人 / subj-org / subjects 父表组织行）
//!   → 回传 entity_id；自然人 / 未绑定 → null
//! - bind_enterprise：company_name 新建 zc_id_orga-non-banking-legal + 绑定，
//!   绑定后 status.operator_org_id = 新组织 id；重复绑定 → 400 ALREADY_BOUND
//! - 特权 UA 豁免：admin/auditor/operator/enterprise UA 未绑 entity → bound=true 且
//!   operator_org_id=null（存量特权用户不被引导页劫持，也不冒充运营组织）
//!
//! 使用 aliothstudio_test 数据库，负数 ID fixture，自建自清。

use ::common::context::RequestContext;
use ::common::testing::connect_test_db;
use actix_web::{test, web, HttpMessage, HttpRequest, HttpResponse};
use alioth_gateway::api::entity_binding;
use sqlx::PgPool;

// ── Fixture IDs（负数段，避免与真实数据冲突）─────────────────────────────────
const USER_ORG_LEGAL: i64 = -9811; // 绑定 zc_id_orga-non-banking-legal
const USER_ORG_SUBJORG: i64 = -9812; // 绑定 zc_id_subj-org
const USER_ORG_SUBJECTS: i64 = -9813; // 绑定 subjects 父表组织行（WZ 库组织形态）
const USER_PERSONAL: i64 = -9814; // 绑定 zc_id_empl-natural
const USER_UNBOUND: i64 = -9815; // 未绑定
const USER_PRIVILEGED: i64 = -9816; // admin UA 豁免（未绑 entity）
const USER_DANGLING: i64 = -9818; // 悬空绑定（entity_id 指向不存在的主体行）
const USER_BIND: i64 = -9817; // bind_enterprise 流程用
const ORG_LEGAL_ID: i64 = -97111;
const ORG_SUBJORG_ID: i64 = -97112;
const ORG_SUBJECTS_ID: i64 = -97113;
const EMPL_PERSONAL_ID: i64 = -97114;

fn req_with_user(user_id: i64) -> HttpRequest {
    let req = test::TestRequest::default().to_http_request();
    req.extensions_mut().insert(RequestContext::new(
        user_id,
        format!("user{}@test.local", user_id),
    ));
    req
}

async fn json_body(resp: HttpResponse) -> serde_json::Value {
    let body = actix_web::body::to_bytes(resp.into_body())
        .await
        .expect("body");
    serde_json::from_slice(&body).expect("json")
}

/// status handler 直调，返回响应 JSON。
async fn call_status(pool: &PgPool, user_id: i64) -> serde_json::Value {
    let resp = entity_binding::status(req_with_user(user_id), web::Data::new(pool.clone())).await;
    assert!(
        resp.status().is_success(),
        "status 应 200: {}",
        resp.status()
    );
    json_body(resp).await
}

/// 幂等建用户（负 ID；重复运行先重置 entity 绑定）。
async fn ensure_test_user(pool: &PgPool, uid: i64) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, status, is_active, created_at, updated_at)
           VALUES ($1, $2, $3, $4, 'active', true, NOW(), NOW())
           ON CONFLICT (id) DO UPDATE
             SET entity_table = NULL, entity_id = NULL, is_active = true"#,
    )
    .bind(uid)
    .bind(format!("eb-test-{uid}"))
    .bind(format!("eb-test-{uid}"))
    .bind(format!("eb-test-{uid}@test.local"))
    .execute(pool)
    .await
    .expect("insert auth_users fixture");
}

async fn cleanup(pool: &PgPool) {
    let users = [
        USER_ORG_LEGAL,
        USER_ORG_SUBJORG,
        USER_ORG_SUBJECTS,
        USER_PERSONAL,
        USER_UNBOUND,
        USER_PRIVILEGED,
        USER_BIND,
    ];
    // 用户↔UA 关联（特权豁免 + bind_enterprise 的 enterprise UA 指派）
    sqlx::query(
        r#"DELETE FROM isahl_auth.ngac_user_rr_attribute
           WHERE fk_user = ANY($1) OR (o_name = 'eb-privileged' AND fk_user = ANY($1))"#,
    )
    .bind(&users[..])
    .execute(pool)
    .await
    .ok();
    // bind_enterprise 新建法人（code=org-{uid}）
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_orga-non-banking-legal"
           WHERE code = ANY($1)"#,
    )
    .bind(users.iter().map(|u| format!("org-{u}")).collect::<Vec<_>>())
    .execute(pool)
    .await
    .ok();
    // subjects 叶表 fixture
    sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1"#)
        .bind(ORG_LEGAL_ID)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_subj-org" WHERE id = $1"#)
        .bind(ORG_SUBJORG_ID)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_subjects WHERE id = $1"#)
        .bind(ORG_SUBJECTS_ID)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_empl-natural" WHERE id = $1"#)
        .bind(EMPL_PERSONAL_ID)
        .execute(pool)
        .await
        .ok();
    // 用户行
    sqlx::query(r#"DELETE FROM isahl_auth.auth_users WHERE id = ANY($1)"#)
        .bind(&users[..])
        .execute(pool)
        .await
        .ok();
}

/// 准备四类绑定 fixture + 未绑定用户。
async fn setup_binding_fixtures(pool: &PgPool) {
    cleanup(pool).await;
    for uid in [
        USER_ORG_LEGAL,
        USER_ORG_SUBJORG,
        USER_ORG_SUBJECTS,
        USER_PERSONAL,
        USER_UNBOUND,
        USER_PRIVILEGED,
        USER_BIND,
    ] {
        ensure_test_user(pool, uid).await;
    }

    // 组织类叶表：非银行法人
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_orga-non-banking-legal" (id, notice, code, created_by_id)
           VALUES ($1, 'eb-test-legal', 'eb-org-legal', -1)"#,
    )
    .bind(ORG_LEGAL_ID)
    .execute(pool)
    .await
    .expect("insert legal org fixture");
    sqlx::query(
        r#"UPDATE isahl_auth.auth_users
           SET entity_table = 'zc_id_orga-non-banking-legal', entity_id = $1
           WHERE id = $2"#,
    )
    .bind(ORG_LEGAL_ID)
    .bind(USER_ORG_LEGAL)
    .execute(pool)
    .await
    .expect("bind legal org user");

    // 组织类叶表：subj-org 子表
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-org" (id, notice, code, created_by_id, updated_by_id)
           VALUES ($1, 'eb-test-subjorg', 'eb-org-subjorg', -1, -1)"#,
    )
    .bind(ORG_SUBJORG_ID)
    .execute(pool)
    .await
    .expect("insert subj-org fixture");
    sqlx::query(
        r#"UPDATE isahl_auth.auth_users
           SET entity_table = 'zc_id_subj-org', entity_id = $1
           WHERE id = $2"#,
    )
    .bind(ORG_SUBJORG_ID)
    .bind(USER_ORG_SUBJORG)
    .execute(pool)
    .await
    .expect("bind subj-org user");

    // 组织类叶表：subjects 父表直落组织行（WZ 等库组织形态）
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_subjects (id, code, notice, created_by_id)
           VALUES ($1, 'EB-ORG-SUBJECTS', 'eb-test-subjects-org', -1)"#,
    )
    .bind(ORG_SUBJECTS_ID)
    .execute(pool)
    .await
    .expect("insert subjects org fixture");
    sqlx::query(
        r#"UPDATE isahl_auth.auth_users
           SET entity_table = 'zc_id_subjects', entity_id = $1
           WHERE id = $2"#,
    )
    .bind(ORG_SUBJECTS_ID)
    .bind(USER_ORG_SUBJECTS)
    .execute(pool)
    .await
    .expect("bind subjects org user");

    // 自然人叶表（非组织 → operator_org_id 必须为 null）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code, created_by_id)
           VALUES ($1, 'eb-test-person', 'eb-empl-person', -1)"#,
    )
    .bind(EMPL_PERSONAL_ID)
    .execute(pool)
    .await
    .expect("insert empl-natural fixture");
    sqlx::query(
        r#"UPDATE isahl_auth.auth_users
           SET entity_table = 'zc_id_empl-natural', entity_id = $1
           WHERE id = $2"#,
    )
    .bind(EMPL_PERSONAL_ID)
    .bind(USER_PERSONAL)
    .execute(pool)
    .await
    .expect("bind personal user");
}

// ── 1.1 status.operator_org_id ─────────────────────────────────────────────

#[tokio::test]
async fn status_org_leaf_returns_operator_org_id() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;

    for (uid, expect_org_id) in [
        (USER_ORG_LEGAL, ORG_LEGAL_ID),
        (USER_ORG_SUBJORG, ORG_SUBJORG_ID),
        (USER_ORG_SUBJECTS, ORG_SUBJECTS_ID),
    ] {
        let json = call_status(&pool, uid).await;
        assert!(
            json["bound"].as_bool().unwrap_or(false),
            "组织绑定用户应 bound"
        );
        assert_eq!(
            json["operator_org_id"].as_str(),
            Some(expect_org_id.to_string().as_str()),
            "user {uid} 的 operator_org_id 应为组织 entity_id"
        );
        assert_eq!(
            json["entity_id"].as_str(),
            Some(expect_org_id.to_string().as_str())
        );
    }
}

#[tokio::test]
async fn status_personal_and_unbound_operator_org_null() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;

    // 自然人：bound 但 operator_org_id = null
    let json = call_status(&pool, USER_PERSONAL).await;
    assert!(
        json["bound"].as_bool().unwrap_or(false),
        "自然人绑定应 bound"
    );
    assert_eq!(json["type"].as_str(), Some("personal"));
    assert!(json["operator_org_id"].is_null(), "自然人不应有运营组织");
    assert_eq!(
        json["entity_id"].as_str(),
        Some(EMPL_PERSONAL_ID.to_string().as_str())
    );

    // 未绑定：bound=false 且 operator_org_id = null
    let json = call_status(&pool, USER_UNBOUND).await;
    assert!(
        !json["bound"].as_bool().unwrap_or(true),
        "未绑定应 bound=false"
    );
    assert!(json["operator_org_id"].is_null(), "未绑定不应有运营组织");
}
// ── 1.3b 悬空绑定：entity_id 指向不存在的主体行 → bound=false ─────────────
// fix-subject-cognition-residual-gaps D5：门控与 /auth/me subject 双轨一致性

#[tokio::test]
async fn status_dangling_entity_counts_as_unbound() {
    let pool = connect_test_db().await;
    ensure_test_user(&pool, USER_DANGLING).await;
    // entity_id 指向不存在的主体行（负 ID 段无对应 subjects 行）
    sqlx::query(
        "UPDATE isahl_auth.auth_users SET entity_table = 'zc_id_subjects', entity_id = -97999 WHERE id = $1",
    )
    .bind(USER_DANGLING)
    .execute(&pool)
    .await
    .expect("set dangling binding");

    let json = call_status(&pool, USER_DANGLING).await;
    assert!(
        !json["bound"].as_bool().unwrap_or(true),
        "悬空绑定应判 bound=false（主体行不存在）: {json}"
    );

    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(USER_DANGLING)
        .execute(&pool)
        .await
        .ok();
}

// ── 1.4 bind_enterprise：新建组织 + 重复绑定拒绝 ─────────────────────────────

#[tokio::test]
async fn bind_enterprise_creates_org_and_repeat_rejected() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;

    let body = web::Json(entity_binding::EnterpriseBindingBody {
        company_name: "EB 测试运营公司".to_string(),
        representative_name: Some("测试代表".to_string()),
        parent_org_id: None,
        entity_id: None,
        entity_table: None,
    });
    let resp = entity_binding::bind_enterprise(
        req_with_user(USER_BIND),
        web::Data::new(pool.clone()),
        body,
    )
    .await;
    assert!(
        resp.status().is_success(),
        "首次企业绑定应成功: {}",
        resp.status()
    );
    let json = json_body(resp).await;
    let org_id: i64 = json["entity_id"]
        .as_str()
        .expect("应返回 entity_id")
        .parse()
        .expect("entity_id 为数字字符串");

    // 法人叶表行已创建（company_name 新建 zc_id_orga-non-banking-legal）
    let (notice, table): (String, String) = sqlx::query_as(
        r#"SELECT o.notice, u.entity_table
           FROM isahl."zc_id_orga-non-banking-legal" o
           JOIN isahl_auth.auth_users u ON u.entity_id = o.id
           WHERE o.id = $1"#,
    )
    .bind(org_id)
    .fetch_one(&pool)
    .await
    .expect("新建法人应存在");
    assert_eq!(notice, "EB 测试运营公司");
    assert_eq!(table, "zc_id_orga-non-banking-legal");

    // 绑定后 status.operator_org_id = 新组织 id
    let status = call_status(&pool, USER_BIND).await;
    assert!(status["bound"].as_bool().unwrap_or(false));
    assert_eq!(
        status["operator_org_id"].as_str(),
        Some(org_id.to_string().as_str())
    );
    assert_eq!(status["type"].as_str(), Some("enterprise"));

    // 重复绑定 → 400 ALREADY_BOUND
    let resp = entity_binding::bind_enterprise(
        req_with_user(USER_BIND),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::EnterpriseBindingBody {
            company_name: "重复绑定公司".to_string(),
            representative_name: None,
            parent_org_id: None,
            entity_id: None,
            entity_table: None,
        }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400, "重复绑定应 400");
    let json = json_body(resp).await;
    assert_eq!(json["error"].as_str(), Some("ALREADY_BOUND"));

    // 清理本次新建的法人行（code=org-{uid}）
    sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1"#)
        .bind(org_id)
        .execute(&pool)
        .await
        .ok();
}

// ── 1.4 特权 UA 豁免 ────────────────────────────────────────────────────────

#[tokio::test]
async fn status_privileged_ua_exempt_without_org() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;

    // 确保 admin UA 存在（test DB 常规有 seed；缺失则自建，幂等）
    let policy_class: i64 =
        match sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1")
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten()
        {
            Some(id) => id,
            None => sqlx::query_scalar(
                r#"INSERT INTO isahl_auth.ngac_policy_class (id, o_name, is_active, created_at)
                   VALUES (isahl.gen_next_zuid(), 'default', true, NOW()) RETURNING id"#,
            )
            .fetch_one(&pool)
            .await
            .expect("create policy class"),
        };
    let admin_ua: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl_auth.ngac_user_attribute
           WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    let admin_ua = match admin_ua {
        Some(id) => id,
        None => sqlx::query_scalar(
            r#"INSERT INTO isahl_auth.ngac_user_attribute
               (id, o_name, fk_policy_class, ancestor_ids, children_ids)
               VALUES (isahl.gen_next_zuid(), 'admin', $1, '{}'::bigint[], '{}'::bigint[])
               RETURNING id"#,
        )
        .bind(policy_class)
        .fetch_one(&pool)
        .await
        .expect("create admin UA"),
    };
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute
           (id, o_name, fk_user, fk_user_attribute, assigned_at, created_at)
           VALUES (isahl.gen_next_zuid(), 'eb-privileged', $1, $2, NOW(), NOW())"#,
    )
    .bind(USER_PRIVILEGED)
    .bind(admin_ua)
    .execute(&pool)
    .await
    .expect("grant admin UA");

    // 豁免：bound=true（不被引导页劫持），但 operator_org_id=null（未绑组织）
    let json = call_status(&pool, USER_PRIVILEGED).await;
    assert!(
        json["bound"].as_bool().unwrap_or(false),
        "特权 UA 应豁免 bound"
    );
    assert!(json["entity_id"].is_null(), "特权用户无 entity");
    assert!(
        json["operator_org_id"].is_null(),
        "特权豁免不等于运营组织绑定"
    );

    // 清理本次指派的 UA 关联
    sqlx::query(r#"DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1"#)
        .bind(USER_PRIVILEGED)
        .execute(&pool)
        .await
        .ok();
}

// ── fix-seed-user-subject-binding：占位绑定识别 + 任意类型绑定 ──────────────

const USER_PLACEHOLDER: i64 = -9819; // seed 占位绑定（基表行 + subject_binding 标记）

/// 幂等准备占位绑定用户（entity_table='zc_id_subjects' + settings.subject_binding）。
async fn ensure_placeholder_user(pool: &PgPool, uid: i64) {
    ensure_test_user(pool, uid).await;
    // 占位主体（subjects 基表行）
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_subjects (id, code, notice, created_by_id)
           VALUES ($1, 'EB-PLACEHOLDER', 'eb-test-placeholder', -1)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(uid + 1_000_000)
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        r#"UPDATE isahl_auth.auth_users
           SET entity_table = 'zc_id_subjects', entity_id = $1,
               settings = COALESCE(settings, '{}'::jsonb) || '{"subject_binding":"eb-test"}'
           WHERE id = $2"#,
    )
    .bind(uid + 1_000_000)
    .bind(uid)
    .execute(pool)
    .await
    .expect("bind placeholder user");
}

#[tokio::test]
async fn status_placeholder_detection() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;
    ensure_placeholder_user(&pool, USER_PLACEHOLDER).await;

    // 占位绑定 → bound=true, placeholder=true
    let json = call_status(&pool, USER_PLACEHOLDER).await;
    assert!(json["bound"].as_bool().unwrap_or(false), "占位仍 bound");
    assert!(
        json["placeholder"].as_bool().unwrap_or(false),
        "占位绑定应 placeholder=true: {json}"
    );

    // 业务绑定（非银行法人）→ placeholder=false
    let json = call_status(&pool, USER_ORG_LEGAL).await;
    assert!(json["bound"].as_bool().unwrap_or(false));
    assert_eq!(
        json["placeholder"].as_bool(),
        Some(false),
        "业务绑定不应判占位"
    );

    // 未绑定 → placeholder=false
    let json = call_status(&pool, USER_UNBOUND).await;
    assert!(json["placeholder"].is_null() || json["placeholder"].as_bool() == Some(false));

    sqlx::query(r#"DELETE FROM isahl_auth.auth_users WHERE id = $1"#)
        .bind(USER_PLACEHOLDER)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn subject_types_lists_whitelist() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;

    let resp =
        entity_binding::subject_types(req_with_user(USER_UNBOUND), web::Data::new(pool.clone()))
            .await;
    assert!(resp.status().is_success());
    let json = json_body(resp).await;
    let types = json["subject_types"]
        .as_array()
        .expect("subject_types 数组");
    assert!(!types.is_empty(), "白名单清单非空");
    let tables: Vec<&str> = types
        .iter()
        .filter_map(|t| t["subject_type"].as_str())
        .collect();
    assert!(
        tables.contains(&"zc_id_orga-non-banking-legal"),
        "白名单含非银行法人叶表: {tables:?}"
    );
    assert!(
        tables.contains(&"zc_id_bank-commercial"),
        "白名单含商业银行叶表: {tables:?}"
    );
    // 叶表写入规则：中间层（法人/组织）禁入白名单
    assert!(
        !tables.contains(&"zc_id_orga-legal"),
        "法人中间层禁写: {tables:?}"
    );
    assert!(
        !tables.contains(&"zc_id_subj-org"),
        "组织中间层禁写: {tables:?}"
    );
    // 法人类型标记 code_required=true
    let legal = types
        .iter()
        .find(|t| t["subject_type"].as_str() == Some("zc_id_orga-non-banking-legal"))
        .expect("含 zc_id_orga-non-banking-legal");
    assert_eq!(legal["code_required"].as_bool(), Some(true));
    assert_eq!(legal["is_legal"].as_bool(), Some(true));

    // 无 token → 401
    let resp = entity_binding::subject_types(
        test::TestRequest::default().to_http_request(),
        web::Data::new(pool.clone()),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 401);
}

#[tokio::test]
async fn bind_subject_creates_legal_and_replaces_placeholder() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;
    ensure_placeholder_user(&pool, USER_PLACEHOLDER).await;

    // 法人类型缺 code → 400
    let resp = entity_binding::bind_subject(
        req_with_user(USER_PLACEHOLDER),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_orga-non-banking-legal".to_string(),
            notice: "EB 测试企业".to_string(),
            code: None,
            entity_id: None,
            rebind: None,
        }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400, "法人缺编码应 400");

    // 正常创建法人 + 替换占位
    let resp = entity_binding::bind_subject(
        req_with_user(USER_PLACEHOLDER),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_orga-non-banking-legal".to_string(),
            notice: "EB 测试企业".to_string(),
            code: Some("91330000EB0000001A".to_string()),
            entity_id: None,
            rebind: None,
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "创建法人绑定应成功: {}",
        resp.status()
    );
    let json = json_body(resp).await;
    let subject_id: i64 = json["entity_id"]
        .as_str()
        .expect("返回 entity_id")
        .parse()
        .expect("数字");

    // 法人行落 zc_id_orga-non-banking-legal 叶表 + 绑定替换 + 占位标记清除
    let (table, settings): (String, Option<serde_json::Value>) =
        sqlx::query_as(r#"SELECT entity_table, settings FROM isahl_auth.auth_users WHERE id = $1"#)
            .bind(USER_PLACEHOLDER)
            .fetch_one(&pool)
            .await
            .expect("user 存在");
    assert_eq!(table, "zc_id_orga-non-banking-legal");
    let sb = settings
        .as_ref()
        .and_then(|s| s.get("subject_binding"))
        .cloned();
    assert!(sb.is_none(), "占位标记应清除: {settings:?}");
    let notice: String = sqlx::query_scalar(
        r#"SELECT notice FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1"#,
    )
    .bind(subject_id)
    .fetch_one(&pool)
    .await
    .expect("法人行存在");
    assert_eq!(notice, "EB 测试企业");

    // 绑定后 status：placeholder=false
    let status = call_status(&pool, USER_PLACEHOLDER).await;
    assert!(status["bound"].as_bool().unwrap_or(false));
    assert_eq!(
        status["placeholder"].as_bool(),
        Some(false),
        "替换后不应再判占位"
    );

    // 重复绑定（已绑真实主体）→ 400 ALREADY_BOUND
    let resp = entity_binding::bind_subject(
        req_with_user(USER_PLACEHOLDER),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_empl-natural".to_string(),
            notice: "重复自然人".to_string(),
            code: None,
            entity_id: None,
            rebind: None,
        }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400);
    let json = json_body(resp).await;
    assert_eq!(json["error"].as_str(), Some("ALREADY_BOUND"));

    // 清理
    sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1"#)
        .bind(subject_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl_auth.auth_users WHERE id = $1"#)
        .bind(USER_PLACEHOLDER)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn bind_subject_whitelist_and_existing_rejected() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;
    ensure_placeholder_user(&pool, USER_PLACEHOLDER).await;

    // 白名单外类型 → 400，无写入
    let resp = entity_binding::bind_subject(
        req_with_user(USER_PLACEHOLDER),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_even-approve".to_string(),
            notice: "注入尝试".to_string(),
            code: None,
            entity_id: None,
            rebind: None,
        }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400, "白名单外应 400");
    let json = json_body(resp).await;
    assert_eq!(json["error"].as_str(), Some("VALIDATION"));

    // 选择已有主体：tableoid 服务端解析实际叶表（add-subject-rebind-management）
    // ——声明类型与行落点不一致也按 DB 实际落表绑定
    let resp = entity_binding::bind_subject(
        req_with_user(USER_PLACEHOLDER),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_orga-non-banking-legal".to_string(),
            notice: "忽略".to_string(),
            code: None,
            entity_id: Some(ORG_LEGAL_ID), // 实为非银行法人行
            rebind: None,
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "选择已有主体应成功: {}",
        resp.status()
    );
    let (bound_table, bound_id): (String, Option<i64>) =
        sqlx::query_as("SELECT entity_table, entity_id FROM isahl_auth.auth_users WHERE id = $1")
            .bind(USER_PLACEHOLDER)
            .fetch_one(&pool)
            .await
            .expect("user");
    assert_eq!(
        bound_table, "zc_id_orga-non-banking-legal",
        "entity_table 应为 tableoid 解析的实际叶表"
    );
    assert_eq!(bound_id, Some(ORG_LEGAL_ID));

    sqlx::query(r#"DELETE FROM isahl_auth.auth_users WHERE id = $1"#)
        .bind(USER_PLACEHOLDER)
        .execute(&pool)
        .await
        .ok();
}

// ── fix-seed-user-subject-binding：占位绑定可经既有 personal/enterprise 路径替换 ──

const POS_PLACEHOLDER_TEST: i64 = -97116;

#[tokio::test]
async fn bind_enterprise_replaces_placeholder() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;
    ensure_placeholder_user(&pool, USER_PLACEHOLDER).await;

    let resp = entity_binding::bind_enterprise(
        req_with_user(USER_PLACEHOLDER),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::EnterpriseBindingBody {
            company_name: "占位替换测试公司".to_string(),
            representative_name: None,
            parent_org_id: None,
            entity_id: None,
            entity_table: None,
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "占位绑定点企业应成功: {}",
        resp.status()
    );
    let json = json_body(resp).await;
    let org_id: i64 = json["entity_id"]
        .as_str()
        .expect("entity_id")
        .parse()
        .unwrap();

    // 绑定替换 + 占位标记清除 + status.placeholder=false
    let (table, settings): (String, Option<serde_json::Value>) =
        sqlx::query_as("SELECT entity_table, settings FROM isahl_auth.auth_users WHERE id = $1")
            .bind(USER_PLACEHOLDER)
            .fetch_one(&pool)
            .await
            .expect("user");
    assert_eq!(table, "zc_id_orga-non-banking-legal");
    assert!(
        settings
            .as_ref()
            .and_then(|s| s.get("subject_binding"))
            .is_none(),
        "占位标记应清除"
    );
    let status = call_status(&pool, USER_PLACEHOLDER).await;
    assert_eq!(status["placeholder"].as_bool(), Some(false));

    sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1"#)
        .bind(org_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl_auth.auth_users WHERE id = $1"#)
        .bind(USER_PLACEHOLDER)
        .execute(&pool)
        .await
        .ok();
}

#[tokio::test]
async fn bind_personal_replaces_placeholder() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;
    ensure_placeholder_user(&pool, USER_PLACEHOLDER).await;

    // 雇佣主体/岗位 fixture
    let pos_id = POS_PLACEHOLDER_TEST;
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, created_by_id)
           VALUES ($1, 'eb-test-pos', 'eb-pos-fixture', -1)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pos_id)
    .execute(&pool)
    .await
    .ok();

    let resp = entity_binding::bind_personal(
        req_with_user(USER_PLACEHOLDER),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::PersonalBindingBody {
            real_name: "占位替换测试人".to_string(),
            employer_org_id: ORG_SUBJECTS_ID,
            position_id: pos_id,
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "占位绑定点个人应成功: {}",
        resp.status()
    );
    let json = json_body(resp).await;
    let empl_id: i64 = json["entity_id"]
        .as_str()
        .expect("entity_id")
        .parse()
        .unwrap();

    // 绑定替换为自然人 + 占位标记清除
    let (table, settings): (String, Option<serde_json::Value>) =
        sqlx::query_as("SELECT entity_table, settings FROM isahl_auth.auth_users WHERE id = $1")
            .bind(USER_PLACEHOLDER)
            .fetch_one(&pool)
            .await
            .expect("user");
    assert_eq!(table, "zc_id_empl-natural");
    assert!(
        settings
            .as_ref()
            .and_then(|s| s.get("subject_binding"))
            .is_none(),
        "占位标记应清除"
    );
    let status = call_status(&pool, USER_PLACEHOLDER).await;
    assert_eq!(status["placeholder"].as_bool(), Some(false));

    sqlx::query(r#"DELETE FROM isahl."zc_id_empl-natural" WHERE id = $1"#)
        .bind(empl_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_subj-position" WHERE id = $1"#)
        .bind(pos_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl_auth.auth_users WHERE id = $1"#)
        .bind(USER_PLACEHOLDER)
        .execute(&pool)
        .await
        .ok();
}

// ── add-system-subject-sync-on-admin-binding：特权绑定同步 system 主体 ────────

const USER_PRIV_SYNC: i64 = -9821; // 特权用户 A（admin UA，触发同步）
const USER_PRIV_SYNC_2: i64 = -9822; // 特权用户 B（first-wins 不覆盖验证）
const USER_UNPRIV_SYNC: i64 = -9823; // 非特权用户（不触发同步）
const SUBJ_SYNC_PLACEHOLDER: i64 = -97117; // subjects 基表占位行

type SystemBinding = (Option<String>, Option<i64>, Option<serde_json::Value>);

async fn read_system_binding(pool: &PgPool) -> SystemBinding {
    sqlx::query_as(
        "SELECT entity_table, entity_id, settings FROM isahl_auth.auth_users WHERE id = 1",
    )
    .fetch_one(pool)
    .await
    .expect("system 用户应存在（seed 契约 id=1）")
}

/// 强制 system 进入占位绑定态（subjects 基表行 + subject_binding 标记）。
async fn force_system_placeholder(pool: &PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_subjects (id, code, notice, created_by_id)
           VALUES ($1, 'SUBJ-SYNC-TEST', 'system 同步测试占位主体', 1)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(SUBJ_SYNC_PLACEHOLDER)
    .execute(pool)
    .await
    .expect("insert subjects placeholder fixture");
    sqlx::query(
        r#"UPDATE isahl_auth.auth_users
           SET entity_table = 'zc_id_subjects', entity_id = $1,
               settings = '{"subject_binding":"system"}'::jsonb, updated_at = NOW()
           WHERE id = 1"#,
    )
    .bind(SUBJ_SYNC_PLACEHOLDER)
    .execute(pool)
    .await
    .expect("set system placeholder");
}

/// 给用户挂 admin UA（幂等；缺失时自建 policy_class/admin UA 兜底）。
async fn grant_admin_ua(pool: &PgPool, uid: i64) {
    let policy_class: i64 =
        match sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
        {
            Some(id) => id,
            None => sqlx::query_scalar(
                r#"INSERT INTO isahl_auth.ngac_policy_class (id, o_name, is_active, created_at)
                   VALUES (isahl.gen_next_zuid(), 'default', true, NOW()) RETURNING id"#,
            )
            .fetch_one(pool)
            .await
            .expect("create policy class"),
        };
    let admin_ua: i64 = match sqlx::query_scalar(
        r#"SELECT id FROM isahl_auth.ngac_user_attribute
           WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    {
        Some(id) => id,
        None => sqlx::query_scalar(
            r#"INSERT INTO isahl_auth.ngac_user_attribute
               (id, o_name, fk_policy_class, ancestor_ids, children_ids)
               VALUES (isahl.gen_next_zuid(), 'admin', $1, '{}'::bigint[], '{}'::bigint[])
               RETURNING id"#,
        )
        .bind(policy_class)
        .fetch_one(pool)
        .await
        .expect("create admin UA"),
    };
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute
           (id, o_name, fk_user, fk_user_attribute, assigned_at, created_at)
           VALUES (isahl.gen_next_zuid(), 'admin', $1, $2, NOW(), NOW())
           ON CONFLICT (fk_user, fk_user_attribute) DO NOTHING"#,
    )
    .bind(uid)
    .bind(admin_ua)
    .execute(pool)
    .await
    .expect("grant admin UA");
}

async fn cleanup_sync_fixtures(pool: &PgPool, restore: SystemBinding) {
    for uid in [USER_PRIV_SYNC, USER_PRIV_SYNC_2, USER_UNPRIV_SYNC] {
        sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
            .bind(uid)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
            .bind(uid)
            .execute(pool)
            .await
            .ok();
    }
    // bind_subject 创建的法人行（code 前缀 eb-sync）
    sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE code LIKE 'eb-sync-%'"#)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.zc_id_subjects WHERE id = $1")
        .bind(SUBJ_SYNC_PLACEHOLDER)
        .execute(pool)
        .await
        .ok();
    // 恢复 system 快照（末位执行）
    sqlx::query(
        "UPDATE isahl_auth.auth_users SET entity_table = $1, entity_id = $2, settings = $3 \
         WHERE id = 1",
    )
    .bind(restore.0)
    .bind(restore.1)
    .bind(restore.2)
    .execute(pool)
    .await
    .ok();
}

#[tokio::test]
async fn admin_binding_syncs_system_subject() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;
    let restore = read_system_binding(&pool).await;
    // 上轮残留兜底（法人 code 幂等）
    sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE code LIKE 'eb-sync-%'"#)
        .execute(&pool)
        .await
        .ok();

    for uid in [USER_PRIV_SYNC, USER_PRIV_SYNC_2, USER_UNPRIV_SYNC] {
        ensure_test_user(&pool, uid).await;
    }
    grant_admin_ua(&pool, USER_PRIV_SYNC).await;
    grant_admin_ua(&pool, USER_PRIV_SYNC_2).await;

    // ── 阶段 1：system 占位 + 特权用户 A 绑法人 → 同步 ──
    force_system_placeholder(&pool).await;
    let resp = entity_binding::bind_subject(
        req_with_user(USER_PRIV_SYNC),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_orga-non-banking-legal".to_string(),
            notice: "同步测试法人A".to_string(),
            code: Some("eb-sync-legal-a".to_string()),
            entity_id: None,
            rebind: None,
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "特权用户绑定应成功: {}",
        resp.status()
    );
    let json = json_body(resp).await;
    assert_eq!(
        json["system_subject_synced"].as_bool(),
        Some(true),
        "阶段 1 应发生同步"
    );
    let legal_a: i64 = json["entity_id"]
        .as_str()
        .expect("entity_id")
        .parse()
        .unwrap();
    let expected_sync = USER_PRIV_SYNC.to_string();

    let (table, entity_id, settings) = read_system_binding(&pool).await;
    assert_eq!(table.as_deref(), Some("zc_id_orga-non-banking-legal"));
    assert_eq!(entity_id, Some(legal_a));
    assert!(
        settings
            .as_ref()
            .and_then(|s| s.get("subject_binding"))
            .is_none(),
        "同步后占位标记应清除"
    );
    assert_eq!(
        settings
            .as_ref()
            .and_then(|s| s.get("subject_sync"))
            .and_then(|v| v.as_str()),
        Some(expected_sync.as_str()),
        "溯源标记应为特权用户 A"
    );

    // ── 阶段 2：特权用户 B 再绑 → first-wins 不覆盖 ──
    let resp = entity_binding::bind_subject(
        req_with_user(USER_PRIV_SYNC_2),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_orga-non-banking-legal".to_string(),
            notice: "同步测试法人B".to_string(),
            code: Some("eb-sync-legal-b".to_string()),
            entity_id: None,
            rebind: None,
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "特权用户 B 绑定应成功: {}",
        resp.status()
    );
    let json = json_body(resp).await;
    assert_eq!(
        json["system_subject_synced"].as_bool(),
        Some(false),
        "system 已绑真实主体不应覆盖"
    );
    let (table2, entity_id2, _) = read_system_binding(&pool).await;
    assert_eq!(table2.as_deref(), Some("zc_id_orga-non-banking-legal"));
    assert_eq!(entity_id2, Some(legal_a), "system 应保持第一特权用户的主体");

    // ── 阶段 3：system 回置占位 + 非特权用户绑定 → 不触发同步 ──
    force_system_placeholder(&pool).await;
    let resp = entity_binding::bind_subject(
        req_with_user(USER_UNPRIV_SYNC),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_orga-non-banking-legal".to_string(),
            notice: "同步测试法人C".to_string(),
            code: Some("eb-sync-legal-c".to_string()),
            entity_id: None,
            rebind: None,
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "非特权用户绑定应成功: {}",
        resp.status()
    );
    let json = json_body(resp).await;
    assert_eq!(
        json["system_subject_synced"].as_bool(),
        Some(false),
        "非特权用户不触发同步"
    );
    let (table3, entity_id3, _) = read_system_binding(&pool).await;
    assert_eq!(
        table3.as_deref(),
        Some("zc_id_subjects"),
        "system 应保持占位态"
    );
    assert_eq!(entity_id3, Some(SUBJ_SYNC_PLACEHOLDER));

    cleanup_sync_fixtures(&pool, restore).await;
}

// ── add-subject-rebind-management：个人改换主体 ─────────────────────────────

const USER_REBIND: i64 = -9824;

#[tokio::test]
async fn bind_subject_rebind_replaces_real_binding() {
    let pool = connect_test_db().await;
    setup_binding_fixtures(&pool).await;
    ensure_test_user(&pool, USER_REBIND).await;
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE code LIKE 'eb-rebind-%'"#,
    )
    .execute(&pool)
    .await
    .ok();

    // 首绑法人 A
    let resp = entity_binding::bind_subject(
        req_with_user(USER_REBIND),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_orga-non-banking-legal".to_string(),
            notice: "改绑测试法人A".to_string(),
            code: Some("eb-rebind-a".to_string()),
            entity_id: None,
            rebind: None,
        }),
    )
    .await;
    assert!(resp.status().is_success(), "首绑应成功: {}", resp.status());
    let json = json_body(resp).await;
    let legal_a: i64 = json["entity_id"]
        .as_str()
        .expect("entity_id")
        .parse()
        .unwrap();

    // 缺省再绑 B → ALREADY_BOUND（幂等门不变）
    let resp = entity_binding::bind_subject(
        req_with_user(USER_REBIND),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_orga-non-banking-legal".to_string(),
            notice: "改绑测试法人B".to_string(),
            code: Some("eb-rebind-b".to_string()),
            entity_id: None,
            rebind: None,
        }),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 400, "缺省改绑应 400");
    let json = json_body(resp).await;
    assert_eq!(json["error"].as_str(), Some("ALREADY_BOUND"));

    // rebind=true 绑 B → 锚点切换 + A 行保留（m2o 非破坏）
    let resp = entity_binding::bind_subject(
        req_with_user(USER_REBIND),
        web::Data::new(pool.clone()),
        web::Json(entity_binding::SubjectBindingBody {
            subject_type: "zc_id_orga-non-banking-legal".to_string(),
            notice: "改绑测试法人B".to_string(),
            code: Some("eb-rebind-b".to_string()),
            entity_id: None,
            rebind: Some(true),
        }),
    )
    .await;
    assert!(
        resp.status().is_success(),
        "显式改绑应成功: {}",
        resp.status()
    );
    let json = json_body(resp).await;
    let legal_b: i64 = json["entity_id"]
        .as_str()
        .expect("entity_id")
        .parse()
        .unwrap();
    assert_ne!(legal_a, legal_b);

    let (table, entity_id): (String, Option<i64>) =
        sqlx::query_as("SELECT entity_table, entity_id FROM isahl_auth.auth_users WHERE id = $1")
            .bind(USER_REBIND)
            .fetch_one(&pool)
            .await
            .expect("user");
    assert_eq!(table, "zc_id_orga-non-banking-legal");
    assert_eq!(entity_id, Some(legal_b), "锚点应切换至 B");

    let a_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1)"#,
    )
    .bind(legal_a)
    .fetch_one(&pool)
    .await
    .unwrap_or(false);
    assert!(a_exists, "旧主体行应保留（m2o 非破坏）");

    sqlx::query(
        r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE code LIKE 'eb-rebind-%'"#,
    )
    .execute(&pool)
    .await
    .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(USER_REBIND)
        .execute(&pool)
        .await
        .ok();
}
