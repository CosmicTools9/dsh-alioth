//! 运输产品行写件（`zc_id_prod-freight_road-{sales,purchase}`）。
//!
//! 主产品与镜像产品同构——族由 `is_sales` 定，主体对由调用方给定（镜像 = 相反族 + **同一主体对，不互换**）；
//! 业务组装（标量创建、合同桥路由、业务校验）留在 ns；起讫桥 `rr_stop` 由本 crate 的
//! `insert_product_stops_tx` 单源承载（各链共用，禁第二份同语义 SQL）。

use sqlx::PgConnection;

use common::AliothError;
use trigger_registry::lifecycle::derive_form_type;

use crate::models::{ContractProductInput, ProductPairInput, ProductRowInput};

const SALES_INSERT: &str = r#"INSERT INTO "isahl"."zc_id_prod-freight_road-sales"
   (id, code, notice, comments, "fk_subj-demand", "fk_subj-provider",
    fk_line, "ck_vehicle-form", fk_previous, dk_scene, dk_factor, dk_function,
    qk_price, qk_w_lading, qk_period, "_f_", "_t_", created_by_id,
    ak_permit_user, ak_access_user)
   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8,
           (SELECT id FROM "isahl"."zc_id_scene" WHERE code = $9 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_factor" WHERE code = $10 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_function" WHERE code = $11 LIMIT 1),
           $12, $13, $14, $15, $16, $17, ARRAY[$17]::bigint[], ARRAY[$17]::bigint[])
   RETURNING id"#;

const PURCHASE_INSERT: &str = r#"INSERT INTO "isahl"."zc_id_prod-freight_road-purchase"
   (id, code, notice, comments, "fk_subj-demand", "fk_subj-provider",
    fk_line, "ck_vehicle-form", fk_previous, dk_scene, dk_factor, dk_function,
    qk_price, qk_w_lading, qk_period, "_f_", "_t_", created_by_id,
    ak_permit_user, ak_access_user)
   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8,
           (SELECT id FROM "isahl"."zc_id_scene" WHERE code = $9 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_factor" WHERE code = $10 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_function" WHERE code = $11 LIMIT 1),
           $12, $13, $14, $15, $16, $17, ARRAY[$17]::bigint[], ARRAY[$17]::bigint[])
   RETURNING id"#;

/// 单张产品行落库。返回产品 id。
///
/// `_f_`/`_t_` 经 `derive_form_type` 从 `fn_code` 派生绑定（单一派生源，调用方不传形态字面量）。
pub async fn insert_product_row_tx(
    conn: &mut PgConnection,
    input: &ProductRowInput<'_>,
) -> Result<i64, AliothError> {
    let (form, tier) = derive_form_type(input.fn_code).ok_or_else(|| AliothError::Validation {
        field: "fnCode".into(),
        message: format!(
            "职能码 {} 无法派生 _f_/_t_（须为 !./!_/↑./↑_/↓./↓_ 六前缀之一）",
            input.fn_code
        ),
    })?;

    let sql = if input.is_sales {
        SALES_INSERT
    } else {
        PURCHASE_INSERT
    };
    let id: i64 = sqlx::query_scalar(sql)
        .bind(input.code)
        .bind(input.notice)
        .bind(input.comments)
        .bind(input.demand_subject)
        .bind(input.provider_subject)
        .bind(input.line_id)
        .bind(input.vehicle_form_id)
        .bind(input.previous_id)
        .bind(input.scene_code)
        .bind(input.factor_code)
        .bind(input.fn_code)
        .bind(input.price_id)
        .bind(input.weight_id)
        .bind(input.period_id)
        .bind(form)
        .bind(tier)
        .bind(input.user_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?;
    Ok(id)
}

/// 合同驱动的运输产品**组装写件**（单一实现）：产品行 + 起讫桥 ×2 + 合同桥（按方向/形态路由）。
///
/// 业务校验（地点存在性）、标量创建与时段派生留在调用方；本函数只承载可静态判定的写件，
/// 保证产品 `code` 约定、起讫桥形态与桥路由在跨 ns 单源。
pub async fn create_contract_transport_product_tx(
    conn: &mut PgConnection,
    input: &ContractProductInput<'_>,
) -> Result<i64, AliothError> {
    let product_code = format!("PRD-{}", input.contract_code);
    let product_id = insert_product_row_tx(
        conn,
        &ProductRowInput {
            is_sales: input.is_sales,
            code: &product_code,
            notice: input.notice,
            comments: input.comments,
            demand_subject: input.demand_subject,
            provider_subject: input.provider_subject,
            line_id: Some(input.line_id),
            vehicle_form_id: Some(input.vehicle_form_id),
            price_id: input.price_id,
            weight_id: input.weight_id,
            period_id: input.period_id,
            previous_id: None,
            fn_code: input.fn_code,
            scene_code: input.scene_code,
            factor_code: input.factor_code,
            user_id: input.user_id,
        },
    )
    .await?;

    // 起讫桥（rr_stop：ref_left=产品, ref_right=场所, ck_category=出发/到达）
    insert_product_stops_tx(
        conn,
        product_id,
        &product_code,
        Some(input.origin_place_id),
        Some(input.dest_place_id),
        input.user_id,
    )
    .await?;

    // 合同桥挂接（销售 master→goods / 销售 single→deal / 采购→demand）
    let bridge_sql = if input.is_sales && !input.is_single {
        BRIDGE_GOODS
    } else if input.is_sales {
        BRIDGE_DEAL
    } else {
        BRIDGE_DEMAND
    };
    sqlx::query(bridge_sql)
        .bind(input.contract_id)
        .bind(product_id)
        .bind(input.user_id)
        .bind(input.valid_segm_id)
        .execute(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?;

    Ok(product_id)
}

const STOP_INSERT: &str = r#"INSERT INTO "isahl"."zc_id_prod-transport_rr_stop"
   (id, code, ref_left, ref_right, ck_category, created_by_id)
   SELECT isahl.gen_next_zuid(), $1, $2, $3,
          (SELECT id FROM "isahl"."zc_id_cate-traffic" WHERE code = $4 LIMIT 1),
          $5"#;

/// 为运输产品写起讫停靠桥 ×2（起 = `ST-DEPART` / 讫 = `ST-ARRIVE`）。
///
/// 产品写件单源——合同驱动（`create_contract_transport_product_tx`）、委托富链
/// （`consignment-writer`）、询价/报价/成交（`transport-dispatch procure`）共用；
/// 桥 `code` 由产品 code 派生（`STOP-{product_code}-D/-A`，与既有 `STOP-PRD-*` 约定同构），
/// 各 ns MUST NOT 保留同语义的第二份 SQL（`REUSE_FIRST_SPEC`）。
/// 地点缺失（`None` / 非正）即跳过、不报错（对齐既有「无站点则跳过」风格）。
pub async fn insert_product_stops_tx(
    conn: &mut PgConnection,
    product_id: i64,
    product_code: &str,
    origin_place_id: Option<i64>,
    dest_place_id: Option<i64>,
    user_id: i64,
) -> Result<(), AliothError> {
    for (suffix, place_id, cate_code) in [
        ("D", origin_place_id, "ST-DEPART"),
        ("A", dest_place_id, "ST-ARRIVE"),
    ] {
        let Some(place_id) = place_id.filter(|pid| *pid > 0) else {
            continue;
        };
        sqlx::query(STOP_INSERT)
            .bind(format!("STOP-{product_code}-{suffix}"))
            .bind(product_id)
            .bind(place_id)
            .bind(cate_code)
            .bind(user_id)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
    }
    Ok(())
}

const BRIDGE_GOODS: &str = r#"INSERT INTO "isahl"."zc_id_contract_rr_goods"
   (ref_left, ref_right, code, notice, qk_period, created_by_id)
   VALUES ($1, $2, 'GOODS', '标的产品（自动创建）', $4, $3)"#;

const BRIDGE_DEAL: &str = r#"INSERT INTO "isahl"."zc_id_contract_rr_deal"
   (ref_left, ref_right, code, notice, qk_period, created_by_id)
   VALUES ($1, $2, 'DEAL', '成交产品（自动创建）', $4, $3)"#;

const BRIDGE_DEMAND: &str = r#"INSERT INTO "isahl"."zc_id_contract_rr_demand"
   (ref_left, ref_right, code, notice, qk_period, created_by_id)
   VALUES ($1, $2, 'DEMAND', '规范需求（自动创建）', $4, $3)"#;

/// 运输产品**成对**落库（一式两份）：主产品（`is_sales` 定族）+ 镜像产品（相反族、`{code}-R`），
/// **同一买卖主体对（不互换）**、同线路/标量，各自 `fk_previous` 挂各自单据。返回 `(主产品 id, 镜像产品 id)`。
pub async fn insert_product_pair_tx(
    conn: &mut PgConnection,
    input: &ProductPairInput<'_>,
) -> Result<(i64, i64), AliothError> {
    let main_id = insert_product_row_tx(
        conn,
        &ProductRowInput {
            is_sales: input.is_sales,
            code: input.code,
            notice: input.notice,
            comments: input.comments,
            demand_subject: input.demand_subject,
            provider_subject: input.provider_subject,
            line_id: input.line_id,
            vehicle_form_id: input.vehicle_form_id,
            price_id: input.price_id,
            weight_id: input.weight_id,
            period_id: input.period_id,
            previous_id: input.previous_main,
            fn_code: input.fn_code,
            scene_code: input.scene_code,
            factor_code: input.factor_code,
            user_id: input.user_id,
        },
    )
    .await?;

    let mirror_code = format!("{}-R", input.code);
    let mirror_id = insert_product_row_tx(
        conn,
        &ProductRowInput {
            is_sales: !input.is_sales,
            code: &mirror_code,
            notice: input.mirror_notice.unwrap_or(input.notice),
            comments: input.mirror_comments.unwrap_or(input.comments),
            demand_subject: input.demand_subject,
            provider_subject: input.provider_subject,
            line_id: input.line_id,
            vehicle_form_id: input.vehicle_form_id,
            price_id: input.price_id,
            weight_id: input.weight_id,
            period_id: input.period_id,
            previous_id: input.previous_mirror,
            fn_code: input.fn_code,
            scene_code: input.scene_code,
            factor_code: input.factor_code,
            user_id: input.user_id,
        },
    )
    .await?;

    Ok((main_id, mirror_id))
}
