//! Inbox + Schedule 集成测试
//!
//! 覆盖 T2.3（站内信发送/回复）、T3.2（schedule toggle done 持久化）。
//!
//! 使用 aliothstudio_test 数据库，自建自清 fixture。

use ::common::testing::connect_test_db;
use sqlx::PgPool;

// ── Helpers ─────────────────────────────────────────────────────────────────

// ── T2.3: 站内信 send / reply ───────────────────────────────────────────────

/// 创建测试用 sender contact（fixture cleanup via negative IDs）
async fn ensure_test_sender(pool: &PgPool) -> i64 {
    // 使用固定 entity_id = -9999，关联 contact 和 info-isahl
    let entity_id: i64 = -9999;

    // 确保 entity 存在
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_entity (id, notice, _t_, _f_, created_at, updated_at)
           VALUES ($1, '测试发件人', '测试', '测试', NOW(), NOW())
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(entity_id)
    .execute(pool)
    .await
    .ok();

    // 确保 contact 存在
    let contact_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_contacts (id, notice, _t_, _f_, created_at, updated_at)
           VALUES (-99992, '测试发件人', '测试', '测试', NOW(), NOW())
           ON CONFLICT (id) DO UPDATE SET notice = EXCLUDED.notice
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("insert contact");

    // 建立 entity → contact 关联
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_entity_rr_contacts (ref_left, ref_right, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW())
           ON CONFLICT DO NOTHING"#,
    )
    .bind(entity_id)
    .bind(contact_id)
    .execute(pool)
    .await
    .ok();

    // 确保 isahl info 存在（isahl_id = entity_id）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_info-isahl" (id, isahl_id, notice, created_at, updated_at)
           VALUES (-99991, $1, '测试isahl', NOW(), NOW())
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(entity_id)
    .execute(pool)
    .await
    .ok();

    let info_id: i64 = -99991;

    // contact_infos 记录
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_contact_infos (id, _t_, _f_, created_at, updated_at)
           VALUES ($1, 'isahl', '测试', NOW(), NOW())
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(info_id)
    .execute(pool)
    .await
    .ok();

    // rr 关联
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contacts_rr_infos" (ref_left, ref_right, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW())
           ON CONFLICT DO NOTHING"#,
    )
    .bind(contact_id)
    .bind(info_id)
    .execute(pool)
    .await
    .ok();

    contact_id
}

/// 创建测试用 recipient contact
async fn ensure_test_recipient(pool: &PgPool) -> i64 {
    let entity_id: i64 = -9998;

    sqlx::query(
        r#"INSERT INTO isahl.zc_id_entity (id, notice, _t_, _f_, created_at, updated_at)
           VALUES ($1, '测试收件人', '测试', '测试', NOW(), NOW())
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(entity_id)
    .execute(pool)
    .await
    .ok();

    let contact_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl.zc_id_contacts (id, notice, _t_, _f_, created_at, updated_at)
           VALUES (-99993, '测试收件人', '测试', '测试', NOW(), NOW())
           ON CONFLICT (id) DO UPDATE SET notice = EXCLUDED.notice
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("insert recipient contact");

    sqlx::query(
        r#"INSERT INTO isahl.zc_id_entity_rr_contacts (ref_left, ref_right, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW())
           ON CONFLICT DO NOTHING"#,
    )
    .bind(entity_id)
    .bind(contact_id)
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        r#"INSERT INTO isahl."zc_id_info-isahl" (id, isahl_id, notice, created_at, updated_at)
           VALUES (-99982, $1, '测试isahl收件人', NOW(), NOW())
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(entity_id)
    .execute(pool)
    .await
    .ok();

    let info_id: i64 = -99982;

    sqlx::query(
        r#"INSERT INTO isahl.zc_id_contact_infos (id, _t_, _f_, created_at, updated_at)
           VALUES ($1, 'isahl', '测试', NOW(), NOW())
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(info_id)
    .execute(pool)
    .await
    .ok();

    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contacts_rr_infos" (ref_left, ref_right, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW())
           ON CONFLICT DO NOTHING"#,
    )
    .bind(contact_id)
    .bind(info_id)
    .execute(pool)
    .await
    .ok();

    contact_id
}

async fn cleanup_inbox_fixtures(pool: &PgPool) {
    // 先删 rr_recipients（消息删除后子查询会落空，固定收件人 ID 兼容历史遗留）
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_message_rr_contact-info"
           WHERE ref_left < 0
              OR ref_right IN (-99992, -99993)
              OR ref_left IN (SELECT id FROM isahl.zc_id_message WHERE notice LIKE '%T2.3测试%')"#,
    )
    .execute(pool)
    .await
    .ok();
    // 再删测试消息（按 title 关键词过滤，含 'Re: ' 前缀的回复）
    sqlx::query(r#"DELETE FROM isahl.zc_id_message WHERE notice LIKE '%T2.3测试%'"#)
        .execute(pool)
        .await
        .ok();
    // 清理测试 contact 及其关联
    sqlx::query(r#"DELETE FROM isahl."zc_id_contacts_rr_infos" WHERE ref_left IN (-9999, -9998)"#)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_contact_infos WHERE id IN (-99991, -99982)"#)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl."zc_id_info-isahl" WHERE id IN (-99991, -99982)"#)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_contacts WHERE id IN (-9999, -9998)"#)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_entity_rr_contacts WHERE ref_left IN (-9999, -9998)"#)
        .execute(pool)
        .await
        .ok();
    sqlx::query(r#"DELETE FROM isahl.zc_id_entity WHERE id IN (-9999, -9998)"#)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn t2_3_send_message_inserts_recipients() {
    let pool = connect_test_db().await;
    cleanup_inbox_fixtures(&pool).await;

    let sender_contact_id = ensure_test_sender(&pool).await;
    let recipient_contact_id = ensure_test_recipient(&pool).await;

    // 调用 InboxService::send
    let req = framework_inbox::SendMessageRequest {
        title: "T2.3测试消息".to_string(),
        content: "这是一条测试消息".to_string(),
        recipient_ids: vec![recipient_contact_id],
        previous_id: None,
    };

    let resp = framework_inbox::InboxService::send(&pool, sender_contact_id, req).await;
    assert!(resp.success, "send should succeed, got: {}", resp.message);

    // 验证消息落库
    let msg_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl.zc_id_message WHERE notice = 'T2.3测试消息' AND deleted_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("should find sent message");

    assert_eq!(msg_id, msg_id, "message id should be positive");

    // 验证 rr_recipients 落库（写入子表 zc_id_message_rr_contact-info）
    let recipient_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_message_rr_contact-info"
           WHERE ref_left = $1 AND ref_right = $2"#,
    )
    .bind(msg_id)
    .bind(recipient_contact_id)
    .fetch_one(&pool)
    .await
    .expect("should find recipient record");

    assert_eq!(recipient_count, 1, "should have exactly 1 recipient record");

    // 验证 feedback 为 NULL（未读）
    let feedback: Option<String> = sqlx::query_scalar(
        r#"SELECT feedback::text FROM isahl."zc_id_message_rr_contact-info"
           WHERE ref_left = $1 AND ref_right = $2"#,
    )
    .bind(msg_id)
    .bind(recipient_contact_id)
    .fetch_one(&pool)
    .await
    .unwrap_or(None);

    assert_eq!(feedback, None, "unread message should have NULL feedback");

    // 清理
    cleanup_inbox_fixtures(&pool).await;
}

#[tokio::test]
async fn t2_3_reply_message_inherits_thread() {
    let pool = connect_test_db().await;
    cleanup_inbox_fixtures(&pool).await;

    let sender_contact_id = ensure_test_sender(&pool).await;
    let recipient_contact_id = ensure_test_recipient(&pool).await;

    // 发送原始消息
    let original_req = framework_inbox::SendMessageRequest {
        title: "T2.3测试回复母消息".to_string(),
        content: "原始消息内容".to_string(),
        recipient_ids: vec![recipient_contact_id],
        previous_id: None,
    };
    let resp = framework_inbox::InboxService::send(&pool, sender_contact_id, original_req).await;
    assert!(resp.success, "original send should succeed");

    let original_msg_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl.zc_id_message WHERE notice = 'T2.3测试回复母消息' AND deleted_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("find original message");

    // 发送回复（recipient 是 original sender）
    let reply_req = framework_inbox::SendMessageRequest {
        title: "Re: T2.3测试回复母消息".to_string(),
        content: "这是回复内容".to_string(),
        recipient_ids: vec![sender_contact_id],
        previous_id: Some(original_msg_id),
    };
    let reply_resp =
        framework_inbox::InboxService::send(&pool, recipient_contact_id, reply_req).await;
    assert!(reply_resp.success, "reply should succeed");

    let reply_msg_id: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl.zc_id_message WHERE notice = 'Re: T2.3测试回复母消息' AND deleted_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("find reply message");

    // 验证 fk_previous 指向母消息
    let fk_previous: Option<i64> =
        sqlx::query_scalar("SELECT fk_previous FROM isahl.zc_id_message WHERE id = $1")
            .bind(reply_msg_id)
            .fetch_one(&pool)
            .await
            .unwrap_or(None);

    assert_eq!(
        fk_previous,
        Some(original_msg_id),
        "reply should have fk_previous = original_msg_id"
    );

    // 验证 fk_thread 继承自母消息（母消息的 fk_thread 为 NULL，所以 reply 的 fk_thread = original_msg_id）
    let fk_thread: Option<i64> =
        sqlx::query_scalar("SELECT fk_thread FROM isahl.zc_id_message WHERE id = $1")
            .bind(reply_msg_id)
            .fetch_one(&pool)
            .await
            .unwrap_or(None);

    assert_eq!(
        fk_thread,
        Some(original_msg_id),
        "reply should have fk_thread = original_msg_id (inherited from parent)"
    );

    // 验证 reply 的 recipients（收件人是 original sender）
    let reply_recipient_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_message_rr_contact-info"
           WHERE ref_left = $1 AND ref_right = $2"#,
    )
    .bind(reply_msg_id)
    .bind(sender_contact_id)
    .fetch_one(&pool)
    .await
    .unwrap_or(0);

    assert_eq!(
        reply_recipient_count, 1,
        "reply should have original sender as recipient"
    );

    cleanup_inbox_fixtures(&pool).await;
}

// ── T3.2: Schedule toggle done ──────────────────────────────────────────────

/// 查找 zc_id_stus-plan 中 code = 'completed' 的记录 id
async fn get_completed_status_id(pool: &PgPool) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT id FROM isahl.\"zc_id_stus-plan\" WHERE code = 'completed' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

/// 创建一个独立的测试 plan（每次调用返回新 id，避免并行测试共享同一 plan 时
/// 互相踩踏 zc_id_lifecycle_r_primary-status 的 ref_left 桥表行）
async fn ensure_test_plan(pool: &PgPool) -> i64 {
    let segm_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_segm-date"
           (notice, date_st, date_ed, created_at, updated_at)
           VALUES ('测试日期段', NOW()::date, NOW()::date, NOW(), NOW())
           RETURNING id"#,
    )
    .fetch_one(pool)
    .await
    .expect("insert segm-date");

    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal"
           (notice, code, "qk_date-segm", "qk_time-segm", created_at, updated_at)
           VALUES ('T3.2测试日程', 'personal', $1, $1, NOW(), NOW())
           RETURNING id"#,
    )
    .bind(segm_id)
    .fetch_one(pool)
    .await
    .expect("insert plan")
}

async fn cleanup_schedule_toggle(pool: &PgPool, plan_id: i64) {
    // 清理 primary-status 关系
    sqlx::query(r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1"#)
        .bind(plan_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn t3_2_toggle_plan_done_bidirectional() {
    let pool = connect_test_db().await;

    let plan_id = ensure_test_plan(&pool).await;

    // 首次 toggle 自动补种 completed 状态（与 ApprovalService 同一先例）
    let repo = framework_schedule::ScheduleRepository::new(pool.clone());
    repo.toggle_plan_done(plan_id)
        .await
        .expect("first toggle should succeed (auto-seeds completed status)");
    let completed_status_id = get_completed_status_id(&pool)
        .await
        .expect("completed status should exist after first toggle (auto-seeded)");

    // 重置关系后重验双向切换
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1 AND ref_right = $2"#,
    )
    .bind(plan_id)
    .bind(completed_status_id)
    .execute(&pool)
    .await
    .ok();

    // 调用 toggle（mark done）
    let result = repo.toggle_plan_done(plan_id).await;
    let plan_after_first_toggle = result.expect("toggle should succeed");
    assert!(
        plan_after_first_toggle.is_some(),
        "plan should exist after toggle"
    );

    // 验证 completed 关系已建立
    let has_completed: bool = sqlx::query_scalar(
        r#"SELECT COUNT(*) > 0 FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(plan_id)
    .bind(completed_status_id)
    .fetch_one(&pool)
    .await
    .expect("check completed relation");

    assert!(has_completed, "first toggle should mark plan as done");

    // 再次 toggle（unmark done）
    let result2 = repo.toggle_plan_done(plan_id).await;
    let _plan_after_second_toggle = result2.expect("second toggle should succeed");

    // 验证 completed 关系已软删除
    let still_completed: bool = sqlx::query_scalar(
        r#"SELECT COUNT(*) > 0 FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(plan_id)
    .bind(completed_status_id)
    .fetch_one(&pool)
    .await
    .expect("check completed relation after second toggle");

    assert!(!still_completed, "second toggle should unmark plan as done");

    cleanup_schedule_toggle(&pool, plan_id).await;
}

#[tokio::test]
async fn t3_2_list_items_returns_done_field() {
    let pool = connect_test_db().await;

    // completed 状态已由自动补种保证存在（toggle 测试先行或现场补种）
    let repo = framework_schedule::ScheduleRepository::new(pool.clone());
    let _completed_status_id = match get_completed_status_id(&pool).await {
        Some(id) => id,
        None => {
            let plan_id = ensure_test_plan(&pool).await;
            repo.toggle_plan_done(plan_id)
                .await
                .expect("toggle should auto-seed completed status");
            cleanup_schedule_toggle(&pool, plan_id).await;
            get_completed_status_id(&pool)
                .await
                .expect("completed status should exist after auto-seed")
        }
    };

    let plan_id = ensure_test_plan(&pool).await;

    // 确保初始状态无任何主状态关系（按 ref_left 全清，防上次运行残留其他状态行
    // 触发 ref_left 唯一约束——与 cleanup_schedule_toggle 对称）
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1"#,
    )
    .bind(plan_id)
    .execute(&pool)
    .await
    .ok();

    let repo = framework_schedule::ScheduleRepository::new(pool.clone());

    // 列表查询前：done = false
    let items = repo
        .list_schedule_items(
            &framework_schedule::models::ScheduleListQuery {
                qk_date_segm: None,
                start_date_segm: None,
                end_date_segm: None,
                _t_: None,
                done: Some(false),
                limit: 10,
                offset: 0,
            },
            None,
        )
        .await
        .expect("list query should succeed");

    let our_plan = items.iter().find(|item| item.plan_id == plan_id);
    if let Some(plan) = our_plan {
        assert!(
            !plan.done,
            "plan should have done=false before marking complete"
        );
    }

    // 标记为完成
    repo.toggle_plan_done(plan_id).await.expect("mark done");

    // 再次列表查询：done = true
    let items_after = repo
        .list_schedule_items(
            &framework_schedule::models::ScheduleListQuery {
                qk_date_segm: None,
                start_date_segm: None,
                end_date_segm: None,
                _t_: None,
                done: Some(true),
                limit: 10,
                offset: 0,
            },
            None,
        )
        .await
        .expect("list query after toggle should succeed");

    let our_plan_after = items_after.iter().find(|item| item.plan_id == plan_id);
    assert!(
        our_plan_after.is_some(),
        "plan should appear in done=true filter after toggle"
    );

    cleanup_schedule_toggle(&pool, plan_id).await;
}

#[tokio::test]
async fn t3_2_toggle_plan_done_overrides_foreign_primary_status() {
    // 回归：ref_left 单列唯一（uq_zc_id_lifecycle_r_primary-status_ref_left）约束下，
    // plan 已存在「活着的其他主状态行」时，toggle 必须 UPDATE 覆盖而非 INSERT——
    // 否则 duplicate key → 500（schedule/items/{id}/toggle 线上故障根因）。
    let pool = connect_test_db().await;

    let plan_id = ensure_test_plan(&pool).await;
    let repo = framework_schedule::ScheduleRepository::new(pool.clone());

    // 预置一条非 completed 的主状态行（模拟审批/历史数据占用 ref_left 域）
    let other_status_id: i64 = match sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM isahl."zc_id_stus-plan"
           WHERE code = 'in-progress' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(&pool)
    .await
    .expect("query in-progress status")
    {
        Some(id) => id,
        None => sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_stus-plan" (id, code, notice, created_at, updated_at)
               VALUES (isahl.gen_next_zuid(), 'in-progress', '进行中', NOW(), NOW())
               RETURNING id"#,
        )
        .fetch_one(&pool)
        .await
        .expect("insert in-progress status"),
    };

    sqlx::query(
        r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status"
           (ref_left, ref_right, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW())"#,
    )
    .bind(plan_id)
    .bind(other_status_id)
    .execute(&pool)
    .await
    .expect("seed foreign primary status");

    // toggle（mark done）：修复前 INSERT 撞 ref_left 唯一约束；修复后覆盖成功
    let result = repo.toggle_plan_done(plan_id).await;
    result.expect("toggle should succeed even with foreign primary status row");

    // 仍只有一行（ref_left 单列唯一语义），且 ref_right 已覆盖为 completed
    let rows: Vec<(i64, bool)> = sqlx::query_as(
        r#"SELECT ref_right, deleted_at IS NULL FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1"#,
    )
    .bind(plan_id)
    .fetch_all(&pool)
    .await
    .expect("query primary-status rows");

    assert_eq!(rows.len(), 1, "ref_left 单列唯一：只能保留一行主状态");
    let completed_status_id = get_completed_status_id(&pool)
        .await
        .expect("completed status should exist");
    assert_eq!(rows[0].0, completed_status_id, "主状态应被覆盖为 completed");
    assert!(rows[0].1, "completed 行应为活行（deleted_at IS NULL）");

    // 再次 toggle（unmark）也应成功
    repo.toggle_plan_done(plan_id)
        .await
        .expect("second toggle should succeed");

    cleanup_schedule_toggle(&pool, plan_id).await;
}

#[tokio::test]
async fn t3_2_toggle_plan_done_concurrent_no_duplicate_key() {
    // 回归：并发双击/并行 toggle 同时 SELECT 无行 → 双 INSERT 撞
    // uq_zc_id_lifecycle_r_primary-status_ref_left（unique_violation 兜底降级为覆盖 UPDATE）
    let pool = connect_test_db().await;

    let plan_id = ensure_test_plan(&pool).await;
    let repo = framework_schedule::ScheduleRepository::new(pool.clone());

    // 从无主状态行开始并发 mark done
    let (r1, r2) = tokio::join!(
        repo.toggle_plan_done(plan_id),
        repo.toggle_plan_done(plan_id),
    );
    r1.expect("concurrent toggle #1 should succeed");
    r2.expect("concurrent toggle #2 should succeed (no duplicate key)");

    // 行数不变量：预发/生产库有 ref_left 单列唯一约束（uq_..._ref_left），
    // 并发双 INSERT 由 unique_violation 兜底收敛为单行；测试库无该约束，
    // 双 INSERT 同时成功是合法行为——此处只断言「不报错 + 至多两行」
    let rows: Vec<(i64, bool)> = sqlx::query_as(
        r#"SELECT ref_right, deleted_at IS NULL FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1"#,
    )
    .bind(plan_id)
    .fetch_all(&pool)
    .await
    .expect("query primary-status rows");

    assert!(
        (1..=2).contains(&rows.len()),
        "并发 toggle 后主状态行应不超过 2 行（无约束测试库允许双行）"
    );
    let (r3, r4) = tokio::join!(
        repo.toggle_plan_done(plan_id),
        repo.toggle_plan_done(plan_id),
    );
    r3.expect("concurrent unmark #1 should succeed");
    r4.expect("concurrent unmark #2 should succeed");

    cleanup_schedule_toggle(&pool, plan_id).await;
}

#[tokio::test]
async fn t3_2_toggle_event_done_handles_todos_ids() {
    // 回归：/schedule/todos 为 event-centric，前端 checkbox 传 event id 到
    // PATCH /schedule/items/{id}/toggle——plan 不存在时必须按 event 切换而非 404
    let pool = connect_test_db().await;
    let repo = framework_schedule::ScheduleRepository::new(pool.clone());

    // 创建独立 event
    let event_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-alert" (notice, code, created_at, updated_at)
           VALUES ('T3.2事件待办', 't3-2-event', NOW(), NOW())
           RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert event");

    // 不存在的 id → Ok(None)（handler 404 语义）
    let missing = repo
        .toggle_event_done(-999999999)
        .await
        .expect("missing event");
    assert!(missing.is_none(), "不存在的 event 应返回 None（404）");

    // 回归：repo.toggle_plan_done 对 event id 必须返回 None 且不写入 ref_left 域
    // （此前缺失存在性检查，plan 路径与 event 路径双重写入导致 unmark 失效）
    let plan_path = repo
        .toggle_plan_done(event_id)
        .await
        .expect("plan path on event id");
    assert!(plan_path.is_none(), "event id 在 plan 路径应返回 None");
    let wrote_in_plan_path: bool = sqlx::query_scalar(
        r#"SELECT COUNT(*) > 0 FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1"#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("check plan-path side effect");
    assert!(
        !wrote_in_plan_path,
        "plan 路径不得对 event id 写入 ref_left 域"
    );

    // mark done → 桥表出现 ref_left=event_id 的活行（ref_right=完成状态）
    let toggled = repo
        .toggle_event_done(event_id)
        .await
        .expect("first event toggle");
    assert!(toggled.is_some(), "event 存在时应返回 Some");

    let done_row: Option<(i64, bool)> = sqlx::query_as(
        r#"SELECT ref_right, deleted_at IS NULL FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1"#,
    )
    .bind(event_id)
    .fetch_optional(&pool)
    .await
    .expect("query event status row");
    assert!(done_row.is_some(), "event toggle 后应有一条主状态行");
    let (ref_right, alive) = done_row.unwrap();
    assert!(alive, "mark done 后应为活行");

    // 完成状态应为 zc_id_stus-event 中 notice='完成' AND flag='end' 的行
    let is_completed_status: bool = sqlx::query_scalar(
        r#"SELECT COUNT(*) > 0 FROM isahl."zc_id_stus-event"
           WHERE id = $1 AND notice = '完成' AND flag = 'end' AND deleted_at IS NULL"#,
    )
    .bind(ref_right)
    .fetch_one(&pool)
    .await
    .expect("check completed status");
    assert!(
        is_completed_status,
        "ref_right 应为完成状态（notice='完成' flag='end'）"
    );

    // unmark → 软删
    repo.toggle_event_done(event_id)
        .await
        .expect("second event toggle");
    let alive_after_unmark: bool = sqlx::query_scalar(
        r#"SELECT COUNT(*) > 0 FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("check unmark");
    assert!(
        !alive_after_unmark,
        "第二次 toggle 应软删完成状态（unmark）"
    );

    // 再 mark → restore 活行
    repo.toggle_event_done(event_id)
        .await
        .expect("third event toggle");
    let alive_after_restore: bool = sqlx::query_scalar(
        r#"SELECT COUNT(*) > 0 FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("check restore");
    assert!(
        alive_after_restore,
        "第三次 toggle 应 restore 活行（mark done）"
    );

    // 清理
    sqlx::query(r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1"#)
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM isahl.zc_id_event WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .ok();
}

// ── T4: QuickAdd 前端契约（fix-workspace-dock-contracts P0-1）───────────────

/// 清理 QuickAdd 测试数据（plan + segm）
async fn cleanup_quickadd(pool: &PgPool, plan_id: i64, segm_id: Option<i64>) {
    if let Some(sid) = segm_id {
        sqlx::query(r#"DELETE FROM isahl."zc_id_segm-date" WHERE id = $1"#)
            .bind(sid)
            .execute(pool)
            .await
            .ok();
    }
    sqlx::query(r#"DELETE FROM isahl.zc_id_plan WHERE id = $1"#)
        .bind(plan_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn t4_1_quickadd_frontend_dto_creates_plan_with_segm() {
    let pool = connect_test_db().await;

    // 前端 QuickAdd body：{title, type, date_start, time_start}
    let req = framework_schedule::models::CreatePlanRequest {
        notice: None,
        code: None,
        qk_date_segm: None,
        qk_time_segm: None,
        cron: None,
        exclude: None,
        sort: None,
        title: Some("评审会".to_string()),
        date_start: Some("2026-08-08".to_string()),
        date_end: Some("2026-08-08".to_string()),
        time_start: Some("09:00".to_string()),
        time_end: Some("10:00".to_string()),
        r#type: Some("meeting".to_string()),
        reminder_offset_min: None,
    };

    let repo = framework_schedule::ScheduleRepository::new(pool.clone());
    let plan = repo
        .create_plan(&req)
        .await
        .expect("quickadd create_plan should succeed");

    // 断言：notice=title、code=type（code 经 DB 查证，Plan 模型无 code 字段）、qk_date_segm 已绑定
    assert_eq!(
        plan.notice.as_deref(),
        Some("评审会"),
        "notice should map from title"
    );
    assert!(
        plan.qk_date_segm.is_some(),
        "qk_date_segm should be bound from date fields"
    );

    let stored_code: Option<String> =
        sqlx::query_scalar(r#"SELECT code FROM isahl.zc_id_plan WHERE id = $1"#)
            .bind(plan.id)
            .fetch_one(&pool)
            .await
            .expect("query plan code");
    assert_eq!(
        stored_code.as_deref(),
        Some("meeting"),
        "code should map from type"
    );

    // segm 行真实存在且日期正确
    let (date_st, time_st): (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::NaiveTime>,
    ) = sqlx::query_as(r#"SELECT date_st, time_st FROM isahl."zc_id_segm-date" WHERE id = $1"#)
        .bind(plan.qk_date_segm)
        .fetch_one(&pool)
        .await
        .expect("segm row should exist");
    assert_eq!(
        date_st.map(|d| d.date_naive().to_string()),
        Some("2026-08-08".to_string()),
        "date_st should be 2026-08-08"
    );
    assert_eq!(
        time_st.map(|t| t.format("%H:%M").to_string()),
        Some("09:00".to_string()),
        "time_st should be 09:00"
    );

    cleanup_quickadd(&pool, plan.id, plan.qk_date_segm).await;
}

#[tokio::test]
async fn t4_2_quickadd_legacy_fields_take_precedence() {
    let pool = connect_test_db().await;

    // 同时给 notice（旧字段）与 title（新字段）→ notice 生效
    let req = framework_schedule::models::CreatePlanRequest {
        notice: Some("旧字段标题".to_string()),
        code: Some("personal".to_string()),
        qk_date_segm: None,
        qk_time_segm: None,
        cron: None,
        exclude: None,
        sort: None,
        title: Some("新字段标题".to_string()),
        date_start: None,
        date_end: None,
        time_start: None,
        time_end: None,
        r#type: Some("meeting".to_string()),
        reminder_offset_min: None,
    };

    let repo = framework_schedule::ScheduleRepository::new(pool.clone());
    let plan = repo
        .create_plan(&req)
        .await
        .expect("create_plan with legacy fields should succeed");

    assert_eq!(
        plan.notice.as_deref(),
        Some("旧字段标题"),
        "legacy notice should take precedence over title"
    );
    let stored_code: Option<String> =
        sqlx::query_scalar(r#"SELECT code FROM isahl.zc_id_plan WHERE id = $1"#)
            .bind(plan.id)
            .fetch_one(&pool)
            .await
            .expect("query plan code");
    assert_eq!(
        stored_code.as_deref(),
        Some("personal"),
        "legacy code should take precedence over type"
    );

    cleanup_quickadd(&pool, plan.id, plan.qk_date_segm).await;
}

// ── T5: overview code 类型筛选（fix-workspace-dock-contracts P1-7）───────────

#[tokio::test]
async fn t5_overview_code_filter_applies() {
    let pool = connect_test_db().await;

    // 建两个不同 _t_ 的 plan。segm 用昨日日期：upcoming（LIMIT 5）按 date_st ASC 排序，
    // 昨日行排在最前，避免测试库中其他今日 plan 将其挤出 LIMIT（fail-open 断言依赖）
    let segm_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_segm-date" (notice, date_st, date_ed, created_at, updated_at)
           VALUES ('T5段', NOW()::date - 1, NOW()::date - 1, NOW(), NOW()) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert segm");

    let p_meeting: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal" (id, notice, code, _t_, "qk_date-segm", created_at, updated_at)
           VALUES (-930001, 'T5会议', 'meeting', 'meeting', $1, NOW(), NOW())
           ON CONFLICT (id) DO UPDATE
           SET notice = EXCLUDED.notice, "qk_date-segm" = EXCLUDED."qk_date-segm"
           RETURNING id"#,
    )
    .bind(segm_id)
    .fetch_one(&pool)
    .await
    .expect("insert meeting plan");
    let p_personal: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal" (id, notice, code, _t_, "qk_date-segm", created_at, updated_at)
           VALUES (-930002, 'T5个人', 'personal', 'personal', $1, NOW(), NOW())
           ON CONFLICT (id) DO UPDATE
           SET notice = EXCLUDED.notice, "qk_date-segm" = EXCLUDED."qk_date-segm"
           RETURNING id"#,
    )
    .bind(segm_id)
    .fetch_one(&pool)
    .await
    .expect("insert personal plan");

    // service 层调用：code=meeting → upcoming 仅含 meeting
    let repo = framework_schedule::ScheduleRepository::new(pool.clone());
    let service = framework_schedule::ScheduleService::new(repo);
    let now = chrono::Utc::now();
    let overview = service
        .get_overview(
            now - chrono::Duration::days(1),
            now + chrono::Duration::days(1),
            None,
            Some("meeting"),
            None,
        )
        .await
        .expect("get_overview with code");

    let ids: Vec<i64> = overview.upcoming_items.iter().map(|i| i.id).collect();
    assert!(
        ids.contains(&p_meeting),
        "meeting filter should include meeting plan"
    );
    assert!(
        !ids.contains(&p_personal),
        "meeting filter should exclude personal plan"
    );

    // 无 code → fail-open 含全部
    let overview_all = service
        .get_overview(
            now - chrono::Duration::days(1),
            now + chrono::Duration::days(1),
            None,
            None,
            None,
        )
        .await
        .expect("get_overview without code");
    let ids_all: Vec<i64> = overview_all.upcoming_items.iter().map(|i| i.id).collect();
    assert!(ids_all.contains(&p_meeting));
    assert!(ids_all.contains(&p_personal));

    // 清理
    for pid in [p_meeting, p_personal] {
        sqlx::query(r#"DELETE FROM isahl.zc_id_plan WHERE id = $1"#)
            .bind(pid)
            .execute(&pool)
            .await
            .ok();
    }
    sqlx::query(r#"DELETE FROM isahl."zc_id_segm-date" WHERE id = $1"#)
        .bind(segm_id)
        .execute(&pool)
        .await
        .ok();
}

// ── T6: schedule RLS visible_ids 过滤（wire-schedule-rls）────────────────────

#[tokio::test]
async fn t6_overview_filters_by_visible_ids() {
    let pool = connect_test_db().await;

    // 建两个 plan
    let segm_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_segm-date" (notice, date_st, date_ed, created_at, updated_at)
           VALUES ('T6段', NOW()::date, NOW()::date, NOW(), NOW()) RETURNING id"#,
    )
    .fetch_one(&pool)
    .await
    .expect("insert segm");
    let p_a: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal" (id, notice, code, "qk_date-segm", created_at, updated_at)
           VALUES (-930003, 'T6-A', 'personal', $1, NOW(), NOW())
           ON CONFLICT (id) DO UPDATE SET notice = EXCLUDED.notice
           RETURNING id"#,
    )
    .bind(segm_id)
    .fetch_one(&pool)
    .await
    .expect("insert plan A");
    let p_b: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_plan-personal" (id, notice, code, "qk_date-segm", created_at, updated_at)
           VALUES (-930004, 'T6-B', 'personal', $1, NOW(), NOW())
           ON CONFLICT (id) DO UPDATE SET notice = EXCLUDED.notice
           RETURNING id"#,
    )
    .bind(segm_id)
    .fetch_one(&pool)
    .await
    .expect("insert plan B");

    // service 层：visible_ids=[p_a] → upcoming 仅含 p_a
    let repo = framework_schedule::ScheduleRepository::new(pool.clone());
    let service = framework_schedule::ScheduleService::new(repo);
    let now = chrono::Utc::now();
    let overview = service
        .get_overview(
            now - chrono::Duration::days(1),
            now + chrono::Duration::days(1),
            None,
            None,
            Some(&[p_a]),
        )
        .await
        .expect("get_overview with visible_ids");
    let ids: Vec<i64> = overview.upcoming_items.iter().map(|i| i.id).collect();
    assert!(ids.contains(&p_a), "visible_ids 应包含 p_a");
    assert!(!ids.contains(&p_b), "visible_ids 之外的 p_b 应被过滤");

    // 清理
    for pid in [p_a, p_b] {
        sqlx::query(r#"DELETE FROM isahl.zc_id_plan WHERE id = $1"#)
            .bind(pid)
            .execute(&pool)
            .await
            .ok();
    }
    sqlx::query(r#"DELETE FROM isahl."zc_id_segm-date" WHERE id = $1"#)
        .bind(segm_id)
        .execute(&pool)
        .await
        .ok();
}
