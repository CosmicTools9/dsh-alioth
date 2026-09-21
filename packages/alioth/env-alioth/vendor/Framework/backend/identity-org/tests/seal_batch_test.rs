//! identity-org 铅封批量创建集成测试（refactor-dispatch-seal-code-generation）
//!
//! 覆盖：code 前缀自动续号（空前缀首建 / 续建接号 / count 缺省 1）
//! + 显式 startCode 冲突 400。依赖：test 库存在 "isahl"."zc_id_devi-seal"。

use crud::repository::AliothRepository;
use identity_org::models::{CreateSealBatchRequest, CreateSealRequest, UpdateSealRequest};
use identity_org::repository::SealRepository;
use sqlx::PgPool;

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = PgPool::connect(&url).await.expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    pool
}

/// 动态测试前缀（纳秒派生，跨运行不冲突；仅字母数字——batch 前缀约束）
fn prefix(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("TS{tag}{:x}", nanos % 0xFFFF_FFFF)
}

/// 字典类型行自愈 ensure（`zc_id_cate-seal`；类型码驱动 ck_category 与缺省批量规模）
async fn ensure_seal_type(pool: &PgPool, code: &str, notice: &str, o_number: Option<&str>) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_cate-seal" (code, notice, o_number)
           VALUES ($1, $2, $3) RETURNING id"#,
    )
    .bind(code)
    .bind(notice)
    .bind(o_number)
    .fetch_one(pool)
    .await
    .expect("ensure seal type")
}

/// 清理本用例自建的类型行
async fn drop_seal_type(pool: &PgPool, code: &str) {
    let _ = sqlx::query(r#"DELETE FROM "isahl"."zc_id_cate-seal" WHERE code = $1"#)
        .bind(code)
        .execute(pool)
        .await;
}

/// 批量请求（`sealType` = 字典类型码；`codePrefix` = 编号前缀；`startCode` 优先于前缀）
fn req(
    seal_type: &str,
    code_prefix: Option<String>,
    count: Option<i64>,
    start_code: Option<String>,
) -> CreateSealBatchRequest {
    CreateSealBatchRequest {
        seal_type: Some(seal_type.to_string()),
        code_prefix,
        start_code,
        count,
        notice: None,
        comments: None,
        waybill_id: None,
    }
}

async fn cleanup(pool: &PgPool, prefix: &str) {
    sqlx::query(r#"DELETE FROM "isahl"."zc_id_devi-seal" WHERE code LIKE $1"#)
        .bind(format!("{prefix}-%"))
        .execute(pool)
        .await
        .expect("cleanup");
}

#[tokio::test]
async fn seal_batch_auto_sequential() {
    let pool = test_pool().await;
    let repo = SealRepository::new(pool.clone());
    let pfx = prefix("A");
    // 类型码 = 自建字典行（`o_number=1` ⇒ 缺省批量规模 1）；编号前缀走 codePrefix（不依赖字典）
    let seal_type = format!("TS{pfx}");
    ensure_seal_type(&pool, &seal_type, "测试类型-自动续号", Some("1")).await;
    cleanup(&pool, &pfx).await;

    // 首建 3 条连号：空前缀从 0001 起等宽 4 位
    let items = repo
        .batch_create(req(&seal_type, Some(pfx.clone()), Some(3), None), 1)
        .await
        .expect("first batch");
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].code, Some(format!("{pfx}-0001")));
    assert_eq!(items[1].code, Some(format!("{pfx}-0002")));
    assert_eq!(items[2].code, Some(format!("{pfx}-0003")));

    // 续建 2 条：从现有最大序号 +1 续号
    let more = repo
        .batch_create(req(&seal_type, Some(pfx.clone()), Some(2), None), 1)
        .await
        .expect("second batch");
    assert_eq!(more[0].code, Some(format!("{pfx}-0004")));
    assert_eq!(more[1].code, Some(format!("{pfx}-0005")));

    // count 缺省：取类型字典 `o_number`（本例 =1 ⇒ 单号）
    let single = repo
        .batch_create(req(&seal_type, Some(pfx.clone()), None, None), 1)
        .await
        .expect("single");
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].code, Some(format!("{pfx}-0006")));

    cleanup(&pool, &pfx).await;
    drop_seal_type(&pool, &seal_type).await;
}

#[tokio::test]
async fn seal_batch_explicit_start_conflict_rejected() {
    let pool = test_pool().await;
    let repo = SealRepository::new(pool.clone());
    let pfx = prefix("B");
    let seal_type = format!("TS{pfx}");
    ensure_seal_type(&pool, &seal_type, "测试类型-显式起始号", Some("1")).await;
    cleanup(&pool, &pfx).await;

    // 种子 1 条（自动续号 → {pfx}-0001）
    repo.batch_create(req(&seal_type, Some(pfx.clone()), Some(1), None), 1)
        .await
        .expect("seed");

    // 显式 startCode 撞现有号 → 400 且消息含冲突号
    let err = repo
        .batch_create(
            req(
                &seal_type,
                Some(pfx.clone()),
                Some(1),
                Some(format!("{pfx}-0001")),
            ),
            1,
        )
        .await;
    match err {
        Err(common::AliothError::BadRequest(msg)) => {
            assert!(msg.contains("铅封号已被使用"), "unexpected msg: {msg}");
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }
    drop_seal_type(&pool, &seal_type).await;
    cleanup(&pool, &pfx).await;
}

/// P3（seal↔waybill 载体迁移）：关联运单编号落既有文本载荷列 `projection`——
/// `comments` 保持自由文本（不再承载 JSON）；列表/详情读回一致；非法运单 id 明确 400。
#[tokio::test]
async fn seal_waybill_lands_in_projection_not_comments() {
    use common::data::ListQuery;

    let pool = test_pool().await;
    let pfx = prefix("WBC");
    // 批量类型（`o_number` 非正整数亦可——本例显式给 count）
    let seal_type = format!("TS{pfx}");
    ensure_seal_type(&pool, &seal_type, "测试类型-运单载体", None).await;
    // 自查运单夹具：不借用库内在册运单（dev 对齐后的 test 库无带 code 的运单行 →
    // `fetch_one` RowNotFound）。写法与本文件两跳桥用例同款（坐标经 scene/factor/function
    // code 子查询解析，禁硬编码 ZUID）。
    let (waybill_id, waybill_code): (i64, String) = sqlx::query_as(
        r#"INSERT INTO "isahl"."zc_id_orde-land"
             (id, code, notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, 'P3 封签载体', 1,
                   (SELECT id FROM "isahl"."zc_id_scene"    WHERE code = 'TD'  AND deleted_at IS NULL),
                   (SELECT id FROM "isahl"."zc_id_factor"   WHERE code = 'FJA' AND deleted_at IS NULL),
                   (SELECT id FROM "isahl"."zc_id_function" WHERE code = '↓_GG' AND deleted_at IS NULL))
           RETURNING id, code"#,
    )
    .bind(format!("{pfx}-WB"))
    .fetch_one(&pool)
    .await
    .expect("自查运单夹具");
    let missing_waybill: i64 =
        sqlx::query_scalar(r#"SELECT COALESCE(MAX(id), 0) + 1 FROM "isahl"."zc_id_orde-land""#)
            .fetch_one(&pool)
            .await
            .expect("缺失运单 id");

    let repo = SealRepository::new(pool.clone());
    let single = |code: String, waybill_id: Option<i64>| CreateSealRequest {
        notice: Some("P3 运单载体".to_string()),
        code: Some(code),
        comments: Some("自由文本备注".to_string()),
        seal_type: None,
        waybill_id,
    };

    // ① 创建：comments 原文本 / projection 编号
    let seal = repo
        .create(single(format!("{pfx}-0001"), Some(waybill_id)), 1)
        .await
        .expect("创建封签");
    let (comments, projection): (Option<String>, Option<String>) = sqlx::query_as(
        r#"SELECT comments, projection FROM "isahl"."zc_id_devi-seal" WHERE id = $1"#,
    )
    .bind(seal.id)
    .fetch_one(&pool)
    .await
    .expect("回读列");
    assert_eq!(
        comments.as_deref(),
        Some("自由文本备注"),
        "comments 须保持自由文本"
    );
    assert!(
        !comments.unwrap_or_default().contains("waybill_id"),
        "comments 不得承载 waybill_id JSON"
    );
    assert_eq!(
        projection.as_deref(),
        Some(waybill_code.as_str()),
        "projection 须为运单编号"
    );

    // ② 读回：列表与详情（同 SELECT_FIELDS）直出编号
    let list = repo
        .list(&ListQuery {
            page: 1,
            page_size: 200,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        })
        .await
        .expect("列表");
    let row = list
        .items
        .iter()
        .find(|s| s.id == seal.id)
        .expect("列表含新行");
    assert_eq!(
        row.waybill_no.as_deref(),
        Some(waybill_code.as_str()),
        "列表 waybill_no 须回读 projection"
    );
    let detail = repo.get_refs(seal.id).await.expect("详情").expect("行存在");
    assert_eq!(
        detail.waybill_no.as_deref(),
        Some(waybill_code.as_str()),
        "详情 waybill_no 须回读 projection"
    );

    // ③ 批量创建：同一载体
    let batch = repo
        .batch_create(
            CreateSealBatchRequest {
                seal_type: Some(seal_type.clone()),
                code_prefix: Some(format!("{pfx}B")),
                start_code: None,
                count: Some(1),
                notice: Some("P3 批量".to_string()),
                comments: Some("批量备注".to_string()),
                waybill_id: Some(waybill_id),
            },
            1,
        )
        .await
        .expect("批量创建");
    let batch_projection: Option<String> =
        sqlx::query_scalar(r#"SELECT projection FROM "isahl"."zc_id_devi-seal" WHERE id = $1"#)
            .bind(batch[0].id)
            .fetch_one(&pool)
            .await
            .expect("批量回读");
    assert_eq!(
        batch_projection.as_deref(),
        Some(waybill_code.as_str()),
        "批量 projection 须为运单编号"
    );
    assert_eq!(
        batch[0].comments.as_deref(),
        Some("批量备注"),
        "批量 comments 须保持自由文本"
    );

    // ④ 更新：运单传入 → projection 跟随；回读不陈旧
    let updated = repo
        .update(
            seal.id,
            UpdateSealRequest {
                notice: None,
                code: None,
                comments: Some("改后备注".to_string()),
                seal_type: None,
                waybill_id: Some(waybill_id),
            },
            1,
        )
        .await
        .expect("更新")
        .expect("行存在");
    assert_eq!(
        updated.waybill_no.as_deref(),
        Some(waybill_code.as_str()),
        "更新后 waybill_no 须回读 projection"
    );
    assert_eq!(
        updated.comments.as_deref(),
        Some("改后备注"),
        "更新后 comments 须为自由文本"
    );

    // ⑤ 非法运单 id → 400（不静默丢弃）
    let bad = repo
        .create(single(format!("{pfx}-0002"), Some(missing_waybill)), 1)
        .await;
    match bad {
        Err(common::AliothError::BadRequest(msg)) => {
            assert!(msg.contains("关联运单不存在"), "unexpected msg: {msg}")
        }
        other => panic!("expected BadRequest, got {other:?}"),
    }

    cleanup(&pool, &pfx).await;
    cleanup(&pool, &format!("{pfx}B")).await;
    let _ = sqlx::query(r#"DELETE FROM "isahl"."zc_id_orde-land" WHERE code = $1"#)
        .bind(format!("{pfx}-WB"))
        .execute(&pool)
        .await;
    drop_seal_type(&pool, &seal_type).await;
}

/// P3b（真结构路径）：封签的运单 = **运输订单↔铅封直连桥**推导
/// （seal → `zc_id_orde-traffic_rr_devi-seal`（ref_left = 运输订单）→ 运单 code）；
/// `projection` 仅在无桥时回退。桥由 transport-operations 装/卸车组装写侧落库，
/// 本测在 test 库构造等价行验证读侧优先级（写侧端到端见 transport-operations
/// `three_info_assembles_load_then_unload_with_confirm_code`）。
#[tokio::test]
async fn seal_waybill_prefers_order_bridge_over_projection() {
    use common::data::ListQuery;

    let pool = test_pool().await;
    let pfx = prefix("WBH");
    let repo = SealRepository::new(pool.clone());

    // 两个自查运单：桥侧（结构路径指向）/ 投影侧（手工关联）
    let (proj_wb_id, proj_wb_code): (i64, String) = sqlx::query_as(
        r#"INSERT INTO "isahl"."zc_id_orde-land"
             (id, code, notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, '两跳桥读侧-投影', 1,
                   (SELECT id FROM "isahl"."zc_id_scene"    WHERE code = 'TD'  AND deleted_at IS NULL),
                   (SELECT id FROM "isahl"."zc_id_factor"   WHERE code = 'FJA' AND deleted_at IS NULL),
                   (SELECT id FROM "isahl"."zc_id_function" WHERE code = '↓_GG' AND deleted_at IS NULL))
           RETURNING id, code"#,
    )
    .bind(format!("{pfx}-WB-PROJ"))
    .fetch_one(&pool)
    .await
    .expect("投影侧运单");
    let (bridge_wb_id, bridge_wb_code): (i64, String) = sqlx::query_as(
        r#"INSERT INTO "isahl"."zc_id_orde-land"
             (id, code, notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, '两跳桥读侧-桥', 1,
                   (SELECT id FROM "isahl"."zc_id_scene"    WHERE code = 'TD'  AND deleted_at IS NULL),
                   (SELECT id FROM "isahl"."zc_id_factor"   WHERE code = 'FJA' AND deleted_at IS NULL),
                   (SELECT id FROM "isahl"."zc_id_function" WHERE code = '↓_GG' AND deleted_at IS NULL))
           RETURNING id, code"#,
    )
    .bind(format!("{pfx}-WB-BRIDGE"))
    .fetch_one(&pool)
    .await
    .expect("桥侧运单");

    // 封签：手工关联落 projection（投影侧运单）
    let seal = repo
        .create(
            CreateSealRequest {
                notice: Some("订单直连桥优先".to_string()),
                code: Some(format!("{pfx}-0001")),
                comments: None,
                seal_type: None,
                waybill_id: Some(proj_wb_id),
            },
            1,
        )
        .await
        .expect("创建封签");

    // ① 无桥 → 回退 projection
    let detail = repo.get_refs(seal.id).await.expect("详情").expect("行存在");
    assert_eq!(
        detail.waybill_no.as_deref(),
        Some(proj_wb_code.as_str()),
        "无桥时应回退 projection"
    );

    // ② 构桥：运输订单 ↔ 铅封**直连桥**（照 transport-operations 组装写侧口径）
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_orde-traffic_rr_devi-seal"
           (id, ref_left, ref_right, created_by_id)
           VALUES (isahl.gen_next_uid(499), $1, $2, 1)"#,
    )
    .bind(bridge_wb_id)
    .bind(seal.id)
    .execute(&pool)
    .await
    .expect("订单↔铅封直连桥");

    // ③ 有桥 → 结构路径优先于 projection（详情 + 列表同口径）
    let detail = repo.get_refs(seal.id).await.expect("详情").expect("行存在");
    assert_eq!(
        detail.waybill_no.as_deref(),
        Some(bridge_wb_code.as_str()),
        "直连桥应优先于 projection（详情）"
    );
    let list = repo
        .list(&ListQuery {
            page: 1,
            page_size: 200,
            filter_field: None,
            filter_op: None,
            filter_value: None,
            sort_field: None,
            sort_order: None,
        })
        .await
        .expect("列表");
    let row = list
        .items
        .iter()
        .find(|s| s.id == seal.id)
        .expect("列表含新行");
    assert_eq!(
        row.waybill_no.as_deref(),
        Some(bridge_wb_code.as_str()),
        "直连桥应优先于 projection（列表）"
    );

    // 清理：桥 → 封签 → 运单
    let _ = sqlx::query(
        r#"DELETE FROM "isahl"."zc_id_orde-traffic_rr_devi-seal" WHERE ref_right = $1"#,
    )
    .bind(seal.id)
    .execute(&pool)
    .await;
    cleanup(&pool, &pfx).await;
    for code in [format!("{pfx}-WB-PROJ"), format!("{pfx}-WB-BRIDGE")] {
        let _ = sqlx::query(r#"DELETE FROM "isahl"."zc_id_orde-land" WHERE code = $1"#)
            .bind(code)
            .execute(&pool)
            .await;
    }
}
