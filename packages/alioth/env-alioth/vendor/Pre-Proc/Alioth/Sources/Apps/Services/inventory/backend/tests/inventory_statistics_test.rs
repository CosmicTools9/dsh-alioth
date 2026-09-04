//! alioth-service-inventory 集成测试（显式物化版）
//!
//! 验证库存时空伴随链路（ADR D-018）：
//! 1. voucher 写入 → 显式物化 → statistics 读到标量 mark（数量口径）
//! 2. 嵌套 rollup：置入（行存在）→ 父容器含子库存；取出 → 回退
//! 3. 盘点实值校准：明细实盘 qk_qty 记录截止值，创建时自动生成校准凭证经 apply_voucher 物化
//! 4. StorageNest 时空交叠唯一约束拦截
//! 5. 时空伴随生效期过滤：rr_storage.qk_period 未覆盖 now() → 统计过滤

use alioth_service_inventory::models::{
    CreateCountingDetailRequest, CreateCountingRequest, CreateStorageNestRequest,
    CreateVoucherRequest,
};
use alioth_service_inventory::services::{
    CountingDetailService, CountingService, StockStatQuery, StockStatService, StorageNestService,
    VoucherService,
};
use common::testing::connect_test_db;
use crud::schema_repository::SchemaRepository;
use rust_decimal::Decimal;

const TEST_PRODUCT: i64 = 9001;

/// 建一个测试储元，返回 id
async fn make_storage(pool: &sqlx::PgPool, tag: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO isahl.zc_id_storage (notice) VALUES ($1) RETURNING id")
        .bind(format!("it-{}", tag))
        .fetch_one(pool)
        .await
        .expect("insert storage")
}

/// 建一个测试时间段，返回 id
async fn make_period(pool: &sqlx::PgPool, tag: &str, offset_days: i32) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_segm-date\" (notice, date_st, date_ed) VALUES ($1, now() + ($2 || ' days')::interval, now() + (($2 + 2) || ' days')::interval) RETURNING id",
    )
    .bind(format!("it-{}", tag))
    .bind(offset_days)
    .fetch_one(pool)
    .await
    .expect("insert period")
}

#[tokio::test]
async fn voucher_writes_materialize_statistics() {
    let pool = connect_test_db().await;
    let svc = VoucherService::new(pool.clone());
    let stat = StockStatService::new(pool.clone());
    let uid: i64 = 1;

    let s1 = make_storage(&pool, "v-s1").await;

    let v = svc
        .create(
            CreateVoucherRequest {
                production_id: Some(TEST_PRODUCT),
                from_storage_id: None,
                to_storage_id: Some(s1),
                qty: None,
                income: Some(10),
                outgo: None,
            },
            uid,
        )
        .await
        .expect("create voucher");

    assert!(v.id > 0);

    let stats = stat
        .statistics(&StockStatQuery {
            production_id: Some(TEST_PRODUCT),
            storage_id: Some(s1),
        })
        .await
        .expect("statistics query");

    assert_eq!(stats.len(), 1, "应有一条物化统计");
    assert_eq!(stats[0].qty, Decimal::new(10, 0), "物化库存应=10");
    assert_eq!(stats[0].production_id, TEST_PRODUCT);
    assert_eq!(stats[0].storage_id, s1);

    svc.create(
        CreateVoucherRequest {
            production_id: Some(TEST_PRODUCT),
            from_storage_id: None,
            to_storage_id: Some(s1),
            qty: None,
            income: Some(5),
            outgo: None,
        },
        uid,
    )
    .await
    .expect("create second voucher");

    let stats = stat
        .statistics(&StockStatQuery {
            production_id: Some(TEST_PRODUCT),
            storage_id: Some(s1),
        })
        .await
        .expect("statistics query 2");
    assert_eq!(stats[0].qty, Decimal::new(15, 0), "物化库存应=15");

    svc.delete(v.id, uid).await.expect("delete voucher");
}

#[tokio::test]
async fn nest_rollup_and_takeout() {
    let pool = connect_test_db().await;
    let v_svc = VoucherService::new(pool.clone());
    let n_svc = StorageNestService::new(pool.clone());
    let stat = StockStatService::new(pool.clone());
    let uid: i64 = 1;

    let child = make_storage(&pool, "n-child").await;
    let parent = make_storage(&pool, "n-parent").await;
    let t1 = make_period(&pool, "n-t1", 0).await;

    let v = v_svc
        .create(
            CreateVoucherRequest {
                production_id: Some(TEST_PRODUCT),
                from_storage_id: None,
                to_storage_id: Some(child),
                qty: None,
                income: Some(10),
                outgo: None,
            },
            uid,
        )
        .await
        .expect("create voucher");

    // 置入（行存在即 IN，无 direction）
    let nest = n_svc
        .create(
            CreateStorageNestRequest {
                parent_id: Some(parent),
                child_id: Some(child),
                period_id: Some(t1),
            },
            uid,
        )
        .await
        .expect("create nest");

    let parent_stats = stat
        .statistics(&StockStatQuery {
            production_id: Some(TEST_PRODUCT),
            storage_id: Some(parent),
        })
        .await
        .expect("parent stats");
    assert_eq!(
        parent_stats[0].qty,
        Decimal::new(10, 0),
        "置入后父容器应含子储元库存"
    );

    // 取出：删除行 → 回退
    n_svc.delete(nest.id, uid).await.expect("delete nest");

    let parent_stats = stat
        .statistics(&StockStatQuery {
            production_id: Some(TEST_PRODUCT),
            storage_id: Some(parent),
        })
        .await
        .expect("parent stats after takeout");
    assert_eq!(
        parent_stats[0].qty,
        Decimal::new(0, 0),
        "取出后父容器应回退"
    );

    v_svc.delete(v.id, uid).await.expect("delete voucher");
}

#[tokio::test]
async fn counting_detail_auto_calibrates_via_voucher() {
    let pool = connect_test_db().await;
    let c_svc = CountingService::new(pool.clone());
    let d_svc = CountingDetailService::new(pool.clone());
    let v_svc = VoucherService::new(pool.clone());
    let stat = StockStatService::new(pool.clone());
    let uid: i64 = 1;

    let s1 = make_storage(&pool, "c-s1").await;

    // 交易凭证：入库 10 → 物化值 10
    let v = v_svc
        .create(
            CreateVoucherRequest {
                production_id: Some(TEST_PRODUCT),
                from_storage_id: None,
                to_storage_id: Some(s1),
                qty: None,
                income: Some(10),
                outgo: None,
            },
            uid,
        )
        .await
        .expect("create voucher");

    // 盘点事件头 + 明细（实盘 8，账面 10 → 盘亏 2）
    let c = c_svc
        .create(
            CreateCountingRequest {
                place_id: Some(s1),
                counted_date: None,
                tpl_id: None,
                matters: vec![],
            },
            uid,
        )
        .await
        .expect("create counting");

    let d = d_svc
        .create(
            CreateCountingDetailRequest {
                counting_id: Some(c.id),
                production_id: Some(TEST_PRODUCT),
                storage_id: Some(s1),
                voucher_id: Some(v.id),
                qty: Some(8),
                w_qty: None,
                v_qty: None,
                counted_date: None,
                biller_id: None,
            },
            uid,
        )
        .await
        .expect("create counting detail");

    // 明细实值 = 截止值 8（qk_qty 为标量引用，读标量真值）
    let actual =
        trigger_registry::stock_materialization::scalar_mark(&pool, d.qty.expect("qty scalar id"))
            .await
            .expect("read qty mark");
    assert_eq!(actual, 8.0, "明细实盘数（截止值）应=8");

    // 自动校准：生成校准凭证（盘亏 → outgo=2），物化更新 = 实盘 8
    let calib: (i64,) = sqlx::query_as(
        r#"SELECT id FROM isahl."zc_id_stat-sto-voucher"
           WHERE notice = '盘点校准' AND fk_production = $1
             AND "fk_subj-storage" = $2 AND deleted_at IS NULL
           ORDER BY id DESC LIMIT 1"#,
    )
    .bind(TEST_PRODUCT)
    .bind(s1)
    .fetch_one(&pool)
    .await
    .expect("calibration voucher exists");

    // 溯源：statement_rr_reason（ref_left=校准凭证 / ref_right=盘点明细）
    let _traced: i64 = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl."zc_id_statement_rr_reason"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(calib.0)
    .bind(d.id)
    .fetch_one(&pool)
    .await
    .expect("calibration traceable to detail");

    let stats = stat
        .statistics(&StockStatQuery {
            production_id: Some(TEST_PRODUCT),
            storage_id: Some(s1),
        })
        .await
        .expect("statistics after calibration");
    assert_eq!(
        stats[0].qty,
        Decimal::new(8, 0),
        "校准凭证自动物化：库存应=实盘 8"
    );
    assert_eq!(
        stats[0].last_counted_qty,
        Some(Decimal::new(8, 0)),
        "最近盘点实盘截止值应=8"
    );
    assert_eq!(
        stats[0].variance,
        Some(Decimal::new(0, 0)),
        "校准后账实一致：差异=0"
    );

    // 清理（逆序）：明细 → 校准凭证（冲销物化回 10）→ 事件头 → 原凭证（冲销回 0）
    d_svc.delete(d.id, uid).await.expect("delete detail");
    v_svc
        .delete(calib.0, uid)
        .await
        .expect("delete calibration voucher");
    c_svc.delete(c.id, uid).await.expect("delete counting");
    v_svc.delete(v.id, uid).await.expect("delete voucher");
}

/// P0-2 盘点事件头与规格对齐（SPEC-counting-event-as-independent-event）：
/// INSERT 不含 fk_production（该物理列已从 live DB 删除），`_refs` 按 fk_index 注册
/// （place/qk_date/storage/subject）解析，事件头读侧不得出现 fk_production。
#[tokio::test]
async fn counting_head_crud_without_fk_production() {
    let pool = connect_test_db().await;
    let c_svc = CountingService::new(pool.clone());
    let uid: i64 = 1;

    // 真实引用目标：place（fk_place → zc_id_place）
    let place_id: i64 =
        sqlx::query_scalar("INSERT INTO isahl.zc_id_place (notice) VALUES ($1) RETURNING id")
            .bind("it-counting-place")
            .fetch_one(&pool)
            .await
            .expect("insert place");

    // 事件头 INSERT 仅 fk_place/qk_date/created_by_id —— 无 fk_production 列
    let c = c_svc
        .create(
            CreateCountingRequest {
                place_id: Some(place_id),
                counted_date: None,
                tpl_id: None,
                matters: vec![],
            },
            uid,
        )
        .await
        .expect("create counting head without fk_production");
    assert!(c.id > 0);
    assert_eq!(c.place_id, Some(place_id));

    // _refs 解析：SchemaRepository.get 以 fk_index 注册为编译期源生成子查询
    let repo = SchemaRepository::new(pool.clone());
    let row = repo
        .get("zc_id_even-counting", c.id)
        .await
        .expect("get counting head with _refs")
        .expect("counting row exists");
    let refs = row
        .get("_refs")
        .and_then(|v| v.as_object())
        .expect("_refs 应为对象");
    for key in ["place", "qk_date", "storage", "subject"] {
        assert!(refs.contains_key(key), "_refs 应含 {key}");
    }
    assert!(
        !refs.contains_key("fk_production"),
        "_refs 不得含 fk_production（事件头无此列）"
    );
    let place = refs.get("place").and_then(|v| v.as_object());
    assert!(place.is_some(), "place 应解析为目标对象");
    assert!(
        place.and_then(|p| p.get("notice")).is_some(),
        "place 对象应含 notice"
    );

    // 清理（逆序）：事件头 → place
    c_svc.delete(c.id, uid).await.expect("delete counting");
    sqlx::query("DELETE FROM isahl.zc_id_place WHERE id = $1")
        .bind(place_id)
        .execute(&pool)
        .await
        .expect("delete place");
}

#[tokio::test]
async fn nest_temporal_uniqueness_blocks_overlap() {
    let pool = connect_test_db().await;
    let n_svc = StorageNestService::new(pool.clone());
    let uid: i64 = 1;

    let child = make_storage(&pool, "u-child").await;
    let parent = make_storage(&pool, "u-parent").await;
    let t1 = make_period(&pool, "u-t1", 10).await;

    let nest = n_svc
        .create(
            CreateStorageNestRequest {
                parent_id: Some(parent),
                child_id: Some(child),
                period_id: Some(t1),
            },
            uid,
        )
        .await
        .expect("first nest");

    let dup = n_svc
        .create(
            CreateStorageNestRequest {
                parent_id: Some(parent),
                child_id: Some(child),
                period_id: Some(t1),
            },
            uid,
        )
        .await;
    assert!(dup.is_err(), "时空交叠应被唯一约束拒绝");

    n_svc.delete(nest.id, uid).await.expect("cleanup");
}

#[tokio::test]
async fn temporal_effectivity_filters_statistics() {
    use alioth_service_inventory::repositories::stock_stat_repository::StockStatRepository;

    let pool = connect_test_db().await;
    let v_svc = VoucherService::new(pool.clone());
    let uid: i64 = 1;

    let s1 = make_storage(&pool, "t-s1").await;
    // 未生效时间段（未来）：date_st = now + 10 天
    let future: i64 = sqlx::query_scalar(
        "INSERT INTO isahl.\"zc_id_segm-date\" (notice, date_st, date_ed) VALUES ($1, now() + interval '10 days', now() + interval '20 days') RETURNING id",
    )
    .bind("it-t-future")
    .fetch_one(&pool)
    .await
    .expect("insert future period");

    // 上架 10（物化无条件维护——事实）
    let v = v_svc
        .create(
            CreateVoucherRequest {
                production_id: Some(TEST_PRODUCT),
                from_storage_id: None,
                to_storage_id: Some(s1),
                qty: None,
                income: Some(10),
                outgo: None,
            },
            uid,
        )
        .await
        .expect("create voucher");

    // 无生效期约束：统计可见（qk_period NULL）
    let repo = StockStatRepository::new(pool.clone());
    let rows = repo
        .statistics(Some(TEST_PRODUCT), Some(s1))
        .await
        .expect("statistics");
    assert_eq!(rows.len(), 1, "无生效期约束应可见");
    assert_eq!(rows[0].qty, Decimal::new(10, 0));

    // 绑定未来生效期：统计应被过滤（关系未生效）
    sqlx::query(
        "UPDATE isahl.\"zc_id_production_rr_storage\" SET qk_period = $1 WHERE ref_left = $2 AND ref_right = $3",
    )
    .bind(future)
    .bind(TEST_PRODUCT)
    .bind(s1)
    .execute(&pool)
    .await
    .expect("bind future period");

    let rows = repo
        .statistics(Some(TEST_PRODUCT), Some(s1))
        .await
        .expect("statistics filtered");
    assert_eq!(rows.len(), 0, "生效期未开始应被过滤");

    // 清理
    v_svc.delete(v.id, uid).await.expect("cleanup");
}

#[tokio::test]
async fn voucher_balance_chain_backfill() {
    let pool = connect_test_db().await;
    let svc = VoucherService::new(pool.clone());
    let uid: i64 = 1;

    let s1 = make_storage(&pool, "b-s1").await;

    // 第一笔：期初=0，期末=10
    let v1 = svc
        .create(
            CreateVoucherRequest {
                production_id: Some(TEST_PRODUCT),
                from_storage_id: None,
                to_storage_id: Some(s1),
                qty: None,
                income: Some(10),
                outgo: None,
            },
            uid,
        )
        .await
        .expect("create voucher 1");
    // qk_* 为标量引用（bigint 存标量 ID）：断言须 JOIN 标量表取真值（mark）
    let (pre1, bal1): (f64, f64) = sqlx::query_as(
        "SELECT CAST(COALESCE(p.mark,0) AS float8), CAST(COALESCE(b.mark,0) AS float8)
         FROM isahl.\"zc_id_stat-sto-voucher\" v
         LEFT JOIN isahl.\"zc_id_scal-common\" p ON p.id = v.qk_pre_balance
         LEFT JOIN isahl.\"zc_id_scal-common\" b ON b.id = v.qk_balance
         WHERE v.id = $1",
    )
    .bind(v1.id)
    .fetch_one(&pool)
    .await
    .expect("read balances 1");
    assert_eq!(pre1, 0.0, "期初余额应=0");
    assert_eq!(bal1, 10.0, "期末余额应=10");

    // 第二笔：期初=10，期末=15
    let v2 = svc
        .create(
            CreateVoucherRequest {
                production_id: Some(TEST_PRODUCT),
                from_storage_id: None,
                to_storage_id: Some(s1),
                qty: None,
                income: Some(5),
                outgo: None,
            },
            uid,
        )
        .await
        .expect("create voucher 2");
    let (pre2, bal2): (f64, f64) = sqlx::query_as(
        "SELECT CAST(COALESCE(p.mark,0) AS float8), CAST(COALESCE(b.mark,0) AS float8)
         FROM isahl.\"zc_id_stat-sto-voucher\" v
         LEFT JOIN isahl.\"zc_id_scal-common\" p ON p.id = v.qk_pre_balance
         LEFT JOIN isahl.\"zc_id_scal-common\" b ON b.id = v.qk_balance
         WHERE v.id = $1",
    )
    .bind(v2.id)
    .fetch_one(&pool)
    .await
    .expect("read balances 2");
    assert_eq!(pre2, 10.0, "第二笔期初应=10");
    assert_eq!(bal2, 15.0, "第二笔期末应=15");

    svc.delete(v1.id, uid).await.expect("cleanup v1");
    svc.delete(v2.id, uid).await.expect("cleanup v2");
}

/// P1-4 范例/实例回链（SPEC-counting-event-template-instance-links）：
/// 无 tpl_id → `_t_`='范例'；携带 tpl_id → `_t_`='实例' 且指向范例；读取经 `_refs`
/// 解析范例 notice。
#[tokio::test]
async fn counting_instance_links_template() {
    let pool = connect_test_db().await;
    let c_svc = CountingService::new(pool.clone());
    let uid: i64 = 1;

    // 范例（模板）：无 tpl_id
    let tpl = c_svc
        .create(
            CreateCountingRequest {
                place_id: None,
                counted_date: None,
                tpl_id: None,
                matters: vec![],
            },
            uid,
        )
        .await
        .expect("create template counting");
    assert_eq!(tpl.t.as_deref(), Some("范例"), "范例 `_t_` 应为 '范例'");
    assert_eq!(tpl.tpl_id, None, "范例无 tpl_id");

    // 实例：tpl_id → 范例
    let inst = c_svc
        .create(
            CreateCountingRequest {
                place_id: None,
                counted_date: None,
                tpl_id: Some(tpl.id),
                matters: vec![],
            },
            uid,
        )
        .await
        .expect("create instance counting");
    assert_eq!(inst.t.as_deref(), Some("实例"), "实例 `_t_` 应为 '实例'");
    assert_eq!(inst.tpl_id, Some(tpl.id), "实例 tpl_id 应指向范例");

    // 落库断言：范例行 `_t_`='范例'，实例行 `_t_`='实例' + tpl_id
    let (tpl_t, inst_t, inst_tpl): (String, String, Option<i64>) = sqlx::query_as(
        r#"SELECT a."_t_", b."_t_", b.tpl_id
           FROM isahl."zc_id_even-counting" a
           JOIN isahl."zc_id_even-counting" b ON b.tpl_id = a.id
           WHERE a.id = $1 AND b.id = $2"#,
    )
    .bind(tpl.id)
    .bind(inst.id)
    .fetch_one(&pool)
    .await
    .expect("read template/instance rows");
    assert_eq!(tpl_t, "范例");
    assert_eq!(inst_t, "实例");
    assert_eq!(inst_tpl, Some(tpl.id));

    // `_refs` 解析：实例 tpl_id → 范例 notice
    let repo = SchemaRepository::new(pool.clone());
    let row = repo
        .get("zc_id_even-counting", inst.id)
        .await
        .expect("get instance with refs")
        .expect("instance exists");
    let refs = row
        .get("_refs")
        .and_then(|v| v.as_object())
        .expect("_refs 应为对象");
    let tpl_ref = refs
        .get("template")
        .and_then(|v| v.as_object())
        .expect("_refs.template 应解析出范例对象");
    assert!(
        tpl_ref.get("notice").is_some(),
        "_refs.template 应含范例 notice"
    );

    c_svc.delete(inst.id, uid).await.expect("cleanup instance");
    c_svc.delete(tpl.id, uid).await.expect("cleanup template");
}

/// P1-4 盘点范围 m2n（SPEC-counting-event-matter-m2n-persisted）：
/// 创建时同事务写入 `zc_id_event_rr_matter`（ref_left=事件 / ref_right=产品），
/// 读取还原产品 `_refs` 对象；空范围零行。
#[tokio::test]
async fn counting_matter_m2n_persisted() {
    let pool = connect_test_db().await;
    let c_svc = CountingService::new(pool.clone());
    let uid: i64 = 1;

    let prod_b: i64 =
        sqlx::query_scalar("INSERT INTO isahl.zc_id_production (notice) VALUES ($1) RETURNING id")
            .bind("it-counting-matter-b")
            .fetch_one(&pool)
            .await
            .expect("insert product B");

    // 多产品范围（含重复 → 幂等去重）
    let c = c_svc
        .create(
            CreateCountingRequest {
                place_id: None,
                counted_date: None,
                tpl_id: None,
                matters: vec![TEST_PRODUCT, prod_b, TEST_PRODUCT],
            },
            uid,
        )
        .await
        .expect("create counting with matters");
    assert_eq!(c.matters.len(), 2, "重复产品应去重");

    // 读取还原
    let got = c_svc.get(c.id).await.expect("get counting").expect("row");
    let mut ids: Vec<i64> = got.matters.iter().map(|m| m.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![TEST_PRODUCT, prod_b], "matters 应还原 A/B");
    assert!(
        got.matters
            .iter()
            .any(|m| m.id == prod_b && m.notice.is_some()),
        "新插入产品 matter 应含 notice"
    );

    // 落库断言：rr_matter 行数 = 2，ref_left=事件 / ref_right=产品
    let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
        r#"SELECT id, ref_left, ref_right FROM isahl."zc_id_event_rr_matter"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY ref_right"#,
    )
    .bind(c.id)
    .fetch_all(&pool)
    .await
    .expect("read rr_matter rows");
    assert_eq!(rows.len(), 2, "rr_matter 应写 2 行");
    assert_eq!((rows[0].1, rows[0].2), (c.id, TEST_PRODUCT));
    assert_eq!((rows[1].1, rows[1].2), (c.id, prod_b));

    // junction `_refs`：matter → 产品对象
    let repo = SchemaRepository::new(pool.clone());
    let jrow = repo
        .get("zc_id_event_rr_matter", rows[0].0)
        .await
        .expect("get junction row");
    let jrefs = jrow
        .and_then(|v| v.get("_refs").and_then(|r| r.as_object()).cloned())
        .expect("junction _refs");
    assert!(jrefs.contains_key("matter"), "junction _refs 应含 matter");

    // 空范围：创建成功，零行
    let empty = c_svc
        .create(
            CreateCountingRequest {
                place_id: None,
                counted_date: None,
                tpl_id: None,
                matters: vec![],
            },
            uid,
        )
        .await
        .expect("create counting with empty matters");
    assert!(empty.matters.is_empty(), "空范围返回空");
    let cnt: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_event_rr_matter"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(empty.id)
    .fetch_one(&pool)
    .await
    .expect("count rr_matter");
    assert_eq!(cnt, 0, "空范围零行");

    c_svc.delete(empty.id, uid).await.expect("cleanup empty");
    c_svc.delete(c.id, uid).await.expect("cleanup counting");
    sqlx::query("DELETE FROM isahl.zc_id_production WHERE id = $1")
        .bind(prod_b)
        .execute(&pool)
        .await
        .expect("cleanup product B");
}
