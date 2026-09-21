//! 镜像解析与级联软删（主合约删除 → 镜像合约及其产物同事务软删）。
//!
//! 顺序约束：镜像/主合约的产品 MUST 在其 `zc_id_contract_rr_matter` 族软删**之前**解析——
//! 产品识别依赖 matter 桥（`ref_right` 命中）；桥行一经软删即无法回溯。

use sqlx::PgConnection;

use common::AliothError;

/// 经 MIR 桥解析对端合约 id（正本 ↔ 镜像双向；`code LIKE 'MIR-%'` 与续约行 `RNW-%` 分流）。
///
/// 桥方向由叶子语义固化（需求性→`ref_left` / 供给性→`ref_right`，裁决 2026-09-17），
/// 正本身份与位置无关（= MIR code 首 id）——本解析按「任一端命中取另一端」双向取对端，
/// 与 [`sync_mirror_status_tx`] 的双向先例同式。
///
/// MUST 在使用方软删 MIR 桥之前调用。
pub async fn resolve_mirror_ids_tx(
    conn: &mut PgConnection,
    contract_id: i64,
) -> Result<Vec<i64>, AliothError> {
    sqlx::query_scalar(
        r#"SELECT CASE WHEN a.ref_left = $1 THEN a.ref_right ELSE a.ref_left END
           FROM "isahl"."zc_id_contract_rr_symmetry" a
           WHERE (a.ref_left = $1 OR a.ref_right = $1)
             AND a.deleted_at IS NULL AND a.code LIKE 'MIR-%'"#,
    )
    .bind(contract_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)
}

/// 合约镜像行状态桥同步（一式两份口径：镜像行与主行**同等待遇**——含状态桥；
/// 正本 `docs/specs/WZ_EXTERNAL_CHAIN_SPEC.md` §3「一式两份镜像」+ `Pre-Proc/WZ/ONTOLOGY_SPEC.md`
/// 「镜像行与主行同等待遇：行级 NGAC 注册、状态桥」）。
///
/// 把写入行的当前主状态桥（`zc_id_lifecycle_r_primary-status`，`ref_left` = 该合约行）
/// 原样复制到其一式两份对端——对端经叶子桥 `zc_id_contract_rr_symmetry` 的 `MIR-%` 行定位
///（唯一合法判据 = 叶子桥 code 前缀；`RNW-%` 续约行不匹配，MUST NOT 用 `-R` 编号后缀猜）。
/// 桥行 `ref_left`=主 / `ref_right`=镜像：**入参为任一端皆可**（主行写 → 同步镜像；镜像写 → 同步主行）。
///
/// 幂等：目标行 `ref_left` 全表唯一（含软删行）→ 一律 `ON CONFLICT (ref_left)` 原地更新/复活
///（与 `common::status::upsert_status_tx` 同范式）。无状态桥 / 无 MIR 桥 / 对端已软删 → no-op。
/// 返回同步的对端行数（0 = 无需同步）。
pub async fn sync_mirror_status_tx(
    conn: &mut PgConnection,
    contract_id: i64,
    user_id: i64,
) -> Result<u64, AliothError> {
    let affected = sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_lifecycle_r_primary-status"
             (ref_left, ref_right, code, notice, status_date, created_by_id, updated_by_id)
           SELECT partner.id, src.ref_right, src.code, '状态流转（镜像同步）',
                  src.status_date, $2, $2
           FROM "isahl"."zc_id_lifecycle_r_primary-status" src
           JOIN LATERAL (
                SELECT CASE WHEN mir.ref_left = src.ref_left
                            THEN mir.ref_right ELSE mir.ref_left END AS id
                  FROM "isahl"."zc_id_contract_rr_symmetry" mir
                 WHERE (mir.ref_left = src.ref_left OR mir.ref_right = src.ref_left)
                   AND mir.code LIKE 'MIR-%' AND mir.deleted_at IS NULL
                 ORDER BY mir.id
                 LIMIT 1
           ) partner ON TRUE
           JOIN "isahl"."zc_id_contract" c
             ON c.id = partner.id AND c.deleted_at IS NULL
           WHERE src.ref_left = $1
           ON CONFLICT (ref_left) DO UPDATE
             SET ref_right = EXCLUDED.ref_right,
                 code = EXCLUDED.code,
                 status_date = EXCLUDED.status_date,
                 updated_at = NOW(),
                 updated_by_id = EXCLUDED.updated_by_id,
                 deleted_at = NULL,
                 deleted_by_id = NULL"#,
    )
    .bind(contract_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    Ok(affected.rows_affected())
}

/// 解析合约自动创建的产品 id（识别 = 标的关系族桥 `ref_right` 且 `code = 'PRD-'||合约code`）。
///
/// MUST 在使用方软删 `zc_id_contract_rr_matter` 之前调用。
pub async fn resolve_contract_product_ids_tx(
    conn: &mut PgConnection,
    contract_id: i64,
) -> Result<Vec<i64>, AliothError> {
    sqlx::query_scalar(
        r#"SELECT p.id FROM "isahl"."zc_id_prod-traffic" p
           WHERE p.code = 'PRD-' || (SELECT code FROM "isahl"."zc_id_contract" WHERE id = $1)
             AND p.deleted_at IS NULL
             AND p.id IN (
               SELECT ref_right FROM "isahl"."zc_id_contract_rr_matter" WHERE ref_left = $1 AND deleted_at IS NULL
             )"#,
    )
    .bind(contract_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)
}

/// 合约产品级联软删（起讫桥 + 两个运输产品叶表；ids 由 [`resolve_contract_product_ids_tx`] 预先解析）。
pub async fn soft_delete_contract_products_tx(
    conn: &mut PgConnection,
    product_ids: &[i64],
    user_id: i64,
) -> Result<(), AliothError> {
    if product_ids.is_empty() {
        return Ok(());
    }
    for sql in [
        r#"UPDATE "isahl"."zc_id_prod-transport_rr_stop" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_left = ANY($2) AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_prod-freight_road-sales" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE id = ANY($2) AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_prod-freight_road-purchase" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE id = ANY($2) AND deleted_at IS NULL"#,
    ] {
        sqlx::query(sql)
            .bind(user_id)
            .bind(product_ids)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
    }
    Ok(())
}

/// 主合约级联软删：产品（+起讫桥）→ 明细/合同方/变更单/主状态桥 → 合约行。
///
/// 与 [`soft_delete_mirror_tx`] 对称（镜像额外软删 MIR 桥）；业务侧附加项（附件行、续约回退）
/// 留在 ns。产品 MUST 在其 `zc_id_contract_rr_matter` 族软删之前解析。
pub async fn soft_delete_contract_tx(
    conn: &mut PgConnection,
    contract_id: i64,
    user_id: i64,
) -> Result<(), AliothError> {
    let product_ids = resolve_contract_product_ids_tx(conn, contract_id).await?;
    soft_delete_contract_products_tx(conn, &product_ids, user_id).await?;

    for sql in [
        r#"UPDATE "isahl"."zc_id_contract_rr_matter" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_left = $2 AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_contract_rr_party" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_left = $2 AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_contract_rr_agreement" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_left = $2 AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_lifecycle_r_primary-status" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_left = $2 AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_contract" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE id = $2 AND deleted_at IS NULL"#,
    ] {
        sqlx::query(sql)
            .bind(user_id)
            .bind(contract_id)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
    }

    Ok(())
}

/// 镜像合约级联软删：镜像产品（+起讫桥）→ 合同方/明细/变更单/主状态桥 → 镜像行 → MIR 桥。
pub async fn soft_delete_mirror_tx(
    conn: &mut PgConnection,
    mirror_id: i64,
    user_id: i64,
) -> Result<(), AliothError> {
    let product_ids = resolve_contract_product_ids_tx(conn, mirror_id).await?;
    soft_delete_contract_products_tx(conn, &product_ids, user_id).await?;

    for sql in [
        r#"UPDATE "isahl"."zc_id_contract_rr_matter" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_left = $2 AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_contract_rr_party" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_left = $2 AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_contract_rr_agreement" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_left = $2 AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_lifecycle_r_primary-status" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_left = $2 AND deleted_at IS NULL"#,
        r#"UPDATE "isahl"."zc_id_contract" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE id = $2 AND deleted_at IS NULL"#,
    ] {
        sqlx::query(sql)
            .bind(user_id)
            .bind(mirror_id)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
    }

    sqlx::query(
        r#"UPDATE "isahl"."zc_id_contract_rr_symmetry" SET deleted_at = NOW(), deleted_by_id = $1
           WHERE ref_right = $2 AND deleted_at IS NULL AND code LIKE 'MIR-%'"#,
    )
    .bind(user_id)
    .bind(mirror_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;

    Ok(())
}
