//! 号牌身份桥写径集成测试（change `align-storage-issuer-and-holding` §D3/§D4）。
//!
//! 被测单元 = `plates::create_vehicle_plate`（`/vehicles/{id}/plates` 与车辆写径的唯一实现）：
//! ① 同号牌跨车**重叠期间** → 拒（`Conflict`，既有 409 语义）——判重域已由「同车辆」改为
//!    「同身份行全局 + 生效期不重叠」；
//! ② 同号牌**不重叠期间** → 允许（换牌 / 历史段），且**复用**同一身份行（同值同分类不新建行）；
//! ③ 无期间 = 覆盖全部时间 `[-∞,+∞]`：与既有有界绑定必然重叠 ⇒ 拒。
//!
//! 依赖：test 库存在 `zc_id_stor-ctn-vehicle` / `zc_id_identity` / `zc_id_cate-identity`
//! （code='plate'）/ `zc_id_entity_rr_identity` / `zc_id_segm-date` 与坐标声明 JE/FJA/↑_DA
//! （及车辆坐标声明）。自清理：尾部硬删（桥 → 期间段 → 身份行 → 车辆）。

use chrono::{DateTime, Duration, Utc};
use common::testing::connect_test_db;
use common::AliothError;
use identity_org::plates::{create_vehicle_plate, PlateInput};
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// 唯一号牌（GA 36-2018 新能源小型形态：省简称 + 发牌机关字母 + `D` + 5 位数字）
fn unique_plate() -> String {
    format!("蒙BD{:05}", (nanos() / 97) % 100_000)
}

/// 建测试车辆（自建自清），返回车辆 id。
///
/// 坐标三元组 = `coords_for_entity("Vehicle")` 的取值 `GC/FJA/↓_GG`（实现·实例），
/// 与写径（`repository/vehicle.rs`）同源；值经 `ontology_binding::resolve` 解析 code→ZUID。
async fn create_vehicle(pool: &PgPool) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("GC", "FJA", "↓_GG"))
        .await
        .expect("resolve vehicle coords");
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_stor-ctn-vehicle"
               (code, notice, "ck_r-type", created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2,
                   (SELECT id FROM isahl."zc_id_cons-r-type-cate" WHERE deleted_at IS NULL
                    ORDER BY id LIMIT 1), 1, $3, $4, $5)
           RETURNING id"#,
    )
    .bind(format!("PLATE-TEST-{}", nanos()))
    .bind("号牌身份桥测试车")
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert test vehicle")
}

/// 该号牌的活动身份行 id（同值 + 同分类 `plate`）
async fn live_plate_identities(pool: &PgPool, plate: &str) -> Vec<i64> {
    sqlx::query_scalar(
        r#"SELECT i.id FROM isahl."zc_id_identity" i
           JOIN isahl."zc_id_cate-identity" c ON c.id = i.ck_category AND c.code = 'plate'
           WHERE upper(i.identity) = upper($1) AND i.deleted_at IS NULL ORDER BY i.id"#,
    )
    .bind(plate)
    .fetch_all(pool)
    .await
    .expect("select live plate identities")
}

/// 该车的活动号牌桥（`(ref_right=身份行, qk_period)`）
async fn live_bridges(pool: &PgPool, vehicle_id: i64) -> Vec<(i64, Option<i64>)> {
    sqlx::query_as(
        r#"SELECT ref_right, qk_period FROM isahl."zc_id_entity_rr_identity"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY id"#,
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await
    .expect("select live plate bridges")
}

/// 尾部硬删（桥 → 期间段 → 身份行 → 车辆），测试库零残留
async fn cleanup(pool: &PgPool, vehicles: &[i64], plate: &str) {
    let mut periods: Vec<i64> = Vec::new();
    for vehicle_id in vehicles {
        periods.extend(
            sqlx::query_scalar::<_, i64>(
                r#"SELECT qk_period FROM isahl."zc_id_entity_rr_identity"
                   WHERE ref_left = $1 AND qk_period IS NOT NULL"#,
            )
            .bind(vehicle_id)
            .fetch_all(pool)
            .await
            .expect("select period ids"),
        );
        sqlx::query(r#"DELETE FROM isahl."zc_id_entity_rr_identity" WHERE ref_left = $1"#)
            .bind(vehicle_id)
            .execute(pool)
            .await
            .expect("delete plate bridges");
    }
    for period_id in periods {
        sqlx::query(r#"DELETE FROM isahl."zc_id_segm-date" WHERE id = $1"#)
            .bind(period_id)
            .execute(pool)
            .await
            .expect("delete period");
    }
    sqlx::query(r#"DELETE FROM isahl."zc_id_identity" WHERE upper(identity) = upper($1)"#)
        .bind(plate)
        .execute(pool)
        .await
        .expect("delete plate identity");
    for vehicle_id in vehicles {
        sqlx::query(r#"DELETE FROM isahl."zc_id_stor-ctn-vehicle" WHERE id = $1"#)
            .bind(vehicle_id)
            .execute(pool)
            .await
            .expect("delete test vehicle");
    }
}

fn input(plate: &str, from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>>) -> PlateInput {
    PlateInput {
        plate: plate.to_string(),
        description: None,
        valid_from: from,
        valid_to: to,
    }
}

/// ① 重叠期间（同号牌跨车）被拒；② 不重叠期间可绑且复用同一身份行；③ 无期间 = 全覆盖
#[tokio::test]
async fn overlapping_binding_rejected_and_non_overlapping_reuses_identity_row() {
    let pool = connect_test_db().await;
    let v1 = create_vehicle(&pool).await;
    let v2 = create_vehicle(&pool).await;
    let plate = unique_plate();
    let t0: DateTime<Utc> = Utc::now();

    let mut conn = pool.acquire().await.expect("acquire conn");

    // V1：有界期间 [t0, t0+30d]
    let (_, first_identity) = create_vehicle_plate(
        &mut conn,
        v1,
        1,
        &input(&plate, Some(t0), Some(t0 + Duration::days(30))),
    )
    .await
    .expect("V1 首次绑定");

    // ① V2 同窗口 ⇒ 与 V1 期间重叠：拒（既有 409 语义）
    let err = create_vehicle_plate(
        &mut conn,
        v2,
        1,
        &input(&plate, Some(t0), Some(t0 + Duration::days(30))),
    )
    .await
    .expect_err("重叠期间必须被拒");
    assert!(
        matches!(err, AliothError::Conflict(_)),
        "重叠须 409 Conflict，实得 {err:?}"
    );

    // ①′ 无期间（= 覆盖全部时间 `[-∞,+∞]`）与既有有界绑定重叠：同样拒
    let err = create_vehicle_plate(&mut conn, v2, 1, &input(&plate, None, None))
        .await
        .expect_err("无期间 = 全覆盖，必然重叠，必须被拒");
    assert!(
        matches!(err, AliothError::Conflict(_)),
        "全覆盖须 409 Conflict，实得 {err:?}"
    );

    // ② 不重叠窗口 [t0+31d, t0+60d] ⇒ 允许，且复用 V1 的身份行（不新建同值行）
    let (_, second_identity) = create_vehicle_plate(
        &mut conn,
        v2,
        1,
        &input(
            &plate,
            Some(t0 + Duration::days(31)),
            Some(t0 + Duration::days(60)),
        ),
    )
    .await
    .expect("不重叠期间必须可绑");
    assert_eq!(
        second_identity, first_identity,
        "同值同分类活动身份行必须复用（MUST NOT 新建第二行）"
    );

    let identities = live_plate_identities(&pool, &plate).await;
    assert_eq!(
        identities,
        vec![first_identity],
        "同号牌活动身份行恰 1 行，实得 {identities:?}"
    );
    assert_eq!(live_bridges(&pool, v1).await.len(), 1, "V1 活动桥恰 1 条");
    let v2_bridges = live_bridges(&pool, v2).await;
    assert_eq!(v2_bridges.len(), 1, "V2 活动桥恰 1 条");
    assert_eq!(
        v2_bridges[0].0, first_identity,
        "V2 桥须引用复用后的同一身份行"
    );
    assert!(v2_bridges[0].1.is_some(), "V2 桥须带生效期段");

    cleanup(&pool, &[v1, v2], &plate).await;
}
