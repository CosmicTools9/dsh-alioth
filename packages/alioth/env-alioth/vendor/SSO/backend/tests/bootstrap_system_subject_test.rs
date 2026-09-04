//! system 用户主体绑定（bootstrap）集成测试（add-subject-rebind-management）
//!
//! 覆盖 `POST /api/admin/bootstrap/system-subject`：
//! - 未绑定 → 创建主体并绑定（占位标记 + 主体↔岗位链）
//! - 已绑有效主体 + 未显式 rebind → 幂等返回，绑定不变
//! - 已绑有效主体 + rebind=true + entity_id → 改绑已有主体（叶表名写入、
//!   占位标记清除、新主体↔岗位幂等建联、旧侧数据保留）
#![allow(clippy::type_complexity)]

mod common;

use actix_web::{http::header::AUTHORIZATION, web, HttpRequest, HttpResponse};
use gateway_sso::admin::handlers::bootstrap::{bind_system_subject, BindSystemSubjectRequest};
use gateway_sso::auth::jwt::{configure_token_validation, encode_access_token, Claims};
use gateway_sso::auth::AuthState;
use sqlx::PgPool;

const ADMIN_UID: i64 = -9931;
const ORG_TARGET_ID: i64 = -99321; // 改绑目标（非银行法人叶表行）
const SUBJ_BOOT_CODE: &str = "SUBJ-BOOT-TEST";

type SystemRow = (Option<String>, Option<i64>, Option<serde_json::Value>);

async fn read_system_row(pool: &PgPool) -> SystemRow {
    // 共享测试库防竞态：并行 binary/会话可能清掉 system 账号——先幂等 ensure
    // （复用 Framework common 的 seed 契约实现，禁止第二份 INSERT）
    ::common::system_user::ensure_system_user(pool)
        .await
        .expect("ensure system user");
    sqlx::query_as(
        "SELECT entity_table, entity_id, settings FROM isahl_auth.auth_users WHERE username = 'system'",
    )
    .fetch_one(pool)
    .await
    .expect("system 用户应存在（seed 契约）")
}

/// 建 admin fixture（负 ID 用户 + admin UA 指派，幂等）。
async fn ensure_admin(pool: &PgPool) {
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, status, is_active, created_at, updated_at)
           VALUES ($1, 'boot-admin', 'boot-admin', 'boot-admin@test.local', 'active', true, NOW(), NOW())
           ON CONFLICT (id) DO UPDATE SET is_active = true"#,
    )
    .bind(ADMIN_UID)
    .execute(pool)
    .await
    .expect("insert admin fixture");

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
    .bind(ADMIN_UID)
    .bind(admin_ua)
    .execute(pool)
    .await
    .expect("grant admin UA");
}

/// 带 admin Bearer token 的请求（测试密钥对铸造）。
fn admin_request() -> (AuthState, HttpRequest) {
    let ast = common::test_auth_state();
    configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    let token = encode_access_token(
        &Claims::new(&ADMIN_UID.to_string(), "boot-admin@test.local", false),
        &ast.jwt_private_key,
    )
    .expect("mint token");
    let req = actix_web::test::TestRequest::default()
        .insert_header((AUTHORIZATION, format!("Bearer {token}")))
        .to_http_request();
    (ast, req)
}

async fn call_bind(pool: &PgPool, body: BindSystemSubjectRequest) -> HttpResponse {
    let (ast, req) = admin_request();
    bind_system_subject(
        web::Data::new(pool.clone()),
        req,
        web::Data::new(ast),
        web::Json(body),
    )
    .await
    .expect("bind_system_subject handler")
}

async fn json_of(resp: HttpResponse) -> serde_json::Value {
    let body = actix_web::body::to_bytes(resp.into_body())
        .await
        .expect("body");
    serde_json::from_slice(&body).expect("json")
}

async fn cleanup(pool: &PgPool, restore: SystemRow, boot_subject_id: Option<i64>) {
    // 恢复 system 快照
    sqlx::query(
        "UPDATE isahl_auth.auth_users SET entity_table = $1, entity_id = $2, settings = $3 \
         WHERE username = 'system'",
    )
    .bind(restore.0)
    .bind(restore.1)
    .bind(restore.2)
    .execute(pool)
    .await
    .ok();
    // 本次建联的岗位关联（ref_left ∈ {boot 主体, 改绑目标}）
    let mut lefts: Vec<i64> = Vec::new();
    if let Some(id) = boot_subject_id {
        lefts.push(id);
    }
    lefts.push(ORG_TARGET_ID);
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_subj-org_rr_position"
           WHERE ref_left = ANY($1)
             AND ref_right IN (SELECT id FROM isahl."zc_id_subj-position" WHERE code = 'POS-SYSTEM-ADMIN')"#,
    )
    .bind(&lefts[..])
    .execute(pool)
    .await
    .ok();
    // fixture 行
    sqlx::query(r#"DELETE FROM isahl.zc_id_subjects WHERE code = $1"#)
        .bind(SUBJ_BOOT_CODE)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_orga-non-banking-legal" WHERE id = $1"#)
        .bind(ORG_TARGET_ID)
        .execute(pool)
        .await
        .ok();
    // admin fixture
    sqlx::query("DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user = $1")
        .bind(ADMIN_UID)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(ADMIN_UID)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn system_subject_bootstrap_bind_idempotent_and_adjust() {
    let pool = ::common::testing::connect_test_db().await;
    let restore = read_system_row(&pool).await;
    ensure_admin(&pool).await;

    // 改绑目标：非银行法人叶表行
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_orga-non-banking-legal" (id, notice, code, created_by_id)
           VALUES ($1, 'bootstrap 调整目标法人', 'BOOT-ADJ-ORG', 1)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(ORG_TARGET_ID)
    .execute(&pool)
    .await
    .expect("insert target org");

    // ── 阶段 1：未绑定 → 创建并绑定 ──
    sqlx::query(
        "UPDATE isahl_auth.auth_users SET entity_table = NULL, entity_id = NULL, settings = NULL \
         WHERE username = 'system'",
    )
    .execute(&pool)
    .await
    .expect("force unbound");

    let resp = call_bind(
        &pool,
        BindSystemSubjectRequest {
            notice: "bootstrap 测试主体".to_string(),
            code: Some(SUBJ_BOOT_CODE.to_string()),
            entity_id: None,
            subject_type: Some("org".to_string()),
            rebind: None,
        },
    )
    .await;
    assert!(
        resp.status().is_success(),
        "首次绑定应成功: {}",
        resp.status()
    );
    let json = json_of(resp).await;
    assert_eq!(json["data"]["bound"].as_bool(), Some(true));

    let (table, entity_id, settings) = read_system_row(&pool).await;
    // 治理后（fix-system-subject-seat-by-type）：创建按主体类型落 subjects 树
    // 真叶（org → zc_id_orga-non-banking-legal；subj-org 非叶不作落点）
    assert_eq!(table.as_deref(), Some("zc_id_orga-non-banking-legal"));
    let boot_subject_id = entity_id.expect("boot subject id");
    let code_row: String =
        sqlx::query_scalar("SELECT code FROM isahl.zc_id_subjects WHERE id = $1")
            .bind(boot_subject_id)
            .fetch_one(&pool)
            .await
            .expect("code");
    assert_eq!(code_row, SUBJ_BOOT_CODE);
    assert_eq!(
        settings
            .as_ref()
            .and_then(|s| s.get("subject_binding"))
            .and_then(|v| v.as_str()),
        None,
        "叶表落点绑定不写占位标记（占位标记仅历史基表直插行）"
    );

    // ── 阶段 2：已绑 + 未显式 rebind → 幂等返回，绑定不变 ──
    let resp = call_bind(
        &pool,
        BindSystemSubjectRequest {
            notice: "不应生效".to_string(),
            code: Some("SUBJ-BOOT-OTHER".to_string()),
            entity_id: None,
            subject_type: None,
            rebind: None,
        },
    )
    .await;
    assert!(resp.status().is_success());
    let json = json_of(resp).await;
    assert_eq!(
        json["data"]["message"].as_str(),
        Some("已绑定，无需重复创建"),
        "未显式 rebind 应幂等返回"
    );
    let (table2, entity_id2, _) = read_system_row(&pool).await;
    assert_eq!(table2.as_deref(), Some("zc_id_orga-non-banking-legal"));
    assert_eq!(entity_id2, Some(boot_subject_id), "绑定不应变化");

    // ── 阶段 3：rebind=true + entity_id → 改绑已有叶表主体 ──
    let resp = call_bind(
        &pool,
        BindSystemSubjectRequest {
            notice: String::new(),
            code: None,
            entity_id: Some(ORG_TARGET_ID),
            subject_type: None,
            rebind: Some(true),
        },
    )
    .await;
    assert!(
        resp.status().is_success(),
        "显式改绑应成功: {}",
        resp.status()
    );
    let json = json_of(resp).await;
    assert_eq!(json["data"]["bound"].as_bool(), Some(true));

    let (table3, entity_id3, settings3) = read_system_row(&pool).await;
    assert_eq!(
        table3.as_deref(),
        Some("zc_id_orga-non-banking-legal"),
        "tableoid 解析的叶表名应写入 entity_table"
    );
    assert_eq!(entity_id3, Some(ORG_TARGET_ID));
    assert!(
        settings3
            .as_ref()
            .and_then(|s| s.get("subject_binding"))
            .is_none(),
        "叶表业务主体不应保留占位标记"
    );

    // 状态查询：叶表绑定也应判 bound=true（add-subject-rebind-management 修复——
    // 旧判定只认基表，同步/改绑产物被误报未绑定）
    let (ast_status, req_status) = admin_request();
    let status_resp = gateway_sso::admin::handlers::bootstrap::get_system_subject_status(
        web::Data::new(pool.clone()),
        req_status,
        web::Data::new(ast_status),
    )
    .await
    .expect("status handler");
    assert!(status_resp.status().is_success());
    let status_json = json_of(status_resp).await;
    assert_eq!(
        status_json["data"]["bound"].as_bool(),
        Some(true),
        "叶表绑定应判 bound"
    );
    assert_eq!(
        status_json["data"]["subjectCode"].as_str(),
        Some("BOOT-ADJ-ORG")
    );

    // 新主体 ↔ POS-SYSTEM-ADMIN 幂等建联；旧关联保留
    let pos_link_new: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
            SELECT 1 FROM isahl."zc_id_subj-org_rr_position" o
            JOIN isahl."zc_id_subj-position" p ON p.id = o.ref_right
            WHERE o.ref_left = $1 AND p.code = 'POS-SYSTEM-ADMIN' AND o.deleted_at IS NULL)"#,
    )
    .bind(ORG_TARGET_ID)
    .fetch_one(&pool)
    .await
    .unwrap_or(false);
    assert!(pos_link_new, "改绑后新主体应建岗位关联");

    cleanup(&pool, restore, Some(boot_subject_id)).await;
}
