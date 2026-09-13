//! 镜像解析与级联软删（主合约删除 → 镜像合约及其产物同事务软删）。
//!
//! 顺序约束：镜像/主合约的产品 MUST 在其 `zc_id_contract_rr_matter` 族软删**之前**解析——
//! 产品识别依赖 matter 桥（`ref_right` 命中）；桥行一经软删即无法回溯。

use sqlx::PgConnection;

use common::AliothError;

/// 经 MIR 桥解析镜像合约 id（`ref_left` = 主合约；`code LIKE 'MIR-%'` 与续约行 `RNW-%` 分流）。
///
/// MUST 在使用方软删 MIR 桥之前调用。
pub async fn resolve_mirror_ids_tx(
    conn: &mut PgConnection,
    main_contract_id: i64,
) -> Result<Vec<i64>, AliothError> {
    sqlx::query_scalar(
        r#"SELECT a.ref_right FROM "isahl"."zc_id_contract_rr_symmetry" a
           WHERE a.ref_left = $1 AND a.deleted_at IS NULL AND a.code LIKE 'MIR-%'"#,
    )
    .bind(main_contract_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)
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
