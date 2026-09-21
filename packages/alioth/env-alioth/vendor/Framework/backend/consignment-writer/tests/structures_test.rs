//! 委托编辑**结构写路**回归（change: `migrate-consignment-fields-to-structures` T10）
//!
//! 被测单元 = `consignment_writer::update_consignment_structures_tx`
//! （`PUT /service/isahl-db/consignments/{id}` 的结构写件单一实现；由
//! `identity-org::repository::consignment::update` 薄调用）。
//!
//! 判据（读侧口径取自 `logi-consignment/repositories/tasks.rs`，与本 change 读侧收口一致）：
//! cargo ← 明细首条非 `DTL-LDG-%` 的 `COALESCE(comments, notice)`；体积 ← 明细 `qk_v_qty` →
//! `zc_id_scale.mark`；起讫 ← `zc_id_prod-transport_rr_stop`（`ck_category` ST-DEPART/ARRIVE、
//! `"over-seq"` 1/2）；时段 ← 产品 `qk_period` → `zc_id_segm-date`。
//! **委托 `comments` 摘要 MUST NOT 参与任何取值**（每个用例预置误导性摘要并在断言后复核其未被消费）。
//!
//! 数据纪律：全部行码带 `STRUCT-TEST-{nanos}` 后缀、自造自清（硬删），
//! MUST NOT 触碰既有夹具数据；幂等用例连调两次证明可重复。

use common::testing::connect_test_db;
use consignment_writer::{update_consignment_structures_tx, UpdateStructuresInput};
use sqlx::PgPool;

/// 误导性摘要（读侧 MUST 忽略；注入错误值以证明「结构值 ≠ 摘要值」）
const MISLEADING: &str = "客户:错误客户 货物:错误货物 起点:火星 讫点:水星 CBM:999";

struct Fixture {
    code: String,
    consignment_id: i64,
    origin_place: i64,
    dest_place: i64,
}

fn suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}{}", nanos % 0xF_FFFF_FFFF, std::process::id() % 100)
}

/// 裸委托（无产品/无明细/无时段）+ 真实停靠行；摘要故意为误导值。
async fn setup(pool: &PgPool) -> Fixture {
    let code = format!("STRUCT-TEST-{}", suffix());
    let places: Vec<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_place" WHERE deleted_at IS NULL ORDER BY id LIMIT 2"#,
    )
    .fetch_all(pool)
    .await
    .expect("test db 需有 ≥2 个 zc_id_place 停靠行");
    assert!(places.len() >= 2, "test db 停靠行不足（需 ≥2）");
    let subject: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_subjects" WHERE deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .expect("subjects probe")
    .unwrap_or(1);
    let cid: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_orde-land"
             (code, notice, comments, fk_subject, fk_object, created_by_id,
              dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $4, 1,
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'TX'  AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'FJA' AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_EV' AND deleted_at IS NULL))
           RETURNING id"#,
    )
    .bind(&code)
    .bind("结构写路测试委托")
    .bind(MISLEADING)
    .bind(subject)
    .fetch_one(pool)
    .await
    .expect("insert consignment");
    Fixture {
        code,
        consignment_id: cid,
        origin_place: places[0],
        dest_place: places[1],
    }
}

/// 自清：先子后父（仅本用例自造行——以行码后缀与 fk_previous/id 定位）
async fn cleanup(pool: &PgPool, fx: &Fixture) {
    let cid = fx.consignment_id;
    let like = format!("%{}%", fx.code); // DTL-{code} / STOP-PRD-{code}-D / VOL-{code} / PERIOD-{code}
    for sql in [
        r#"DELETE FROM isahl."zc_id_prod-transport_rr_stop"
            WHERE ref_left IN (SELECT id FROM isahl."zc_id_prod-freight_road-sales" WHERE fk_previous = $1 OR code LIKE $2)
               OR code LIKE $2"#,
        r#"DELETE FROM isahl."zc_id_deta-trade_order" WHERE fk_list = $1 OR code LIKE $2"#,
        r#"DELETE FROM isahl."zc_id_prod-freight_road-sales" WHERE fk_previous = $1 OR code LIKE $2"#,
        r#"DELETE FROM isahl."zc_id_scal-volume" WHERE code LIKE $2"#,
        r#"DELETE FROM isahl."zc_id_scal-price" WHERE code LIKE $2"#,
        r#"DELETE FROM isahl."zc_id_scal-weight" WHERE code LIKE $2"#,
        r#"DELETE FROM isahl."zc_id_scal-amount" WHERE code LIKE $2"#,
        r#"DELETE FROM isahl."zc_id_segm-date" WHERE code LIKE $2"#,
        r#"DELETE FROM isahl."zc_id_orde-land" WHERE id = $1"#,
    ] {
        sqlx::query(sql)
            .bind(cid)
            .bind(&like)
            .execute(pool)
            .await
            .expect("cleanup");
    }
}

// ── 读侧派生探针（表达式同 tasks.rs 口径）────────────────────────────────────────

/// 明细首条非 `DTL-LDG-%` 的 `COALESCE(comments, notice)`
async fn derived_cargo(pool: &PgPool, cid: i64) -> Option<String> {
    sqlx::query_scalar(
        r#"SELECT COALESCE(dd.comments, dd.notice) FROM isahl."zc_id_deta-trade_order" dd
           WHERE dd.fk_list = $1 AND dd.deleted_at IS NULL
           ORDER BY CASE WHEN dd.code LIKE 'DTL-LDG-%' THEN 1 ELSE 0 END, dd.id LIMIT 1"#,
    )
    .bind(cid)
    .fetch_optional(pool)
    .await
    .expect("derived_cargo")
    .flatten()
}

/// 主明细 `qk_v_qty` → `zc_id_scale.mark`（体积真值）
async fn derived_cbm(pool: &PgPool, cid: i64) -> Option<String> {
    sqlx::query_scalar(
        r#"SELECT trim_scale(vt.mark)::text FROM isahl."zc_id_deta-trade_order" dv
           LEFT JOIN isahl."zc_id_scale" vt ON vt.id = dv.qk_v_qty
           WHERE dv.fk_list = $1 AND dv.deleted_at IS NULL
           ORDER BY CASE WHEN dv.code LIKE 'DTL-LDG-%' THEN 1 ELSE 0 END, dv.id LIMIT 1"#,
    )
    .bind(cid)
    .fetch_optional(pool)
    .await
    .expect("derived_cbm")
    .flatten()
}

/// 停靠桥 `(over_seq, ck_category.code, ref_right(place id), place.notice)`，按 over-seq 升序
async fn derived_stops(pool: &PgPool, cid: i64) -> Vec<(i32, String, i64, String)> {
    sqlx::query_as(
        r#"SELECT rs."over-seq", c.code, rs.ref_right, p.notice
           FROM isahl."zc_id_prod-transport_rr_stop" rs
           JOIN isahl."zc_id_cate-traffic" c ON c.id = rs.ck_category
           JOIN isahl."zc_id_place" p ON p.id = rs.ref_right
           WHERE rs.ref_left IN (SELECT id FROM isahl."zc_id_prod-freight_road-sales" WHERE fk_previous = $1)
             AND rs.deleted_at IS NULL
           ORDER BY rs."over-seq" NULLS LAST, rs.id"#,
    )
    .bind(cid)
    .fetch_all(pool)
    .await
    .expect("derived_stops")
}

/// 时段显示（`date_st`/`date_ed` 按 +08 渲染）
async fn derived_period(pool: &PgPool, cid: i64) -> (Option<String>, Option<String>) {
    sqlx::query_as(
        r#"SELECT to_char(seg.date_st AT TIME ZONE 'Asia/Shanghai','YYYY-MM-DD'),
                  to_char(seg.date_ed AT TIME ZONE 'Asia/Shanghai','YYYY-MM-DD')
           FROM isahl."zc_id_prod-freight_road-sales" fp
           LEFT JOIN isahl."zc_id_segm-date" seg ON seg.id = fp.qk_period
           WHERE fp.fk_previous = $1 AND fp.deleted_at IS NULL LIMIT 1"#,
    )
    .bind(cid)
    .fetch_optional(pool)
    .await
    .expect("derived_period")
    .unwrap_or((None, None))
}

/// 结构行计数（明细 / 产品 / 停靠桥）
async fn counts(pool: &PgPool, cid: i64) -> (i64, i64, i64) {
    let details: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_deta-trade_order" WHERE fk_list = $1 AND deleted_at IS NULL"#,
    )
    .bind(cid)
    .fetch_one(pool)
    .await
    .expect("count details");
    let products: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_prod-freight_road-sales" WHERE fk_previous = $1 AND deleted_at IS NULL"#,
    )
    .bind(cid)
    .fetch_one(pool)
    .await
    .expect("count products");
    let stops: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_prod-transport_rr_stop"
           WHERE ref_left IN (SELECT id FROM isahl."zc_id_prod-freight_road-sales" WHERE fk_previous = $1)
             AND deleted_at IS NULL"#,
    )
    .bind(cid)
    .fetch_one(pool)
    .await
    .expect("count stops");
    (details, products, stops)
}

async fn run_update(pool: &PgPool, cid: i64, input: &UpdateStructuresInput) -> Result<(), String> {
    let mut conn = pool.acquire().await.expect("acquire");
    update_consignment_structures_tx(&mut conn, cid, input, 1)
        .await
        .map_err(|e| format!("{e:?}"))
}

/// 摘要未被消费的复核（每用例调用）：委托 comments 原文不变
async fn assert_summary_untouched(pool: &PgPool, cid: i64) {
    let comments: Option<String> =
        sqlx::query_scalar(r#"SELECT comments FROM isahl."zc_id_orde-land" WHERE id = $1"#)
            .bind(cid)
            .fetch_one(pool)
            .await
            .expect("comments probe");
    assert_eq!(
        comments.as_deref(),
        Some(MISLEADING),
        "委托 comments 摘要 MUST 原样保留（仅展示，不参与取值）"
    );
}

/// ① update 路径：结构字段落结构载体，摘要被忽略
#[tokio::test]
async fn update_path_writes_structures_and_ignores_summary() {
    let pool = connect_test_db().await;
    let fx = setup(&pool).await;
    let input = UpdateStructuresInput {
        cargo: Some("结构货物A".into()),
        cbm: Some(12.5),
        origin: Some(fx.origin_place.to_string()),
        dest: Some(fx.dest_place.to_string()),
        ..Default::default()
    };

    run_update(&pool, fx.consignment_id, &input)
        .await
        .expect("update 应成功");

    assert_eq!(
        derived_cargo(&pool, fx.consignment_id).await.as_deref(),
        Some("结构货物A"),
        "货物 MUST 落明细主行（非 comments 摘要）"
    );
    assert_eq!(
        derived_cbm(&pool, fx.consignment_id).await.as_deref(),
        Some("12.5"),
        "体积 MUST 落明细 qk_v_qty → zc_id_scale.mark"
    );
    let stops = derived_stops(&pool, fx.consignment_id).await;
    assert_eq!(stops.len(), 2, "起讫两行，实测 {stops:?}");
    assert_eq!((stops[0].0, stops[0].1.as_str()), (1, "ST-DEPART"));
    assert_eq!(stops[0].2, fx.origin_place, "起点停靠行 = origin_place");
    assert_eq!((stops[1].0, stops[1].1.as_str()), (2, "ST-ARRIVE"));
    assert_eq!(stops[1].2, fx.dest_place, "讫点停靠行 = dest_place");

    // 摘要未被消费：派生值 ≠ 摘要注入的错误值
    assert_ne!(
        derived_cargo(&pool, fx.consignment_id).await.as_deref(),
        Some("错误货物")
    );
    assert_ne!(
        derived_cbm(&pool, fx.consignment_id).await.as_deref(),
        Some("999")
    );
    assert_summary_untouched(&pool, fx.consignment_id).await;

    cleanup(&pool, &fx).await;
}

/// ② 起讫同址：写入 MUST NOT 撞唯一键（唯一键不含 ck_category）
#[tokio::test]
async fn same_site_origin_dest_does_not_violate_unique_key() {
    let pool = connect_test_db().await;
    let fx = setup(&pool).await;
    let same = fx.origin_place.to_string();
    let input = UpdateStructuresInput {
        cargo: Some("结构货物B".into()),
        origin: Some(same.clone()),
        dest: Some(same),
        ..Default::default()
    };

    run_update(&pool, fx.consignment_id, &input)
        .await
        .expect("起讫同址 MUST NOT 报唯一键冲突（23505）");

    let stops = derived_stops(&pool, fx.consignment_id).await;
    // 「地点优先」实现：起点先建 ST-DEPART 行，讫点认领同一行并改判据 ST-ARRIVE
    // ⇒ 同址仅剩 1 行（模型唯一键 (ref_left, ref_right, qk_period) 不含 ck_category）
    assert_eq!(
        stops.len(),
        1,
        "同址仅 1 行（地点优先认领），实测 {stops:?}"
    );
    assert_eq!(stops[0].1, "ST-ARRIVE", "被认领行判据 = 后写的讫点侧");
    assert_eq!(stops[0].2, fx.origin_place);

    cleanup(&pool, &fx).await;
}

/// ③ 幂等：同输入连调两次 → 行数/取值不变
#[tokio::test]
async fn repeated_update_is_idempotent() {
    let pool = connect_test_db().await;
    let fx = setup(&pool).await;
    let input = UpdateStructuresInput {
        cargo: Some("结构货物C".into()),
        cbm: Some(3.25),
        price: Some(120.0),
        origin: Some(fx.origin_place.to_string()),
        dest: Some(fx.dest_place.to_string()),
        pickup_time: Some("2026-10-01".into()),
        eta: Some("2026-10-03".into()),
        ..Default::default()
    };

    run_update(&pool, fx.consignment_id, &input)
        .await
        .expect("首次 update");
    let first = counts(&pool, fx.consignment_id).await;
    let first_cargo = derived_cargo(&pool, fx.consignment_id).await;

    run_update(&pool, fx.consignment_id, &input)
        .await
        .expect("二次 update");
    let second = counts(&pool, fx.consignment_id).await;

    assert_eq!(
        first, second,
        "重复保存 MUST NOT 增生结构行（明细/产品/停靠桥）"
    );
    assert_eq!(first, (1, 1, 2), "结构行数应为 明细 1 / 产品 1 / 停靠桥 2");
    assert_eq!(
        derived_cargo(&pool, fx.consignment_id).await,
        first_cargo,
        "重复保存取值稳定"
    );

    cleanup(&pool, &fx).await;
}

/// ④ create 路径：裸委托（无产品/明细/时段）逐项补建后派生有值
#[tokio::test]
async fn create_path_backfills_missing_structures() {
    let pool = connect_test_db().await;
    let fx = setup(&pool).await;
    assert_eq!(
        counts(&pool, fx.consignment_id).await,
        (0, 0, 0),
        "前置：裸委托无任何结构行"
    );

    let input = UpdateStructuresInput {
        cargo: Some("结构货物D".into()),
        cbm: Some(7.25),
        origin: Some(fx.origin_place.to_string()),
        dest: Some(fx.dest_place.to_string()),
        pickup_time: Some("2026-12-01".into()),
        eta: Some("2026-12-03".into()),
        ..Default::default()
    };

    run_update(&pool, fx.consignment_id, &input)
        .await
        .expect("裸委托补建应成功");

    assert_eq!(
        derived_cargo(&pool, fx.consignment_id).await.as_deref(),
        Some("结构货物D")
    );
    assert_eq!(
        derived_cbm(&pool, fx.consignment_id).await.as_deref(),
        Some("7.25")
    );
    let stops = derived_stops(&pool, fx.consignment_id).await;
    assert_eq!(
        stops.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(),
        vec!["ST-DEPART", "ST-ARRIVE"],
        "补建起讫桥，实测 {stops:?}"
    );
    assert_eq!(stops[0].2, fx.origin_place);
    assert_eq!(stops[1].2, fx.dest_place);
    let (pickup, eta) = derived_period(&pool, fx.consignment_id).await;
    assert_eq!(pickup.as_deref(), Some("2026-12-01"), "补建时段 date_st");
    assert_eq!(eta.as_deref(), Some("2026-12-03"), "补建时段 date_ed");

    cleanup(&pool, &fx).await;
}

/// 货量/金额读数（读侧口径：明细 `qk_w_qty`/`qk_amount` → 标量 `mark` 真值；
/// 夹具仅一条主明细，故单行派生等价于列表的聚合）
async fn derived_scale_mark(pool: &PgPool, cid: i64, column: &str) -> Option<String> {
    let sql = format!(
        r#"SELECT trim_scale(st.mark)::text FROM isahl."zc_id_deta-trade_order" d
           LEFT JOIN isahl."zc_id_scale" st ON st.id = d.{column}
           WHERE d.fk_list = $1 AND d.deleted_at IS NULL
           ORDER BY CASE WHEN d.code LIKE 'DTL-LDG-%' THEN 1 ELSE 0 END, d.id LIMIT 1"#
    );
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(cid)
        .fetch_optional(pool)
        .await
        .expect("derived_scale_mark")
        .flatten()
}

/// 承运商读侧口径（同 `logi-consignment/repositories/tasks.rs` 的 `cr` 投影：
/// 产品 `"fk_subj-provider"`（明细 `fk_deal` 优先，`fk_previous` 兜底）→ `fk_object` → 平台锚，
/// 逐候选剔除系统占位主体）
async fn derived_carrier(pool: &PgPool, cid: i64) -> Option<String> {
    sqlx::query_scalar(
        r#"SELECT cr.notice
           FROM isahl."zc_id_orde-land" o
           LEFT JOIN isahl."zc_id_prod-freight_road-sales" fp
                  ON fp.id = (SELECT d.fk_deal FROM isahl."zc_id_deta-trade_order" d
                              WHERE d.fk_list = o.id AND d.deleted_at IS NULL
                              ORDER BY CASE WHEN d.code LIKE 'DTL-LDG-%' THEN 1 ELSE 0 END, d.id LIMIT 1)
                 AND fp.deleted_at IS NULL
           LEFT JOIN isahl."zc_id_prod-freight_road-sales" fp_carrier
                  ON fp_carrier.fk_previous = o.id AND fp_carrier.deleted_at IS NULL
           LEFT JOIN isahl."zc_id_subjects" cr ON cr.id = COALESCE(
               CASE WHEN EXISTS (SELECT 1 FROM isahl."zc_id_subjects" s
                                 WHERE s.id = fp."fk_subj-provider" AND s.deleted_at IS NULL
                                   AND s.code NOT IN ('SUBJ-ISAH-ADMIN','POS-SYSTEM-ADMIN'))
                    THEN fp."fk_subj-provider" END,
               CASE WHEN EXISTS (SELECT 1 FROM isahl."zc_id_subjects" s
                                 WHERE s.id = fp_carrier."fk_subj-provider" AND s.deleted_at IS NULL
                                   AND s.code NOT IN ('SUBJ-ISAH-ADMIN','POS-SYSTEM-ADMIN'))
                    THEN fp_carrier."fk_subj-provider" END,
               CASE WHEN EXISTS (SELECT 1 FROM isahl."zc_id_subjects" s
                                 WHERE s.id = o.fk_object AND s.deleted_at IS NULL
                                   AND s.code NOT IN ('SUBJ-ISAH-ADMIN','POS-SYSTEM-ADMIN'))
                    THEN o.fk_object END,
               (SELECT s2.id FROM isahl."zc_id_subjects" s2
                JOIN isahl_auth.auth_users su ON su.entity_id = s2.id AND su.id = 1
                WHERE s2.deleted_at IS NULL LIMIT 1))
           WHERE o.id = $1"#,
    )
    .bind(cid)
    .fetch_optional(pool)
    .await
    .expect("derived_carrier")
    .flatten()
}

/// ⑤ 货量/金额：结构真值落标量（货量 MUST NOT 取整、金额按分）
#[tokio::test]
async fn update_writes_weight_and_amount_as_scalar_truth() {
    let pool = connect_test_db().await;
    let fx = setup(&pool).await;
    let input = UpdateStructuresInput {
        weight_ton: Some(34.5),
        amount: Some(12345.678),
        ..Default::default()
    };

    run_update(&pool, fx.consignment_id, &input)
        .await
        .expect("货量/金额写入应成功");

    assert_eq!(
        derived_scale_mark(&pool, fx.consignment_id, "qk_w_qty")
            .await
            .as_deref(),
        Some("34.5"),
        "货量 MUST 按入参原值落标量 mark（MUST NOT 取整为 35）"
    );
    assert_eq!(
        derived_scale_mark(&pool, fx.consignment_id, "qk_amount")
            .await
            .as_deref(),
        Some("12345.68"),
        "金额 MUST 按分保留两位落标量 mark"
    );

    // 幂等：同值重写不增生标量行
    let before = counts(&pool, fx.consignment_id).await;
    run_update(&pool, fx.consignment_id, &input)
        .await
        .expect("重复写入应成功");
    assert_eq!(
        counts(&pool, fx.consignment_id).await,
        before,
        "重复保存 MUST NOT 增生结构行"
    );
    assert_eq!(
        derived_scale_mark(&pool, fx.consignment_id, "qk_w_qty")
            .await
            .as_deref(),
        Some("34.5")
    );

    cleanup(&pool, &fx).await;
}

/// ⑥ 承运商：写→读回一致（读侧口径）；非法引用被拒且原值不变
#[tokio::test]
async fn carrier_written_reads_back_and_invalid_reference_is_rejected() {
    let pool = connect_test_db().await;
    let fx = setup(&pool).await;
    let subjects: Vec<(i64, String)> = sqlx::query_as(
        r#"SELECT id, COALESCE(notice, '') FROM isahl."zc_id_subjects"
           WHERE deleted_at IS NULL
             AND COALESCE(code, '') NOT IN ('SUBJ-ISAH-ADMIN', 'POS-SYSTEM-ADMIN')
           ORDER BY id LIMIT 2"#,
    )
    .fetch_all(&pool)
    .await
    .expect("subjects probe");
    assert!(
        subjects.len() >= 2,
        "test db 需 ≥2 个非系统主体（承运商夹具）"
    );

    run_update(
        &pool,
        fx.consignment_id,
        &UpdateStructuresInput {
            carrier: Some(subjects[0].0.to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("写承运商应成功");
    assert_eq!(
        derived_carrier(&pool, fx.consignment_id).await.as_deref(),
        Some(subjects[0].1.as_str()),
        "承运商读回 MUST 等于所写主体（读侧口径）"
    );

    // 改指另一主体：读回随之切换且不增生结构行
    let products_before = counts(&pool, fx.consignment_id).await.1;
    run_update(
        &pool,
        fx.consignment_id,
        &UpdateStructuresInput {
            carrier: Some(subjects[1].1.clone()),
            ..Default::default()
        },
    )
    .await
    .expect("按名称改指承运商应成功");
    assert_eq!(
        derived_carrier(&pool, fx.consignment_id).await.as_deref(),
        Some(subjects[1].1.as_str()),
        "按名称写入后读回 MUST 切换到该主体"
    );
    assert_eq!(
        counts(&pool, fx.consignment_id).await.1,
        products_before,
        "改指承运商 MUST NOT 增生运输产品行"
    );

    // 非法引用：拒绝且保持原值（fail-visible，无半写）
    let err = run_update(
        &pool,
        fx.consignment_id,
        &UpdateStructuresInput {
            carrier: Some(format!("不存在的主体-{}", fx.code)),
            ..Default::default()
        },
    )
    .await
    .expect_err("非法承运商 MUST 被拒");
    assert!(
        err.contains("找不到承运商主体"),
        "错误须指明承运商解析失败，实测 {err}"
    );
    assert_eq!(
        derived_carrier(&pool, fx.consignment_id).await.as_deref(),
        Some(subjects[1].1.as_str()),
        "被拒后承运商 MUST 保持原值"
    );

    cleanup(&pool, &fx).await;
}
