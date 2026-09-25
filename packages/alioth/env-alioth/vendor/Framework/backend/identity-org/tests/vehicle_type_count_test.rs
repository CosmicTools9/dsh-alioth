//! 回归测试：车型目录 `vehicle_count` 不把「挂在母类上的车」计入具体型号行
//!
//! 判据来源：`openspec/specs/vehicle-form-dict` :: `vehicle-form-count-scope-excludes-ancestors`
//! （修复前：计数 SQL 含 `c.code LIKE vc.code || '-%'` 一支 ⇒ 挂在母类 `VT-X` 的车会同时计入
//!  `VT-X` 与其每个兄弟型号 `VT-X-*`，本测试第二断言即为该回归的守卫。）
//!
//! 依赖：test 库存在模型级字典表 `isahl."zc_id_cons-r-type-cate"` 与车辆表
//! `isahl."zc_id_stor-ctn-vehicle"`（模型种子已重放）。夹具全程在事务内，测试结束回滚。
//! SQL 与本 handler 共用同一常量 `VEHICLE_TYPE_LIST_SQL`（test seam，避免两处 SQL 漂移）。

use identity_org::handlers::tracking::{VehicleTypeRow, VEHICLE_TYPE_LIST_SQL};
use sqlx::{PgPool, Postgres, Transaction};

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

async fn dict_row_id(tx: &mut Transaction<'_, Postgres>) -> i64 {
    sqlx::query_scalar("SELECT isahl.gen_next_uid(65)")
        .fetch_one(&mut **tx)
        .await
        .expect("gen_next_uid(65)")
}

async fn counts(tx: &mut Transaction<'_, Postgres>) -> Vec<(String, i64)> {
    let rows: Vec<VehicleTypeRow> = sqlx::query_as(VEHICLE_TYPE_LIST_SQL)
        .fetch_all(&mut **tx)
        .await
        .expect("run vehicle-type list SQL");
    rows.into_iter()
        .map(|(_, code, _, count, _)| (code.unwrap_or_default(), count))
        .collect()
}

fn count_of(rows: &[(String, i64)], code: &str) -> i64 {
    rows.iter()
        .find(|(c, _)| c == code)
        .map(|(_, n)| *n)
        .unwrap_or_else(|| panic!("字典型行缺失: {code}"))
}

#[tokio::test]
async fn model_row_count_excludes_vehicle_bound_to_group_row() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.expect("begin tx");

    // 夹具：母类（r-form 空）+ 具体型号（r-form 带长度），两者 code 前缀相同
    let group_id = dict_row_id(&mut tx).await;
    let model_id = dict_row_id(&mut tx).await;
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_cons-r-type-cate" (id, code, notice, created_by_id, updated_by_id)
           VALUES ($1, 'VT-TESTC', '测试母类', 1, 1)"#,
    )
    .bind(group_id)
    .execute(&mut *tx)
    .await
    .expect("insert group row");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_cons-r-type-cate" (id, code, notice, created_by_id, updated_by_id, "r-form")
           VALUES ($1, 'VT-TESTC-9P9', '测试型号(9.9m)', 1, 1, '{"length":"9.9m","width":"2.4m"}'::jsonb)"#,
    )
    .bind(model_id)
    .execute(&mut *tx)
    .await
    .expect("insert model row");

    // 一辆车挂在**母类**上（lifecycle 叶表：dk 坐标三件套经字典解析，禁硬编码 ZUID）
    let vehicle_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_stor-ctn-vehicle"
               (code, notice, created_by_id, updated_by_id, "ck_r-type", dk_scene, dk_factor, dk_function)
           VALUES ('VT-COUNT-TEST-1', '计数测试车', 1, 1, $1,
                   (SELECT id FROM isahl.zc_id_scene WHERE code = 'TX' AND deleted_at IS NULL ORDER BY id LIMIT 1),
                   (SELECT id FROM isahl.zc_id_factor WHERE code = 'FJA' AND deleted_at IS NULL ORDER BY id LIMIT 1),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_GG' AND deleted_at IS NULL ORDER BY id LIMIT 1))
           RETURNING id"#,
    )
    .bind(group_id)
    .fetch_one(&mut *tx)
    .await
    .expect("insert vehicle bound to group row");

    let after_group_binding = counts(&mut tx).await;
    assert_eq!(
        count_of(&after_group_binding, "VT-TESTC"),
        1,
        "母类行聚合自身与子孙的绑定"
    );
    assert_eq!(
        count_of(&after_group_binding, "VT-TESTC-9P9"),
        0,
        "型号行 MUST NOT 计入挂在祖先（母类）上的车——修复前该值为 1（计数膨胀回归）"
    );

    // 车改挂**型号**：型号行计 1，母类行仍聚合到 1（且不重复计入）
    sqlx::query(r#"UPDATE isahl."zc_id_stor-ctn-vehicle" SET "ck_r-type" = $1 WHERE id = $2"#)
        .bind(model_id)
        .bind(vehicle_id)
        .execute(&mut *tx)
        .await
        .expect("rebind vehicle to model row");

    let after_model_binding = counts(&mut tx).await;
    assert_eq!(
        count_of(&after_model_binding, "VT-TESTC-9P9"),
        1,
        "型号行计入自身绑定"
    );
    assert_eq!(
        count_of(&after_model_binding, "VT-TESTC"),
        1,
        "母类行聚合其型号的绑定"
    );

    tx.rollback().await.expect("rollback fixture");
}
