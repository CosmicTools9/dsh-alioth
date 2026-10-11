//! 分级（密级）上限在 **SSO 决策面**的集成测试（NGAC_SPEC §2.7，D-1b）。
//!
//! 覆盖两条 SSO 消费位的**标签数据路径**（Framework 内层门由
//! `Framework/backend/common/tests/ngac_label_ceiling_test.rs` 覆盖）：
//! - `check_access`（→ `decide_access`，唯一语义源）：白名单 + 资源标签 + clearance 比较 ⇒ Deny / Permit；
//! - `list_resource_access`（RLS 列表）：同一规则逐行扣减 ⇒ 行不可见 / 可见。
//!
//! 夹具自建自清（前缀 `zzlcsso`），可重复运行。

mod common;

use ::common::testing::connect_test_db;
use actix_web::body::MessageBody;
use actix_web::web;
use gateway_sso::auth::jwt::Claims;
use gateway_sso::ngac::pdp::{check_access, list_resource_access, PdpCheckRequest};
use ngac_contract::PdpListRequest;
use sqlx::PgPool;

const RT: &str = "zzlcsso_res";
const FK: i64 = 880001;
const RT_FREE: &str = "zzlcsso_free";
const FK_FREE: i64 = 880002;
const LABEL_LOW: &str = "zzlcsso_rank_low";
const LABEL_HIGH: &str = "zzlcsso_rank_high";
/// 不存在于 `org_policy_label` 的标签 code（悬空标签：行为等同「无标签」⇒ 上限不介入）。
const LABEL_DANGLING: &str = "zzlcsso_rank_missing";
const FK_DANGLING: i64 = 880003;
const WL_CLASS: &str = "zzlcsso_wl";

struct Fixture {
    low_user: i64,
    high_user: i64,
    admin_user: i64,
}

async fn cleanup(pool: &PgPool) {
    for sql in [
        "DELETE FROM isahl_auth.ngac_prohibition WHERE o_name LIKE 'zzlcsso%'",
        "DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute IN \
           (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zzlcsso%')",
        "DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user IN \
           (SELECT id FROM isahl_auth.auth_users WHERE username LIKE 'zzlcsso%')",
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type IN ('zzlcsso_res','zzlcsso_free')",
        "DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'zzlcsso%'",
        "DELETE FROM isahl_auth.org_policy_class WHERE code LIKE 'zzlcsso%'",
        "DELETE FROM isahl_auth.org_policy_label WHERE code LIKE 'zzlcsso%'",
        "DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zzlcsso%'",
    ] {
        sqlx::query(sql)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("cleanup {sql}: {e}"));
    }
}

async fn default_pc(pool: &PgPool) -> i64 {
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_policy_class (o_name) VALUES ('default') ON CONFLICT DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("ensure default pc");
    sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name='default' LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("default pc")
}

/// 建「用户 + 带 clearance 的 UA + 绑定」，返回 (user_id, ua_id)。
async fn mk_subject(pool: &PgPool, pc: i64, slug: &str, clearance: &str) -> (i64, i64) {
    let username = format!("zzlcsso_{slug}");
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, status, created_at, updated_at) \
         VALUES (isahl.gen_next_zuid(), $1, 'active', NOW(), NOW()) \
         ON CONFLICT (username) DO NOTHING",
    )
    .bind(&username)
    .execute(pool)
    .await
    .expect("insert user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username=$1 LIMIT 1")
            .bind(&username)
            .fetch_one(pool)
            .await
            .expect("user id");
    let ua_name = format!("zzlcsso-ua-{slug}");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, property, created_at, updated_at) \
         VALUES ($1, $2, $3, NOW(), NOW()) \
         ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind(&ua_name)
    .bind(pc)
    .bind(serde_json::json!({ "clearance": clearance }))
    .execute(pool)
    .await
    .expect("insert UA");
    let ua_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name=$1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(&ua_name)
    .fetch_one(pool)
    .await
    .expect("UA id");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at) \
         VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(ua_id)
    .execute(pool)
    .await
    .expect("bind UA");
    (user_id, ua_id)
}

/// 建资源行 OA（property 可带 label）。**手工夹具**：产品路径当前**无实例行标签写径**
/// （§12 登记：派生器只写集合 OA、`ngac_grants`/管理面 OA 端点不写 `property`）
/// —— 本测试用于验证谓词语义，不代表实例标签已可产出。
async fn mk_resource(pool: &PgPool, pc: i64, rt: &str, fk: i64, label: Option<&str>) -> i64 {
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_object_attribute \
            (o_name, fk_policy_class, resource_type, fk_resource, property, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, NOW(), NOW()) \
         ON CONFLICT (resource_type, fk_resource) DO UPDATE SET deleted_at = NULL, property = EXCLUDED.property",
    )
    .bind(format!("{rt}-oa"))
    .bind(pc)
    .bind(rt)
    .bind(fk)
    .bind(label.map(|l| serde_json::json!({ "label": l })))
    .execute(pool)
    .await
    .expect("insert OA");
    sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type=$1 AND fk_resource=$2 LIMIT 1",
    )
    .bind(rt)
    .bind(fk)
    .fetch_one(pool)
    .await
    .expect("OA id")
}

async fn grant_read(pool: &PgPool, pc: i64, ua: i64, oa: i64) {
    grant_rights(pool, pc, ua, oa, &["read"]).await;
}

/// 一次关联携带多个动作（`ngac_association` 对 `(fk_user_attribute, fk_object_attribute,
/// fk_policy_class)` 唯一 ⇒ 同一 UA/OA 只能有一行，动作 MUST 走 `ak_access_rights` 数组）。
async fn grant_rights(pool: &PgPool, pc: i64, ua: i64, oa: i64, rights: &[&str]) {
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = ANY($1)")
            .bind(rights)
            .fetch_all(pool)
            .await
            .expect("AR lookup");
    assert_eq!(
        ids.len(),
        rights.len(),
        "all ARs must be seeded: {rights:?}"
    );
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_association \
            (fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, NOW(), NOW())",
    )
    .bind(ua)
    .bind(oa)
    .bind(&ids)
    .bind(pc)
    .execute(pool)
    .await
    .expect("insert association");
}

/// 建一个绑定到 `default` 策略类 `admin` UA 的管理员用户（**无 clearance**），返回 user_id。
async fn mk_admin_subject(pool: &PgPool, pc: i64) -> i64 {
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, created_at, updated_at) \
         VALUES ('admin', $1, NOW(), NOW()) \
         ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind(pc)
    .execute(pool)
    .await
    .expect("ensure admin UA");
    let admin_ua: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute \
          WHERE o_name='admin' AND fk_policy_class=$1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(pc)
    .fetch_one(pool)
    .await
    .expect("admin UA id");
    let username = "zzlcsso_admin";
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, status, created_at, updated_at) \
         VALUES (isahl.gen_next_zuid(), $1, 'active', NOW(), NOW()) \
         ON CONFLICT (username) DO NOTHING",
    )
    .bind(username)
    .execute(pool)
    .await
    .expect("insert admin user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username=$1 LIMIT 1")
            .bind(username)
            .fetch_one(pool)
            .await
            .expect("admin user id");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at) \
         VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(admin_ua)
    .execute(pool)
    .await
    .expect("bind admin UA");
    user_id
}

/// 白名单类（`org_policy_class.scope.resource_types = [RT]`）+ 两档标签。
async fn mk_whitelist_and_labels(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO isahl_auth.org_policy_class \
            (code, notice, state, version, scope, ua_template, audit_required, created_at, updated_at) \
         VALUES ($1, 'label ceiling itest', 'active', 1, $2, '{}'::jsonb, false, NOW(), NOW())",
    )
    .bind(WL_CLASS)
    .bind(serde_json::json!({ "resource_types": [RT] }))
    .execute(pool)
    .await
    .expect("insert whitelist class");
    for (code, rank) in [(LABEL_LOW, 1), (LABEL_HIGH, 5)] {
        sqlx::query(
            "INSERT INTO isahl_auth.org_policy_label (code, rank, domain, notice, is_active, created_at, updated_at) \
             VALUES ($1, $2, 'data', 'itest', true, NOW(), NOW())",
        )
        .bind(code)
        .bind(rank)
        .execute(pool)
        .await
        .expect("insert label");
    }
}

async fn setup(pool: &PgPool) -> Fixture {
    cleanup(pool).await;
    let pc = default_pc(pool).await;
    mk_whitelist_and_labels(pool).await;
    let (low_user, ua_low) = mk_subject(pool, pc, "low", LABEL_LOW).await;
    let (high_user, ua_high) = mk_subject(pool, pc, "high", LABEL_HIGH).await;
    let oa_secret = mk_resource(pool, pc, RT, FK, Some(LABEL_HIGH)).await;
    let oa_free = mk_resource(pool, pc, RT_FREE, FK_FREE, Some(LABEL_HIGH)).await;
    // 集合级 OA（`RT:0`）：审查的用户视图逐 resource_type 只读集合级 OA（fk_resource=0），
    // 亦使 `RT:0` 的 decide 可用于交叉验证。
    let oa_collection = mk_resource(pool, pc, RT, 0, Some(LABEL_HIGH)).await;
    // 悬空标签行（property 写了一个字典里不存在的 code）⇒ 行为等同无标签（不介入）
    let oa_dangling = mk_resource(pool, pc, RT, FK_DANGLING, Some(LABEL_DANGLING)).await;
    grant_read(pool, pc, ua_low, oa_secret).await;
    grant_read(pool, pc, ua_high, oa_secret).await;
    grant_read(pool, pc, ua_low, oa_free).await;
    grant_read(pool, pc, ua_low, oa_dangling).await;
    // 集合级 OA（`RT:0`）：一次关联携带 `read` + `read:price`（审查用户视图走集合级 OA；
    // 列级授权上限用例需 `read:price`）
    grant_rights(pool, pc, ua_low, oa_collection, &["read", "read:price"]).await;
    grant_rights(pool, pc, ua_high, oa_collection, &["read", "read:price"]).await;
    let admin_user = mk_admin_subject(pool, pc).await;
    // 反向夹具（§2.7「主体 clearance 只取全局有效 UA 集」）：一个持有**高 clearance**、
    // 但**未绑定给任何主体**的 UA（≈ 资源受限的属权派生 UA 形态：存在且带 clearance，但不属主体闭包）
    // ⇒ 它 MUST NOT 抬升任何主体的密级。
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class, property, created_at, updated_at) \
         VALUES ('zzlcsso-ua-decoy', $1, $2, NOW(), NOW()) \
         ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) DO NOTHING",
    )
    .bind(pc)
    .bind(serde_json::json!({ "clearance": LABEL_HIGH }))
    .execute(pool)
    .await
    .expect("insert decoy UA");
    Fixture {
        low_user,
        high_user,
        admin_user,
    }
}

/// 端到端组合（裁决 B-0 后的**唯一**未覆盖链）：canonical 声明 → 派生器产出两侧 carrier → 判定生效。
///
/// 与同文件其余用例的区别：本用例的**标签与 clearance 全部由 `derive_from_class` 产生**（不手写 OA/UA property），
/// 因而验证的是「产品路径」（`class.label_code` → 集合 OA `property.label` + 派生 UA `property.clearance` → 判决）。
#[tokio::test]
async fn grading_end_to_end_from_canonical_declaration() {
    let pool: PgPool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    cleanup_e2e(&pool).await;

    // 两档标签 + 两个 canonical 类（各自白名单自有资源类型；label rank 2 / 5）
    for (code, rank) in [(E2E_LABEL_LO, 2), (E2E_LABEL_HI, 5)] {
        sqlx::query(
            "INSERT INTO isahl_auth.org_policy_label (code, rank, domain, notice, is_active, created_at, updated_at) \
             VALUES ($1, $2, 'data', 'e2e itest', true, NOW(), NOW())",
        )
        .bind(code)
        .bind(rank)
        .execute(&pool)
        .await
        .expect("insert label");
    }
    let pc = default_pc(&pool).await;
    let mut class_ids = Vec::new();
    for (slug, rt, label) in [
        ("lo", E2E_RT_LO, E2E_LABEL_LO),
        ("hi", E2E_RT_HI, E2E_LABEL_HI),
    ] {
        let class_id: i64 = sqlx::query_scalar(
            "INSERT INTO isahl_auth.org_policy_class \
                (code, notice, scope, ua_template, state, label_code, created_at, updated_at) \
             VALUES ($1, $1 || '-notice', $2, '{\"name_rule\":\"position:{code}\"}', 'active', $3, NOW(), NOW()) \
             RETURNING id",
        )
        .bind(format!("zzlc2_{slug}"))
        .bind(serde_json::json!({ "actions": ["read"], "resource_types": [rt] }))
        .bind(label)
        .fetch_one(&pool)
        .await
        .expect("insert class");
        sqlx::query(
            "INSERT INTO isahl_auth.org_policy_rule \
                (policy_class_id, subject_code, resource_type, actions, state, created_at, updated_at) \
             VALUES ($1, 'subj-e2e', $2, '[\"read\"]', 'active', NOW(), NOW())",
        )
        .bind(class_id)
        .bind(rt)
        .execute(&pool)
        .await
        .expect("insert rule");
        class_ids.push(class_id);
    }

    // canonical → 派生器产出**两侧** carrier（不手写任何 property）
    for class_id in &class_ids {
        ::common::ngac_policy::derive_from_class(&pool, *class_id)
            .await
            .expect("derive");
    }
    // 证据①：资源侧标签
    assert_eq!(
        derived_collection_label(&pool, E2E_RT_HI).await.as_deref(),
        Some(E2E_LABEL_HI),
        "派生器 MUST 从 canonical label_code 产出集合 OA 标签"
    );
    // 证据②：主体侧 clearance
    assert_eq!(
        derived_ua_clearance(&pool, "position:zzlc2_lo")
            .await
            .as_deref(),
        Some(E2E_LABEL_LO),
        "裁决 B-0：派生器 MUST 产出派生 UA 的 clearance"
    );

    // 主体：绑定低档 UA（模拟认知绑定 ⇒ 进入 effective_ua_ids），并**额外**给低档主体一条到高档资源的读关联
    //（无关联则判决本就会拒，无法证明「拒因 = 密级上限」）。
    let ua_lo: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'position:zzlc2_lo' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("lo UA");
    let username = "zzlc2_e2e_user";
    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (id, username, status, created_at, updated_at) \
         VALUES (isahl.gen_next_zuid(), $1, 'active', NOW(), NOW()) ON CONFLICT (username) DO NOTHING",
    )
    .bind(username)
    .execute(&pool)
    .await
    .expect("insert user");
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM isahl_auth.auth_users WHERE username=$1 LIMIT 1")
            .bind(username)
            .fetch_one(&pool)
            .await
            .expect("user id");
    sqlx::query(
        "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, created_at, updated_at) \
         VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(ua_lo)
    .execute(&pool)
    .await
    .expect("bind lo UA");
    let oa_hi: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type=$1 AND fk_resource=0 LIMIT 1",
    )
    .bind(E2E_RT_HI)
    .fetch_one(&pool)
    .await
    .expect("hi OA");
    grant_read(&pool, pc, ua_lo, oa_hi).await;

    // 正例：低档主体读低档资源（clearance 2 ≥ label 2）⇒ 放行
    let ok = call_check(&pool, user_id, &format!("{E2E_RT_LO}:0")).await;
    assert_eq!(
        ok["permitted"], true,
        "同档（clearance == label）MUST 放行: {ok}"
    );
    // 反例：同主体读高档资源（clearance 2 < label 5，且**已有读关联**）⇒ 拒绝，且原因指向密级上限
    let denied = call_check(&pool, user_id, &format!("{E2E_RT_HI}:0")).await;
    assert_eq!(
        denied["permitted"], false,
        "跨档（clearance < label）MUST 拒绝（即便有 read 关联）: {denied}"
    );
    assert!(
        denied["reason"]
            .as_str()
            .unwrap_or("")
            .contains("label ceiling"),
        "拒绝原因 MUST 指向密级上限: {denied}"
    );

    cleanup_e2e(&pool).await;
}

/// 端到端用例的专属常量（与其余用例隔离命名）。
const E2E_LABEL_LO: &str = "zzlc2_rank_lo";
const E2E_LABEL_HI: &str = "zzlc2_rank_hi";
const E2E_RT_LO: &str = "zzlc2_res_lo";
const E2E_RT_HI: &str = "zzlc2_res_hi";

/// 读取派生集合 OA（`fk_resource = 0`）上的标签（§2.7 资源侧 carrier）。
async fn derived_collection_label(pool: &PgPool, resource_type: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT property ->> 'label' FROM isahl_auth.ngac_object_attribute \
          WHERE resource_type = $1 AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(resource_type)
    .fetch_one(pool)
    .await
    .expect("read derived collection OA label")
}

/// 读取派生 UA 上的主体 clearance（裁决 B-0 的生产方）。
async fn derived_ua_clearance(pool: &PgPool, ua_name: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT property ->> 'clearance' FROM isahl_auth.ngac_user_attribute \
          WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(ua_name)
    .fetch_one(pool)
    .await
    .expect("read derived UA clearance")
}

/// 端到端用例清理（派生器产物：UA/OA/association + 类/规则/标签 + 用户）。
async fn cleanup_e2e(pool: &PgPool) {
    for sql in [
        "DELETE FROM isahl_auth.ngac_association WHERE fk_user_attribute IN \
           (SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'position:zzlc2%')",
        "DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute IN \
           (SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type IN ('zzlc2_res_lo','zzlc2_res_hi'))",
        "DELETE FROM isahl_auth.ngac_user_rr_attribute WHERE fk_user IN \
           (SELECT id FROM isahl_auth.auth_users WHERE username LIKE 'zzlc2_%')",
        "DELETE FROM isahl_auth.ngac_object_attribute WHERE resource_type IN ('zzlc2_res_lo','zzlc2_res_hi')",
        "DELETE FROM isahl_auth.ngac_user_attribute WHERE o_name LIKE 'position:zzlc2%'",
        "DELETE FROM isahl_auth.org_policy_rule WHERE resource_type IN ('zzlc2_res_lo','zzlc2_res_hi')",
        "DELETE FROM isahl_auth.org_policy_class WHERE code LIKE 'zzlc2_%'",
        "DELETE FROM isahl_auth.org_policy_label WHERE code LIKE 'zzlc2_%'",
        "DELETE FROM isahl_auth.auth_users WHERE username LIKE 'zzlc2_%'",
    ] {
        sqlx::query(sql)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("cleanup_e2e {sql}: {e}"));
    }
}

async fn call_check(pool: &PgPool, user_id: i64, resource: &str) -> serde_json::Value {
    use actix_web::HttpMessage as _;
    let req = actix_web::test::TestRequest::post()
        .insert_header(("Authorization", "Bearer test-token"))
        .to_http_request();
    req.extensions_mut()
        .insert(Claims::new(&user_id.to_string(), "", false));
    let resp = check_access(
        req,
        web::Data::new(pool.clone()),
        web::Json(PdpCheckRequest {
            user_id,
            resource: resource.to_string(),
            action: "read".to_string(),
        }),
    )
    .await;
    let bytes = resp.into_body().try_into_bytes().unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn call_list(pool: &PgPool, user_id: i64, resource_type: &str) -> serde_json::Value {
    use actix_web::HttpMessage as _;
    let req = actix_web::test::TestRequest::get()
        .insert_header(("Authorization", "Bearer test-token"))
        .to_http_request();
    req.extensions_mut()
        .insert(Claims::new(&user_id.to_string(), "", false));
    let resp = list_resource_access(
        req,
        web::Data::new(pool.clone()),
        web::Json(PdpListRequest {
            user_id,
            resource_type: resource_type.to_string(),
            action: "read".to_string(),
        }),
    )
    .await;
    let bytes = resp.into_body().try_into_bytes().unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn call_columns(pool: &PgPool, user_id: i64, resource_type: &str) -> Vec<String> {
    use actix_web::HttpMessage as _;
    let req = actix_web::test::TestRequest::post()
        .insert_header(("Authorization", "Bearer test-token"))
        .to_http_request();
    req.extensions_mut()
        .insert(Claims::new(&user_id.to_string(), "", false));
    let resp = gateway_sso::ngac::pdp::list_column_access(
        req,
        web::Data::new(pool.clone()),
        web::Json(ngac_contract::PdpColumnsRequest {
            user_id,
            resource_type: resource_type.to_string(),
        }),
    )
    .await;
    let bytes = resp.into_body().try_into_bytes().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["columns"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| c.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn visible_ids(body: &serde_json::Value) -> Vec<i64> {
    body["visible_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    v.as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| v.as_i64())
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn sso_decision_and_rls_respect_label_ceiling() {
    let pool: PgPool = connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup schema");
    let f = setup(&pool).await;
    let secret = format!("{RT}:{FK}");
    let free = format!("{RT_FREE}:{FK_FREE}");

    // 1) 低 clearance + 白名单 + 资源标签 ⇒ Deny（即使存在 read 关联）
    let d = call_check(&pool, f.low_user, &secret).await;
    assert_eq!(
        d["permitted"], false,
        "clearance(1) < label(5) MUST deny: {d}"
    );
    assert!(
        d["reason"].as_str().unwrap_or("").contains("label ceiling"),
        "拒绝原因 MUST 可区分密级上限与 prohibition: {d}"
    );
    // 2) 等高 clearance ⇒ Permit（关联命中）
    let d = call_check(&pool, f.high_user, &secret).await;
    assert_eq!(
        d["permitted"], true,
        "clearance(5) == label(5) MUST pass: {d}"
    );
    // 3) 非白名单资源类型 ⇒ 谓词不介入 ⇒ Permit
    let d = call_check(&pool, f.low_user, &free).await;
    assert_eq!(
        d["permitted"], true,
        "non-whitelisted resource type MUST be unaffected: {d}"
    );

    // 4) RLS 列表同规则：低 clearance ⇒ 行不可见；等高 ⇒ 可见
    let low_ids = visible_ids(&call_list(&pool, f.low_user, RT).await);
    assert!(
        !low_ids.contains(&FK),
        "low clearance MUST NOT see labeled row: {low_ids:?}"
    );
    let high_ids = visible_ids(&call_list(&pool, f.high_user, RT).await);
    assert!(
        high_ids.contains(&FK),
        "sufficient clearance MUST see labeled row: {high_ids:?}"
    );
    // 5) 非白名单类型 ⇒ 列表不受影响
    let free_ids = visible_ids(&call_list(&pool, f.low_user, RT_FREE).await);
    assert!(
        free_ids.contains(&FK_FREE),
        "non-whitelisted type MUST be unaffected: {free_ids:?}"
    );

    // 6) explain 与 decide 一致（同一判据，禁「explain 说 Permit、真决策 Deny」）
    use gateway_sso::auth::jwt::encode_access_token;
    let ast = common::test_auth_state();
    gateway_sso::auth::jwt::configure_token_validation(
        "http://localhost:9002".to_string(),
        "http://localhost:9002".to_string(),
    );
    let token = encode_access_token(
        &Claims::new(&f.low_user.to_string(), "", false),
        &ast.jwt_private_key,
    )
    .expect("mint token");
    let req = actix_web::test::TestRequest::post()
        .insert_header(("Authorization", format!("Bearer {token}")))
        .to_http_request();
    let resp = gateway_sso::ngac::pdp::ngac_decide_explain_self(
        req,
        web::Data::new(pool.clone()),
        web::Data::new(ast.clone()),
        web::Json(gateway_sso::ngac::pdp::SelfExplainRequest {
            resource: secret.clone(),
            action: "read".to_string(),
        }),
    )
    .await;
    let bytes = resp.into_body().try_into_bytes().unwrap();
    let ex: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        ex["permitted"], false,
        "explain MUST 与 decide 一致（密级上限）: {ex}"
    );
    assert!(
        ex["reason"]
            .as_str()
            .unwrap_or("")
            .contains("label ceiling"),
        "explain 拒绝原因 MUST 指向密级上限: {ex}"
    );

    // 7) 矩阵与审查视图同判据（命中 ⇒ 以 denied 呈现，effective/allowed 不含该项）
    use gateway_sso::ngac::pdp::Pdp;
    use gateway_sso::ngac::pip::PostgresPip;
    let pip = PostgresPip::new(pool.clone());
    let pdp = Pdp::global();
    let pc_id = default_pc(&pool).await;
    let m = pdp.policy_matrix(&pip, pc_id).await.expect("policy matrix");
    let (low_ua, oa_secret) = cell_ids(&pool, f.low_user, RT, FK).await;
    let cell = m
        .cells
        .iter()
        .find(|c| c.ua_id == low_ua && c.oa_id == oa_secret)
        .expect("low UA × labeled OA cell");
    assert!(
        cell.denied.iter().any(|r| r == "read"),
        "矩阵 MUST 以 denied 呈现密级上限: {:?}",
        cell.denied
    );
    assert!(
        !cell.effective.iter().any(|r| r == "read"),
        "矩阵 effective MUST NOT 含被密级上限拒绝的项: {:?}",
        cell.effective
    );

    let user_review = pdp
        .user_access_review(&pip, f.low_user)
        .await
        .expect("user review");
    let row = user_review
        .permissions
        .iter()
        .find(|p| p.resource_type == RT)
        .expect("RT permission row");
    assert!(
        row.denied.iter().any(|r| r == "read"),
        "用户审查 MUST 以 denied 呈现密级上限: {:?}",
        row.denied
    );
    assert!(
        !row.allowed.iter().any(|r| r == "read"),
        "用户审查 allowed MUST NOT 含被密级上限拒绝的项: {:?}",
        row.allowed
    );
    // 正对照：等高 clearance 主体在集合级 OA 上仍为 allowed
    let high_review = pdp
        .user_access_review(&pip, f.high_user)
        .await
        .expect("high user review");
    let high_row = high_review
        .permissions
        .iter()
        .find(|p| p.resource_type == RT)
        .expect("RT permission row (high)");
    assert!(
        high_row.allowed.iter().any(|r| r == "read"),
        "等高 clearance MUST 保持 allowed: {:?}",
        high_row.allowed
    );

    let res_review = pdp
        .resource_access_review(&pip, RT, FK)
        .await
        .expect("resource review");
    let holder = res_review
        .holders
        .iter()
        .find(|h| h.ua_id == low_ua)
        .expect("low UA holder row");
    assert!(
        holder.denied.iter().any(|r| r == "read"),
        "资源审查 MUST 以 denied 呈现密级上限: {:?}",
        holder.denied
    );

    // 8) 列级授权同样受上限约束（元数据旁路禁止）
    let low_cols = call_columns(&pool, f.low_user, RT).await;
    assert!(
        low_cols.is_empty(),
        "低 clearance MUST NOT 见列级授权: {low_cols:?}"
    );
    let high_cols = call_columns(&pool, f.high_user, RT).await;
    assert!(
        high_cols.iter().any(|c| c == "price"),
        "等高 clearance MUST 见 read:price（列名）: {high_cols:?}"
    );

    // 9) admin 同样受上限约束（§2.7，与 prohibition 同族）：白名单类型 MUST NOT 以 None（全量）规避
    let admin_body = call_list(&pool, f.admin_user, RT).await;
    assert!(
        !admin_body["visible_ids"].is_null(),
        "白名单资源类型 admin MUST NOT 返回全量可见（None）: {admin_body}"
    );
    assert!(
        !visible_ids(&admin_body).contains(&FK),
        "admin（无 clearance）MUST NOT 看到高于自身密级的行: {admin_body}"
    );
    // 非白名单类型：admin 维持 §6.2 全量语义（None）
    let admin_free = call_list(&pool, f.admin_user, RT_FREE).await;
    assert!(
        admin_free["visible_ids"].is_null(),
        "非白名单类型 admin MUST 维持全量不过滤: {admin_free}"
    );

    // 10) 反向边界（§2.7）：存在带高 clearance 的 UA，但**不属主体有效闭包** ⇒ MUST NOT 抬升密级
    let decoy_in_closure: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM isahl_auth.effective_ua_ids($1) e \
           JOIN isahl_auth.ngac_user_attribute ua ON ua.id = e.ua_id \
          WHERE ua.o_name = 'zzlcsso-ua-decoy' AND ua.deleted_at IS NULL)",
    )
    .bind(f.low_user)
    .fetch_one(&pool)
    .await
    .expect("decoy closure probe");
    assert!(
        !decoy_in_closure,
        "夹具前提：decoy UA MUST NOT 属低密级主体闭包（否则本断言无意义）"
    );
    let d = call_check(&pool, f.low_user, &secret).await;
    assert_eq!(
        d["permitted"], false,
        "闭包外的高 clearance UA（属权派生形态）MUST NOT 抬升主体密级: {d}"
    );

    // 11) 悬空标签（§2.7，行为等同无标签 ⇒ 不介入）：白名单类型 + property.label 指向字典外 code ⇒ 放行
    let d = call_check(&pool, f.low_user, &format!("{RT}:{FK_DANGLING}")).await;
    assert_eq!(
        d["permitted"], true,
        "悬空标签行 MUST 等同无标签（不介入）而非误拒: {d}"
    );

    cleanup(&pool).await;
}

/// 取夹具的 (low UA id, 目标 OA id)。
async fn cell_ids(pool: &PgPool, user_id: i64, rt: &str, fk: i64) -> (i64, i64) {
    let ua: i64 = sqlx::query_scalar(
        "SELECT fk_user_attribute FROM isahl_auth.ngac_user_rr_attribute \
          WHERE fk_user = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .expect("low UA id");
    let oa: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1 AND fk_resource = $2 LIMIT 1",
    )
    .bind(rt)
    .bind(fk)
    .fetch_one(pool)
    .await
    .expect("OA id");
    (ua, oa)
}
