//! 派车链通用读件（迁自 transport-dispatch `repositories/bom.rs`）。

use common::AliothError as ApiError;

/// 车辆号牌（**实体↔身份桥**：`zc_id_entity_rr_identity` → `zc_id_identity.identity`，分类 `plate`）
/// ——追踪 notice/comments 显示号牌而非车辆 id（批注轮 65）。
///
/// 口径（change `align-vehicle-plate-identity`）：车辆 `notice`=描述信息、`code`=序列号（车架号/VIN），
/// 二者均**不承载号牌**；旧口径（`code`/`notice` 双读 + `WZ-E2E-VH-` 前缀特判）已作废。
/// 无活动桥行 ⇒ 回退车辆 id（保持调用方「必有值」契约，不臆造号牌）。
pub async fn dispatch_vehicle_plate(
    conn: &mut sqlx::PgConnection,
    vehicle_id: i64,
) -> Result<String, ApiError> {
    // 子查询无桥行 ⇒ NULL：query_scalar 推断 O=String 时 decode NULL 报「unexpected null」
    // ⇒ 显式 Option<String> + flatten（既有实测坑，保留同形）
    let plate: Option<String> = sqlx::query_scalar::<_, Option<String>>(
    r#"SELECT (SELECT i.identity
                 FROM "isahl"."zc_id_entity_rr_identity" b
                 JOIN "isahl"."zc_id_identity" i ON i.id = b.ref_right AND i.deleted_at IS NULL
                 JOIN "isahl"."zc_id_cate-identity" c ON c.id = i.ck_category AND c.deleted_at IS NULL
                WHERE b.ref_left = v.id AND b.deleted_at IS NULL AND c.code = 'plate'
                ORDER BY b.id DESC LIMIT 1)
       FROM "isahl"."zc_id_stor-ctn-vehicle" v
       WHERE v.id = $1 AND v.deleted_at IS NULL"#,
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
    // 载体迁移（用户裁决 2026-09-21，报缺产物 R6）：容量池关系行原写读在声明语义
    // 「关联-文件↔URL」的桥表上（挪用）；合法载体 = `zc_id_prod-payload_rr_stor-container`
    // （关联-载荷↔容器，⊂ `zc_id_production_rr_storage`；mv_inventory 读父表故照常可见）。
    let existing: Option<i64> = sqlx::query_scalar(
        r#"SELECT r.ref_left
       FROM "isahl"."zc_id_prod-payload_rr_stor-container" r
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
       VALUES (isahl.gen_next_uid(419), $1, $2, 999999999, 1) RETURNING id"#,
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
    // 绑定行（容量引用）——载体迁移（2026-09-21 裁决，报缺产物 R6）：原落声明语义「文件↔URL」
    // 的桥表（挪用）→ 改落关联-载荷↔容器；id 走载体自身 uid 段（模型默认 517，原借用为 335）。
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_prod-payload_rr_stor-container"
       (id, code, notice, ref_left, ref_right, qk_p_capacity, created_by_id)
       VALUES (isahl.gen_next_uid(517), $1, $2, $3, $4, $5, 1)"#,
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
