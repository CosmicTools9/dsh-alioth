//! 派车链通用读件（迁自 transport-dispatch `repositories/bom.rs`）。

use common::AliothError as ApiError;

/// 车辆车牌（notice 优先，code 兜底）——追踪 comments 显示车牌而非 id（批注轮 65）
pub async fn dispatch_vehicle_plate(
    conn: &mut sqlx::PgConnection,
    vehicle_id: i64,
) -> Result<String, ApiError> {
    // 批注轮（dev 实测）：车辆 code 可为 NULL → CASE 整体 NULL → query_scalar 推断 O=String
    // 时 decode NULL 报「unexpected null」——显式 Option<String>；code NULL 时取 notice（车牌语义）
    let plate: Option<String> = sqlx::query_scalar::<_, Option<String>>(
    r#"SELECT CASE WHEN COALESCE(code, '') LIKE 'WZ-E2E-VH-%' THEN notice ELSE COALESCE(code, notice, '') END
       FROM "isahl"."zc_id_stor-ctn-vehicle"
       WHERE id = $1 AND deleted_at IS NULL"#,
)
.bind(vehicle_id)
.fetch_optional(&mut *conn)
.await?
.flatten();
    Ok(plate.unwrap_or_else(|| vehicle_id.to_string()))
}

/// 自动建默认线路容量池（用户裁决：默认容量近乎无限，平台不关注承运商运力）。
/// 幂等：已有线路容量行直接复用；否则建 CAP-LINE-{line} 产品 + 容量标量（999999999）+ 绑定行。
pub async fn ensure_default_capacity_pool(
    tx: &mut sqlx::PgConnection,
    line_id: i64,
) -> Result<i64, ApiError> {
    // 已有线路池（家族优先序与 Step 0d 同构）
    let existing: Option<i64> = sqlx::query_scalar(
        r#"SELECT r.ref_left
       FROM "isahl"."zc_id_file_rr_url" r
       JOIN "isahl"."zc_id_prod-freight_road-sales" p
         ON p.id = r.ref_left AND p.deleted_at IS NULL
       WHERE r.ref_right = $1 AND r.qk_p_capacity IS NOT NULL AND r.deleted_at IS NULL
       ORDER BY CASE WHEN p.code LIKE 'CAP-SALE-%' THEN 0
                     WHEN p.code LIKE 'CAP-TL-%' THEN 1
                     WHEN p.code LIKE 'CAP-LINE-%' THEN 2 ELSE 3 END,
                r.ref_left
       LIMIT 1"#,
    )
    .bind(line_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(pid) = existing {
        return Ok(pid);
    }
    // CAP-LINE-{line} 已存在则复用
    if let Some(pid) = sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM "isahl"."zc_id_prod-freight_road-sales"
       WHERE code = 'CAP-LINE-' || $1::text AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(line_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        return Ok(pid);
    }
    // 建容量标量（默认近乎无限）
    let cap_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_scal-common" (id, code, notice, mark, created_by_id)
       VALUES (isahl.gen_next_zuid(), $1, $2, 999999999, 1) RETURNING id"#,
    )
    .bind(format!("CAP-DEFAULT-{}", line_id))
    .bind(format!("线路 {} 默认容量", line_id))
    .fetch_one(&mut *tx)
    .await?;
    // 本体坐标（§6.12 叶表坐标声明即必须）：容量池产品 = 实现·范例·装载标准——经
    // `DkEntity::CapacityConfig` 单一坐标源解析 code→ZUID（禁硬编码 ZUID）；
    // `_f_`/`_t_` 由同一职能码经 `derive_form_type` 派生后参数绑定（禁字面量、禁省略）。
    let (dk_scene, dk_factor, dk_function) = crate::ontology::resolve_ontology_coords(
        &mut *tx,
        crate::ontology::DkEntity::CapacityConfig,
    )
    .await?;
    let (pool_form, pool_tier) = crate::ontology::DkEntity::CapacityConfig.form_type();
    // 建池产品
    let pool_id: i64 = sqlx::query_scalar(
    r#"INSERT INTO "isahl"."zc_id_prod-freight_road-sales"
       (id, code, notice, comments, fk_line, _f_, _t_, created_by_id, dk_scene, dk_factor, dk_function)
       VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, 1, $7, $8, $9)
       RETURNING id"#,
)
.bind(format!("CAP-LINE-{}", line_id))
.bind(format!("线路容量池 {}", line_id))
.bind(format!("线路 {} 默认容量池（自动建）", line_id))
.bind(line_id)
.bind(pool_form)
.bind(pool_tier)
.bind(dk_scene)
.bind(dk_factor)
.bind(dk_function)
.fetch_one(&mut *tx)
.await?;
    // 绑定行（容量引用）
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_file_rr_url"
       (id, code, notice, ref_left, ref_right, qk_p_capacity, created_by_id)
       VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, 1)"#,
    )
    .bind(format!("CAP-LINE-STO-{}", line_id))
    .bind(format!("线路 {} 默认容量绑定", line_id))
    .bind(pool_id)
    .bind(line_id)
    .bind(cap_id)
    .execute(&mut *tx)
    .await?;
    Ok(pool_id)
}
