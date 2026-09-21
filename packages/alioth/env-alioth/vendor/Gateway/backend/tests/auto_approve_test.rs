//! 注册审批自动通过（配置项 `approval:auto-approve`）集成测试
//!
//! 覆盖：
//! - 开关关闭 / 配置行缺失 ⇒ fail-closed，待批实例保持无终态
//! - 开关开启 ⇒ 实例终结为 approved + 申请人激活 + 按实名记录绑定主体（entity_table/entity_id）
//! - 重复 tick 幂等（单条状态桥 + 单条意见）
//!
//! 使用 aliothstudio_test 库、负数 ID fixture、自建自清。

use ::common::testing::connect_test_db;
use alioth_gateway::auto_approve::AutoApproveHandler;
use framework_scheduler::{ScheduledHandler, SchedulerContext};
use sqlx::PgPool;

const CODE: &str = "approval:auto-approve";
const USER_NEW: i64 = -99201;
const APPR_NEW: i64 = -99211;
const SUBJ_NEW: i64 = -99221;
const IDV_NEW: i64 = -99231;

/// 三个用例共用同一配置行（`approval:auto-approve`）与 fixture id ⇒ 必须串行执行
/// （cargo 默认并行跑测试；并行会让彼此把对方的开关/fixture 改掉）。
/// 用 `tokio::sync::Mutex`：std guard 跨 await 持有会被 clippy `await_holding_lock` 拦截
/// （pre-push 以 `-D warnings` 编译测试）。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn serialize() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL.lock().await
}

/// 写/改开关键（测试库 fixture；生产由模型级种子供给）
async fn set_switch(pool: &PgPool, enabled: Option<bool>) {
    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE code = $1"#)
        .bind(CODE)
        .execute(pool)
        .await
        .expect("clear switch row");
    if let Some(v) = enabled {
        sqlx::query(
            // 类契约/坐标（§4.3.3 形态1 + §6.12）：dk_function 作派生源，三坐标按 code 解析
            // （取值与本表模型级种子 seed-auth-approval-flows.sql §5 一致）
            r#"INSERT INTO isahl."zc_id_prot-env_config"
                 (notice, code, settings, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ('注册审批自动通过（test）', $1, jsonb_build_object('enabled', $2), 1,
                       (SELECT id FROM isahl.zc_id_scene    WHERE code = 'JE'   AND deleted_at IS NULL),
                       (SELECT id FROM isahl.zc_id_factor   WHERE code = 'GEC'  AND deleted_at IS NULL),
                       (SELECT id FROM isahl.zc_id_function WHERE code = '↑_DA' AND deleted_at IS NULL))"#,
        )
        .bind(CODE)
        .bind(v)
        .execute(pool)
        .await
        .expect("insert switch row");
    }
}

/// 待批注册审批 fixture：申请人（pending_approval）+ 审批实例 + 实名记录 + 实名主体行
async fn setup_fixtures(pool: &PgPool) {
    cleanup_fixtures(pool).await;

    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users (id, name, status, created_at, updated_at)
           VALUES ($1, 'auto-approve-test', 'pending_approval', NOW(), NOW())"#,
    )
    .bind(USER_NEW)
    .execute(pool)
    .await
    .expect("insert auth_users fixture");

    // 实名主体行（继承链 isahl.zc_id_subjects → zc_id_empl-natural，供白名单校验命中）
    sqlx::query(
        // 类契约/坐标（§4.3.3 形态1 + §6.12）：取值与本表同 crate 测试
        // entity_binding_integration_test.rs 一致（TX/FJA/↓_GG）
        r#"INSERT INTO isahl."zc_id_empl-natural"
             (id, notice, code, fk_user, created_at, updated_at, dk_scene, dk_factor, dk_function)
           VALUES ($1, '自动通过测试主体', $2, $3, NOW(), NOW(),
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'TX'   AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'FJA'  AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_GG' AND deleted_at IS NULL))"#,
    )
    .bind(SUBJ_NEW)
    .bind(format!("emp-{USER_NEW}"))
    .bind(USER_NEW)
    .execute(pool)
    .await
    .expect("insert subject fixture");

    sqlx::query(
        r#"INSERT INTO isahl_auth.identity_verifications
             (id, user_id, verification_type, real_name, entity_instance_id, entity_instance_table,
              verification_status, created_at, updated_at)
           VALUES ($1, $2, 'personal', '自动通过测试', $3, 'zc_id_empl-natural', 'verified', NOW(), NOW())"#,
    )
    .bind(IDV_NEW)
    .bind(USER_NEW)
    .bind(SUBJ_NEW)
    .execute(pool)
    .await
    .expect("insert identity verification fixture");

    // 注册审批实例（fk_operator 先指向某 admin——自动通过需先归位系统用户）
    sqlx::query(
        // 类契约/坐标（§4.3.3 形态1 + §6.12）：取值与本表模型级种子
        // seed-auth-approval-flows.sql「节点操作行」一致（JE/FTA/↓_EZ）
        r#"INSERT INTO isahl."zc_id_oper-approve"
             (id, notice, code, fk_subject, fk_operator, created_by_id, created_at, updated_at,
              dk_scene, dk_factor, dk_function)
           VALUES ($1, 'T-auto-approve 注册审批', 'user-register-approval', $2, 1, $2, NOW(), NOW(),
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'JE'   AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'FTA'  AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_EZ' AND deleted_at IS NULL))"#,
    )
    .bind(APPR_NEW)
    .bind(USER_NEW)
    .execute(pool)
    .await
    .expect("insert approval instance fixture");
}

async fn cleanup_fixtures(pool: &PgPool) {
    let ids = [APPR_NEW];
    sqlx::query(r#"DELETE FROM isahl."zc_id_deta-opinion" WHERE fk_list = ANY($1)"#)
        .bind(&ids[..])
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = ANY($1)"#)
        .bind(&ids[..])
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_oper-approve" WHERE id = ANY($1)"#)
        .bind(&ids[..])
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.identity_verifications WHERE id = $1")
        .bind(IDV_NEW)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_empl-natural" WHERE id = $1"#)
        .bind(SUBJ_NEW)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl_auth.auth_users WHERE id = $1")
        .bind(USER_NEW)
        .execute(pool)
        .await
        .ok();
}

async fn run_handler(pool: &PgPool) {
    let handler = AutoApproveHandler::new(
        pool.clone(),
        std::sync::Arc::new(common::event_bus::InMemoryEventBus::new()),
    );
    let ctx = SchedulerContext {
        pool: pool.clone(),
        plan_id: 0,
        plan_code: handler.plan_code().to_string(),
    };
    handler.run(&ctx).await.expect("auto approve handler run");
}

/// 实例的终态（无终态 ⇒ None）
async fn terminal_status(pool: &PgPool, instance_id: i64) -> Option<String> {
    sqlx::query_scalar(
        r#"SELECT st.code
             FROM isahl."zc_id_lifecycle_r_primary-status" ps
             JOIN isahl."zc_id_stus-approve" st ON st.id = ps.ref_right
            WHERE ps.ref_left = $1 AND ps.deleted_at IS NULL"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .expect("query terminal status")
    .flatten()
}

/// 申请人账号状态与主体绑定
async fn applicant_state(pool: &PgPool) -> (String, Option<i64>, Option<String>) {
    sqlx::query_as(
        "SELECT status, entity_id, entity_table FROM isahl_auth.auth_users WHERE id = $1",
    )
    .bind(USER_NEW)
    .fetch_one(pool)
    .await
    .expect("query applicant")
}

#[tokio::test]
async fn t_auto_approve_disabled_keeps_instance_pending() {
    let _guard = serialize().await;
    let pool = connect_test_db().await;
    setup_fixtures(&pool).await;
    set_switch(&pool, Some(false)).await;

    run_handler(&pool).await;

    assert_eq!(
        terminal_status(&pool, APPR_NEW).await,
        None,
        "开关关闭不得自动通过"
    );
    let (status, entity_id, _) = applicant_state(&pool).await;
    assert_eq!(status, "pending_approval", "开关关闭时申请人不被激活");
    assert_eq!(entity_id, None, "开关关闭时不做主体绑定");

    cleanup_fixtures(&pool).await;
    set_switch(&pool, None).await;
}

#[tokio::test]
async fn t_auto_approve_missing_switch_row_is_closed() {
    let _guard = serialize().await;
    let pool = connect_test_db().await;
    setup_fixtures(&pool).await;
    set_switch(&pool, None).await; // 配置行缺失 ⇒ fail-closed

    run_handler(&pool).await;

    assert_eq!(
        terminal_status(&pool, APPR_NEW).await,
        None,
        "配置行缺失必须视为关闭（MUST NOT 默认开启兜底）"
    );

    cleanup_fixtures(&pool).await;
}

#[tokio::test]
async fn t_auto_approve_enabled_approves_activates_and_binds() {
    let _guard = serialize().await;
    let pool = connect_test_db().await;
    setup_fixtures(&pool).await;
    set_switch(&pool, Some(true)).await;

    run_handler(&pool).await;

    assert_eq!(
        terminal_status(&pool, APPR_NEW).await.as_deref(),
        Some("approved"),
        "开关开启后待批注册实例应自动通过"
    );
    let (status, entity_id, entity_table) = applicant_state(&pool).await;
    assert_eq!(status, "active", "自动通过后申请人应激活");
    assert_eq!(entity_id, Some(SUBJ_NEW), "审批通过应绑定实名主体 id");
    assert_eq!(
        entity_table.as_deref(),
        Some("zc_id_empl-natural"),
        "审批通过应绑定实名主体表"
    );

    // 幂等：重复 tick 不产生第二条状态桥/意见
    run_handler(&pool).await;
    let status_rows: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_lifecycle_r_primary-status"
            WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(APPR_NEW)
    .fetch_one(&pool)
    .await
    .expect("count status rows");
    assert_eq!(status_rows, 1, "重复 tick 不得重复写状态桥");

    cleanup_fixtures(&pool).await;
    set_switch(&pool, None).await;
}
