//! 委托写链事务（create_full）——迁移自 transport-dispatch
//! `repositories::FjdRepository::create_consignment_inner`（原 600+ 行表级全链，行为等价迁移）：
//!
//! Step 顺序：货描校验 → TrafficLine 校验 → scal-weight/amount → orde-land 主档（+rr_contract 幂等桥）
//! → segm-date 时间窗 → scal-price（成交单价）→ freight_road-sales（+request + prod-sales 桥）
//! → rr_stop×2（ST-DEPART/ARRIVE）→ prod-loading → storage 运力占用 → com-voucher 凭证（守卫）
//! → deta-trade_order（DTL + DTL-LDG）→ 货描标签桥。
//!
//! 主体语义参数化：`customer_id`/运营组织解析外置（WriteContext），事务内不再按 user 解析组织。

use crate::coords::{resolve_coords, WriterDk};
use crate::mirror::{insert_order_mirror_tx, OrderMirrorInput};
use crate::models::{CreateConsignmentInput, CreateConsignmentOutput, WriteContext};
use common::AliothError as WriterError;
use common::AliothError;
use sqlx::PgConnection;

// ============================================
// 凭证族「物/货」列（交易对象）——运行期探测 SQL 模板
// ============================================
//
// 交易对象列＝凭证族的「物/货」列（货 = 卖方销售产品，模型声明目标 `zc_id_prod-sales`
// 销售子树）：新模型为 `fk_payload`，模型未同步的库仍是 `fk_production`。以下模板统一含
// `{title_col}` 占位符，运行期以 `trigger_registry::stock_materialization::voucher_title_column`
// 的探测值替换后执行——读/写/守卫 rec 键同列同源。替换值恒为该函数返回的两个编译期
// 字面量之一（非用户输入、非表名），故替换结果经 `sqlx::AssertSqlSafe` 执行
// （先例：trigger-registry `TITLE_VOUCHER_INSERT_SQL` 同模式）。

/// 容量池损坏凭证防御（父表级只读校验；`{title_col}` = 交易对象列）
const CORRUPT_VOUCHER_COUNT_SQL: &str = r#"SELECT COUNT(*) FROM "isahl"."zc_id_stat-sto-voucher" v
               LEFT JOIN "isahl"."zc_id_scale" si ON si.id = v.qk_income
               LEFT JOIN "isahl"."zc_id_scale" so ON so.id = v.qk_outgo
               WHERE v.{title_col} = $1 AND v.deleted_at IS NULL
                 AND ((v.qk_income IS NOT NULL AND si.id IS NULL)
                   OR (v.qk_outgo IS NOT NULL AND so.id IS NULL))"#;

/// 委托交易凭证 OUT（下单扣可售）INSERT 模板；`{title_col}` = 交易对象列
const COM_VOUCHER_OUT_INSERT_SQL: &str = r#"INSERT INTO "isahl"."zc_id_stat-com-voucher"
               (id, code, notice, comments, {title_col}, "fk_subj-storage", "fk_obj-storage",
                qk_outgo, "ck_sto-title", dk_scene, dk_factor, dk_function, _t_, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $5, $6,
                       (SELECT id FROM "isahl"."zc_id_cate-sto-title" WHERE code = 'STO-SHELF' LIMIT 1),
                       $7, $8, $9, $10, $11)
               ON CONFLICT (code) WHERE deleted_at IS NULL DO NOTHING
               RETURNING id"#;

/// 委托交易凭证 IN（履约入池）INSERT 模板；`{title_col}` = 交易对象列
const COM_VOUCHER_IN_INSERT_SQL: &str = r#"INSERT INTO "isahl"."zc_id_stat-com-voucher"
               (id, code, notice, comments, {title_col}, "fk_subj-storage", "fk_obj-storage",
                qk_income, "ck_sto-title", dk_scene, dk_factor, dk_function, _t_, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $5, $6,
                       (SELECT id FROM "isahl"."zc_id_cate-sto-title" WHERE code = 'STO-FULFILL' LIMIT 1),
                       $7, $8, $9, $10, $11)
               ON CONFLICT (code) WHERE deleted_at IS NULL DO NOTHING"#;

/// 下单绑合同前置校验（fix-wz-contract-order-product-chain G3）：
/// ① 合同存在且未删（父表 `zc_id_contract`，PG 继承覆盖叶表）；② 有当前状态桥时
/// `stus-contract.code` 须 ∈ {active, executing}（无桥 = legacy/手工数据放行——真实写径
/// `contract-writer::insert_contract_row_tx` 恒写 draft 桥）；③ 下单客户主体须在合同方桥
/// （`rr_party.ref_right`，与 provider-contracts 候选过滤同口径）。失败即 400。
pub async fn ensure_order_contract_valid_tx(
    conn: &mut sqlx::PgConnection,
    contract_id: i64,
    customer_subject_id: i64,
) -> Result<(), WriterError> {
    let exists: bool = sqlx::query_scalar(
    r#"SELECT EXISTS (SELECT 1 FROM "isahl"."zc_id_contract" WHERE id = $1 AND deleted_at IS NULL)"#,
)
.bind(contract_id)
.fetch_one(&mut *conn)
.await
.map_err(WriterError::from_sqlx)?;
    if !exists {
        return Err(WriterError::BadRequest(format!(
            "合同 {contract_id} 不存在或已删除"
        )));
    }
    if let Some(st) =
        common::status::current_status_opt(&mut *conn, contract_id, "zc_id_stus-contract").await?
    {
        if st != "active" && st != "executing" {
            return Err(WriterError::BadRequest(format!(
                "合同 {contract_id} 当前状态 {st} 不可绑单（须 active/executing）"
            )));
        }
    }
    let in_parties: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM "isahl"."zc_id_contract_rr_party"
       WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL)"#,
    )
    .bind(contract_id)
    .bind(customer_subject_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(WriterError::from_sqlx)?;
    if !in_parties {
        return Err(WriterError::BadRequest(format!(
            "下单客户主体 {customer_subject_id} 非合同 {contract_id} 的合同方"
        )));
    }
    Ok(())
}

/// 创建正式委托（单事务内全链写入；调用方管理 commit/rollback）。
/// 行为与迁移前 dispatch `create_consignment_inner` 逐表一致（含坐标 "GC"/"FJA"/"↓_BE"、
/// voucher 守卫原语、NOT EXISTS/ON CONFLICT 幂等、comments 人类可读摘要、重量 ROUND 取整）。
pub async fn create_full(
    conn: &mut sqlx::PgConnection,
    ctx: &WriteContext,
    req: &CreateConsignmentInput,
) -> Result<CreateConsignmentOutput, WriterError> {
    // ── Step 0: 校验货描标签全部有效 ──
    if !req.goods_tag_ids.is_empty() {
        let valid_count: (i64,) = sqlx::query_as(
            r#"SELECT COUNT(*) FROM "isahl"."zc_id_cons-goods-tags"
               WHERE id = ANY($1) AND deleted_at IS NULL"#,
        )
        .bind(&req.goods_tag_ids)
        .fetch_one(&mut *conn)
        .await?;
        if valid_count.0 as usize != req.goods_tag_ids.len() {
            return Err(WriterError::BadRequest("部分货描标签不存在或已删除".into()));
        }
        let distinct: (i64,) =
            sqlx::query_as(r#"SELECT COUNT(DISTINCT id) FROM unnest($1::bigint[]) AS id"#)
                .bind(&req.goods_tag_ids)
                .fetch_one(&mut *conn)
                .await?;
        if distinct.0 as usize != req.goods_tag_ids.len() {
            return Err(WriterError::BadRequest("货描标签含有重复ID".into()));
        }
    }

    // ── Step 0b: 合同前置校验（fix-wz-contract-order-product-chain G3）──
    // 前端候选过滤（provider-contracts）不是服务端守卫：存在性/状态/合同方在此断言。
    if let Some(contract_id) = req.contract_id {
        ensure_order_contract_valid_tx(conn, contract_id, ctx.customer_subject_id).await?;
    }

    // ── Step 0c: 承运商引用守卫 ──
    // 创建写路把 `carrier_id` 直落产品 `"fk_subj-provider"`，而该列**无外键约束**
    // （库内 `pg_constraint` 实测 0 条 FK）⇒ 非法引用静默入库后，读侧解析不到主体会按
    // COALESCE 链回落到另一主体（错误不可见）。此处与编辑写路 `apply_carrier_tx` 同语义拒绝。
    if let Some(carrier_id) = req.carrier_id {
        let carrier_ok: bool = sqlx::query_scalar(
            r#"SELECT EXISTS (SELECT 1 FROM "isahl"."zc_id_subjects" WHERE id = $1 AND deleted_at IS NULL)"#,
        )
        .bind(carrier_id)
        .fetch_one(&mut *conn)
        .await?;
        if !carrier_ok {
            return Err(WriterError::BadRequest(format!(
                "承运商主体 {carrier_id} 不存在或已删除"
            )));
        }
    }

    // ── Step 1: 查 TrafficLine 校验线路存在 ──
    let _line_name: String = sqlx::query_scalar(
        r#"SELECT notice
           FROM "isahl"."zc_id_stor-traffic_line"
           WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(req.traffic_line_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|_| WriterError::NotFound(format!("TrafficLine {} not found", req.traffic_line_id)))?;

    // ── Step 2: 创建标量 (weight + volume + amount) ──
    // 货量按入参原值落库：`zc_id_scal-weight.mark` = numeric(30,10)，小数货量是常态
    // （取整会篡改委托货量，并与同一标量 notice 的原文自相矛盾）；金额按分保留两位。
    let weight_scale_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_scal-weight" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_uid(432), $1, $2, $3::numeric, $4)
           RETURNING id"#,
    )
    .bind(format!("WT-{}", chrono::Utc::now().timestamp()))
    .bind(format!("{}吨", req.weight_ton))
    .bind(req.weight_ton)
    .bind(ctx.actor_user_id)
    .fetch_one(&mut *conn)
    .await?;

    // 体积 m³ → 明细 `qk_v_qty` → `zc_id_scal-volume`（码前缀 `VOL-`，同编辑写路
    // `DETAIL_SCALAR_VOLUME` 口径）。入参 0 = 未填体积 → 不建标量（读侧 NULL → 前端占位）。
    // 原实现只把体积拼进 `comments` 摘要而不落结构载体 ⇒ 读侧（`logi-consignment`
    // consignment-detail 的 `SUM(qk_v_qty)`）恒 NULL（批注 0f6e74d2：委托填了体积却不显示）。
    let volume_scale_id: Option<i64> = if req.volume_cbm > 0.0 {
        Some(
            sqlx::query_scalar(
                r#"INSERT INTO "isahl"."zc_id_scal-volume" (id, code, notice, mark, created_by_id)
                   VALUES (isahl.gen_next_uid(431), $1, $2, $3::numeric, $4)
                   RETURNING id"#,
            )
            .bind(format!("VOL-{}", chrono::Utc::now().timestamp()))
            .bind(format!("{} m³", req.volume_cbm))
            .bind(req.volume_cbm)
            .bind(ctx.actor_user_id)
            .fetch_one(&mut *conn)
            .await?,
        )
    } else {
        None
    };

    let amount_scale_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_scal-amount" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_uid(416), $1, $2, ROUND($3::numeric, 2), $4)
           RETURNING id"#,
    )
    .bind(format!("AMT-{}", chrono::Utc::now().timestamp()))
    .bind(format!("¥{:.2}", req.price_amount))
    .bind(req.price_amount)
    .bind(ctx.actor_user_id)
    .fetch_one(&mut *conn)
    .await?;

    // ── Step 3: 创建委托单 (orde-land) ──
    // 双层双方（D1）：fk_subject=委托方 A、fk_object=运营组织 B（ctx 注入）。
    // 单号：`code_override` 优先（拆分小委托 `{大委托code}-S{n}`，D6），否则既有 `CNS-{ts}` 硬生成。
    let consign_code = match req.code_override.as_deref() {
        Some(c) if !c.trim().is_empty() => c.trim().to_string(),
        _ => format!(
            "CNS-{}",
            chrono::Utc::now()
                .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).expect("+08"))
                .format("%Y%m%d-%H%M%S")
        ),
    };
    // 起讫名称解析（notice；id 无效回退原串——fail-visible，与 rr_stop 桥语义一致）
    let origin_name: String = sqlx::query_scalar(
        r#"SELECT notice FROM "isahl"."zc_id_place" WHERE id = $1::bigint AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(req.origin.parse::<i64>().ok())
    .fetch_optional(&mut *conn)
    .await?
    .flatten()
    .unwrap_or_else(|| req.origin.clone());
    let dest_name: String = sqlx::query_scalar(
        r#"SELECT notice FROM "isahl"."zc_id_place" WHERE id = $1::bigint AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(req.dest.parse::<i64>().ok())
    .fetch_optional(&mut *conn)
    .await?
    .flatten()
    .unwrap_or_else(|| req.dest.clone());
    // 矩阵 #2（用户裁决 2026-09-11）：订单驱动 → 单据与产品形态均为 实现·实例；
    // 由单一派生源 derive_form_type 取值后参数绑定（禁字面量对）
    let (trade_form, trade_tier) = WriterDk::ConsignmentTrade.form_type();
    // notice = 货描纯文本（name 字段语义；2026-09-14 用户更正：运输内容列只应显示货描）。
    // 起讫/线路已有结构化家园（rr_stop 桥 / fk_line），不再重复拼进名称——旧格式
    // 「起点→讫点 货描」让门户运输内容列出现冗余前缀（讫点为空时更退化为「京唐港→ 煤」）。
    // 富摘要（起讫+货描+体积）仍落 comments，读侧 cargo 解析（extract_cargo_name）不受影响。
    let consign_notice = req.cargo_desc.clone();
    let consign_comments = if req.volume_cbm > 0.0 {
        format!(
            "{}→{} {}（体积 {} m³）",
            origin_name, dest_name, req.cargo_desc, req.volume_cbm
        )
    } else {
        format!("{}→{} {}", origin_name, dest_name, req.cargo_desc)
    };
    let consignment_id: i64 = {
        // 叶表坐标（§6.12）：订单行 dk 经静态绑定解析（禁硬编码 ZUID）
        let (order_dk_scene, order_dk_factor, order_dk_function) =
            resolve_coords(conn, WriterDk::OrderDocument).await?;
        let id = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_orde-land" (id, code, notice, fk_subject, fk_object, fk_contract, comments, created_by_id, "_f_", "_t_", dk_scene, dk_factor, dk_function, ak_permit_user, ak_access_user)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, ARRAY[$13]::bigint[], ARRAY[$13]::bigint[])
               RETURNING id"#,
        )
        .bind(&consign_code)
        .bind(&consign_notice)
        .bind(ctx.customer_subject_id)
        .bind(ctx.operator_org_id)
        .bind(req.contract_id)
        .bind(&consign_comments)
        .bind(ctx.actor_user_id)
        .bind(trade_form)
        .bind(trade_tier)
        .bind(order_dk_scene)
        .bind(order_dk_factor)
        .bind(order_dk_function)
        // 行级权属（D5）：当前操作者 uid 落 ak_permit_user/ak_access_user，读侧口径
        // `ak_permit_user @> ARRAY[uid]`（scenes.rs / AP list_outgo_waybills）。
        .bind(ctx.actor_user_id)
        .fetch_one(&mut *conn)
        .await?;
        // 合同 junction 落库（m2m 规范载体；NOT EXISTS 幂等）
        if let Some(ct_id) = req.contract_id {
            sqlx::query(
                r#"INSERT INTO "isahl"."zc_id_order_rr_contract"
                   (id, code, notice, ref_left, ref_right, created_by_id)
                   SELECT isahl.gen_next_uid(470), $1, $2, $3, $4, $5
                   WHERE NOT EXISTS (
                     SELECT 1 FROM "isahl"."zc_id_order_rr_contract"
                     WHERE ref_left = $3 AND ref_right = $4 AND deleted_at IS NULL)"#,
            )
            .bind(format!("ORC-{}", consign_code))
            .bind(format!("{} 销售合同挂接", consign_code))
            .bind(id)
            .bind(ct_id)
            .bind(ctx.actor_user_id)
            .execute(&mut *conn)
            .await?;
        }
        id
    };

    // ── 初始状态桥（2026-09-14 用户更正：初始状态应为「待受理」语义）──
    // 创建即落 ST-NEW（新委托 = 待受理族；status_mapper 中 ST-NEW/ST-ORDERED/ST-PREPARING
    // 与缺省均 → pending_review）。缺桥会让门户「我的委托」状态列 LEFT JOIN 得 NULL 显示「—」。
    // 范式同 transport-dispatch dispatch_core 受理链（INSERT..SELECT + NOT EXISTS，
    // UNIQUE(ref_left) 覆盖软删行）；字典缺行 fail-visible（同 procure.rs draft 状态处置）。
    let st_new_id: Option<i64> = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_lifecycle_r_primary-status" (ref_left, ref_right, id, code)
           SELECT $1, s.id, isahl.gen_next_uid(686), 'ST-NEW'
           FROM (SELECT id FROM "isahl"."zc_id_stus-trade" WHERE code = 'ST-NEW' AND deleted_at IS NULL ORDER BY id LIMIT 1) s
           WHERE NOT EXISTS (
             SELECT 1 FROM "isahl"."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1)
           RETURNING ref_right"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut *conn)
    .await?;
    if st_new_id.is_none() {
        return Err(WriterError::Internal(
            "委托状态字典缺 ST-NEW 行——zc_id_stus-trade 归模型级种子（Framework/seed/seed-standard-dicts.sql），请确认模型级种子已重放"
                .into(),
        ));
    }

    // ── 下单不派生合同（用户裁决 2026-09-14：「委托是委托，合同是合同，创建订单不用创合同」）──
    // 原「下单合同对」（诉求合同 `CT-{code}-REQ` + 销售合同 `CT-{code}` 及其一式两份镜像 +
    // `zc_id_order_rr_contract` 四行桥）整段移除；委托与合同的关联只走**用户下单时已选合同**
    // （`fk_contract` + `order_rr_contract` 幂等桥，见上方 `req.contract_id` 分支）。
    // 镜像委托行同样只关联主委托所关联合同的**镜像合同**（见下方镜像写入，经 MIR 桥解析）。

    // ── 一式两份镜像订单（矩阵 #2，用户裁决 2026-09-11）──
    // 同表同类别、甲/乙主体互换（fk_subject↔fk_object）、code=`{code}-R`；
    // 互链走**叶子**桥 `zc_id_lifecycle_rr_form`（非叶 `zc_id_lifecycle_rr_non_self` 82 子表不可直插）。
    // 镜像行 + 桥统一经 `mirror::insert_order_mirror_tx` 单源（与派车运单链同实现）；
    // 镜像行内已补 ak_permit_user/ak_access_user（同源真实操作者；读侧行级权属可见）；主行已有状态时镜像即时同位。
    // 镜像合同（用户裁决 2026-09-14「创合同时已经有镜像的，镜像委托关联上镜像合同」）：
    // 主委托 `fk_contract` 的镜像合同经 **MIR 桥**解析（`contract-writer::mirror::resolve_mirror_ids_tx`
    // 单源，禁第二份同语义 SQL）；主委托未关联合同 / 该合同无镜像行 → NULL（不造值）。
    let mirror_contract_id: Option<i64> = match req.contract_id {
        Some(main_contract_id) => {
            contract_writer::mirror::resolve_mirror_ids_tx(conn, main_contract_id)
                .await?
                .into_iter()
                .next()
        }
        None => None,
    };

    let mirror_consignment_id: i64 = insert_order_mirror_tx(
        conn,
        consignment_id,
        &OrderMirrorInput {
            code: &format!("{}-R", consign_code),
            notice: &format!("{}（镜像）", consign_notice),
            comments: &consign_comments,
            subject: Some(ctx.operator_org_id),
            object: Some(ctx.customer_subject_id),
            qk_date: None,
            fk_contract: mirror_contract_id,
            fn_code: "↓_BE",
            kind_label: "订单",
            user_id: ctx.actor_user_id,
        },
    )
    .await?;

    // ── Step 4: 创建产品 (freight_road-sales)，线路用 fk_line、起讫地经 rr_stop 桥接 ──
    // 属权（D4）：fk_subj-demand=委托方 A、fk_subj-provider=卖方 B；形态 = 实现·实例（↓_BE 派生）
    let (dk_scene_id, dk_factor_id, dk_function_id) =
        resolve_coords(conn, WriterDk::ConsignmentTrade).await?;
    // 矩阵 #2（用户裁决 2026-09-11）：订单驱动产品 = 实现·实例；`_f_`/`_t_` 双绑（上式派生，
    // 单一派生源 derive_form_type；DB 无形态触发器）
    // 预计提货/预计到达落独立时间段表 zc_id_segm-date（date_st=预计出发/date_ed=预计到达）
    let parse_local = |s: &str| {
        let t = s.trim();
        chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M")
            .or_else(|_| {
                chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
                    .map(|d| d.and_time(chrono::NaiveTime::MIN))
            })
            .ok()
            .map(|n| n.and_utc() - chrono::Duration::hours(8))
    };
    let pickup_ts = req.pickup_time.as_deref().and_then(parse_local);
    let eta_ts = req.eta.as_deref().and_then(parse_local);
    let eta_seg_id: Option<i64> = match req.period_id.filter(|pid| *pid > 0) {
        // 继承既有时间段行（拆单小委托自大委托产品 `qk_period` 带值）：同一批货共用一个
        // `zc_id_segm-date` 行，不复制日期行；非正 id 视为未给（宁缺勿脏引用）。
        Some(inherited) => Some(inherited),
        None => match (pickup_ts, eta_ts) {
            (None, None) => None,
            (p, e) => Some(
                sqlx::query_scalar(
                    r#"INSERT INTO "isahl"."zc_id_segm-date"
                   (id, code, notice, date_st, date_ed, created_by_id)
                   VALUES (isahl.gen_next_uid(437), $1, $2, $3, $4, $5)
                   RETURNING id"#,
                )
                .bind(format!("PERIOD-{}", consign_code))
                .bind("预计提货-预计送达")
                .bind(p)
                .bind(e)
                .bind(ctx.actor_user_id)
                .fetch_one(&mut *conn)
                .await?,
            ),
        },
    };
    // 成交单价标量（元/吨；重量为零或无金额时 NULL）
    let prd_price_id: Option<i64> = if req.price_amount > 0.0 && req.weight_ton > 0.0 {
        let unit_price = rust_decimal::Decimal::try_from(req.price_amount / req.weight_ton)
            .unwrap_or_default()
            .round_dp(2);
        Some(
            sqlx::query_scalar(
                r#"INSERT INTO "isahl"."zc_id_scal-price" (id, code, notice, mark, created_by_id)
                   VALUES (isahl.gen_next_uid(428), $1, $2, $3, $4)
                   RETURNING id"#,
            )
            .bind(format!("PRC-PRD-{}", consign_code))
            .bind(format!("{} 成交单价", consign_code))
            .bind(unit_price)
            .bind(ctx.actor_user_id)
            .fetch_one(&mut *conn)
            .await?,
        )
    } else {
        None
    };

    // ── Step 4: 产品对（一式两份，矩阵 #2）──
    // 主：销售实例（`fk_previous` = 委托）——下游派车解析线路（dispatch_core Step 0a）、起讫 stop 归属、
    //     委托矩阵聚合、OA 侧对称写入均依赖该桥；缺失则派车报「委托无线路信息」。
    // 镜像：采购族（方向相反、`PRD-{code}-R`、`fk_previous` = 镜像订单）；同一买卖主体对/线路/标量。
    // 两者统一经 `contract-writer::insert_product_pair_tx` 单份实现（形态由职能码派生，禁字面量对）。
    let (product_id, mirror_product_id) = contract_writer::insert_product_pair_tx(
        conn,
        &contract_writer::ProductPairInput {
            is_sales: true,
            code: &format!("PRD-{}", consign_code),
            notice: &format!("{}→{} 干线", origin_name, dest_name),
            comments: &format!("委托销售实例 线路 {}", req.traffic_line_id),
            mirror_notice: Some(&format!("{}→{} 干线（镜像）", origin_name, dest_name)),
            mirror_comments: Some(&format!("镜像采购实例 线路 {}", req.traffic_line_id)),
            demand_subject: ctx.customer_subject_id,
            // 物流服务商（承运商）——显式选择优先，未选回退运营组织
            provider_subject: req.carrier_id.unwrap_or(ctx.operator_org_id),
            line_id: Some(req.traffic_line_id),
            vehicle_form_id: req.vehicle_type_id,
            price_id: prd_price_id,
            weight_id: None,
            period_id: eta_seg_id,
            previous_main: Some(consignment_id),
            // 镜像产品挂**镜像订单**（矩阵 #2：镜像行与主行同等待遇）——小委托亦已建镜像单，
            // 与主委托路径同口径（镜像采购族 `fk_previous` = `{code}-R` 镜像单 id）。
            previous_mirror: Some(mirror_consignment_id),
            fn_code: "↓_BE",
            scene_code: "GC",
            factor_code: "FJA",
            user_id: ctx.actor_user_id,
        },
    )
    .await?;

    // ── Step 4c: 物化委托方 A 的 request 实例（{code}-REQ）+ prod-request_rr_prod-sales 桥 ──
    // 小委托裁剪（D6）：MUST NOT 建自动 `{code}-REQ`——它不是询价（询价由 `procure::create_inquiry`
    // 以 `INQ-*` 承载），且会以 `fk_previous`=本单武装 `AWARD_NOT_CONFIRMED` 发单门禁
    //（`consignment_inquiry_award_confirmed` 按 `request.fk_previous` 统计未确认询盘）。
    // 故 `request_id = None`（明细 `fk_demand` 落 NULL）。
    let request_id: Option<i64> = if req.sub_consignment {
        None
    } else {
        let req_price_id: Option<i64> = match req.expected_price {
            Some(ep) if ep > 0.0 => Some(
                sqlx::query_scalar(
                    r#"INSERT INTO "isahl"."zc_id_scal-price" (id, code, notice, mark, created_by_id)
                       VALUES (isahl.gen_next_uid(428), $1, $2, $3::numeric, $4)
                       RETURNING id"#,
                )
                .bind(format!("PRC-REQ-{}", consign_code))
                .bind(format!("{} 询盘预期价", consign_code))
                .bind(ep)
                .bind(ctx.actor_user_id)
                .fetch_one(&mut *conn)
                .await?,
            ),
            _ => None,
        };
        let request_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_prod-freight_road-request"
               (id, code, notice, comments, "fk_subj-demand", "fk_subj-provider",
                fk_line, qk_w_lading,
                dk_scene, dk_factor, dk_function, qk_price, "_f_", "_t_", created_by_id,
                ak_permit_user, fk_previous)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3::text,
                       $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, ARRAY[$14]::bigint[], $15)
               RETURNING id"#,
        )
        .bind(format!("{}-REQ", consign_code))
        .bind(format!("{}→{} 客户诉求", origin_name, dest_name))
        .bind(format!("客户诉求 线路 {}", req.traffic_line_id))
        .bind(ctx.customer_subject_id)
        .bind(ctx.operator_org_id)
        .bind(req.traffic_line_id)
        .bind(weight_scale_id)
        .bind(dk_scene_id)
        .bind(dk_factor_id)
        .bind(dk_function_id)
        .bind(req_price_id)
        .bind(trade_form)
        .bind(trade_tier)
        .bind(ctx.actor_user_id)
        // fk_previous = 本委托 id：委托派生的询价（`{委托code}-REQ`）必须直连委托，
        // 否则「询价管理」按 consignmentId（= request.fk_previous）过滤时**看不到该委托的询单**
        // （用户批注 11ef1b6c「新建的委托询单数据全没」，2026-09-12 实测：该列此前为 NULL，
        //  只能靠 code 前缀约定关联；对照 waybill-writer 同型插入是带 fk_previous 的）。
        .bind(consignment_id)
        .fetch_one(&mut *conn)
        .await?;
        // 起讫 stop 桥（对齐平台询盘 INQ-STOP over-seq 1/2——平台询价列表经 rr_stop 读起讫地）
        for (_tag, place_raw, seq) in [("O", &req.origin, 1i16), ("D", &req.dest, 2i16)] {
            let place_id = place_raw.parse::<i64>().ok();
            let stop_code = format!("INQ-STOP-{request_id}-{seq}");
            // 起讫 stop 为询价展示增强——失败仅丢起讫信息，不阻断委托创建。
            // MUST 经 SAVEPOINT 隔离：裸 INSERT 一失败即终止整个事务（PG 事务 aborted 后
            // 后续语句全部报「当前事务被终止」→ 委托创建整体 500，实测 dest 侧 stop 插入
            // 失败即整链失败，错误信息被 abort 掩盖）。先例：approval/handlers/publish.rs
            // `update_flow_lifecycle_status_tx` 审计写入的 SAVEPOINT 隔离。
            sqlx::query(r#"SAVEPOINT inquiry_stop"#)
                .execute(&mut *conn)
                .await?;
            let stop_res = sqlx::query(
                r#"INSERT INTO "isahl"."zc_id_prod-transport_rr_stop"
                   (id, code, notice, ref_left, ref_right, created_by_id, "over-seq")
                   VALUES (isahl.gen_next_uid(288), $1, $2, $3, $4, $5, $6)"#,
            )
            .bind(&stop_code)
            .bind(format!("询盘起讫点 {seq}"))
            .bind(request_id)
            .bind(place_id)
            .bind(ctx.actor_user_id)
            .bind(seq)
            .execute(&mut *conn)
            .await;
            match stop_res {
                Ok(_) => {
                    sqlx::query(r#"RELEASE SAVEPOINT inquiry_stop"#)
                        .execute(&mut *conn)
                        .await?;
                }
                Err(e) => {
                    sqlx::query(r#"ROLLBACK TO SAVEPOINT inquiry_stop"#)
                        .execute(&mut *conn)
                        .await?;
                    log::warn!("询盘起讫 stop 写入失败（{stop_code}，降级继续）: {e}");
                }
            }
        }
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_prod-request_rr_prod-sales"
               (code, notice, ref_left, ref_right, created_by_id)
               VALUES ($1, $2, $3, $4, $5)"#,
        )
        .bind(format!("REL-RS-{}-{}", request_id, product_id))
        .bind("诉求→销售应答")
        .bind(request_id)
        .bind(product_id)
        .bind(ctx.actor_user_id)
        .execute(&mut *conn)
        .await?;
        Some(request_id)
    };

    // ── Step 4d: 线路销售范例（CAP-SALE，B 可售池）——委托明细 fk_goods 引用；无则 NULL ──
    // 载体迁移（用户裁决 2026-09-21，报缺产物 R6）：容量池关系行原写读在声明语义
    // 「关联-文件↔URL」的桥表上（挪用）；合法载体 = `zc_id_prod-payload_rr_stor-container`
    // （关联-载荷↔容器，⊂ `zc_id_production_rr_storage`；mv_inventory 读父表故照常可见）。
    let pool_product: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT r.ref_left FROM "isahl"."zc_id_prod-payload_rr_stor-container" r
           LEFT JOIN "isahl"."zc_id_prod-freight_road-sales" pp
             ON pp.id = r.ref_left AND pp.deleted_at IS NULL
           WHERE r.ref_right = $1 AND r.qk_p_capacity IS NOT NULL AND r.deleted_at IS NULL
           ORDER BY CASE WHEN pp.code LIKE 'CAP-SALE-%' THEN 0
                         WHEN pp.code LIKE 'CAP-TL-%' THEN 1
                         WHEN pp.code LIKE 'CAP-LINE-%' THEN 2 ELSE 3 END NULLS LAST
           LIMIT 1"#,
    )
    .bind(req.traffic_line_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();

    // ── Step 4a: 起讫地桥接（rr_stop：ref_left=产品, ref_right=场所 id, ck_category=出发/到达）──
    // 产品对（主 sales + 镜像 purchase）同起讫；写件单源 contract_writer::insert_product_stops_tx
    // （合同驱动/询价成交各链共用，禁第二份同语义 SQL）。起讫 id 非法即跳过（不阻断委托创建）。
    for (prd, prd_code) in [
        (product_id, format!("PRD-{}", consign_code)),
        (mirror_product_id, format!("PRD-{}-R", consign_code)),
    ] {
        contract_writer::insert_product_stops_tx(
            conn,
            prd,
            &prd_code,
            req.origin.parse::<i64>().ok(),
            req.dest.parse::<i64>().ok(),
            ctx.actor_user_id,
        )
        .await?;
    }

    // ── Step 4b: 创建装载包装产品 (prod-loading) ──
    // `fk_previous` = 本委托 id：全部读侧（`transport-operations` 运单详情的货描标签/轨迹链、
    // `logi-consignment` 委托读侧的 `ldg.fk_previous = o.id`）都以此为「装载包装 ↔ 订单」的
    // 唯一结构链（派车写路 `waybill-writer/dispatch.rs` 同式落 `wb_id`）。原实现漏绑 ⇒ 链恒断：
    // 批注 66405902「运输性质没显示」——货描标签 `GT-COAL` 已挂在装载产品上，但读侧按
    // `fk_previous = o.id` 反查不到装载行 ⇒ `goods_tag_group` NULL ⇒ 运输性质恒 '—'。
    // 坐标三元组（§6.12/§4.3.3）：装载包装 dk 经静态绑定解析（禁硬编码 ZUID）
    let (ldg_dk_scene, ldg_dk_factor, ldg_dk_function) =
        resolve_coords(conn, WriterDk::LoadingPackage).await?;
    let loading_product_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_prod-loading-request"
           (id, code, notice, fk_previous, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7)
           RETURNING id"#,
    )
    .bind(format!("LDG-{}", consign_code))
    .bind(&req.cargo_desc)
    .bind(consignment_id)
    .bind(ctx.actor_user_id)
    .bind(ldg_dk_scene)
    .bind(ldg_dk_factor)
    .bind(ldg_dk_function)
    .fetch_one(&mut *conn)
    .await?;

    // ── Step 5: 创建委托运力占用记录（`zc_id_prod-payload_rr_stor-container`，⊂ production_rr_storage），
    // qk_qty = 委托重量 ──
    // 载体迁移（用户裁决 2026-09-21，报缺产物 R6）：委托运力占用原落声明语义「关联-文件↔URL」
    // 的桥表（挪用）；合法载体 = 关联-载荷↔容器；id 走载体自身 uid 段（模型默认 517，原借用为 335）。
    // 小委托裁剪（D6）：同一批货量大委托下单时已扣运力占用，MUST NOT 重复扣减 → inventory_id = 0。
    let inventory_id: i64 = if req.sub_consignment {
        0
    } else {
        sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_prod-payload_rr_stor-container"
               (id, code, notice, comments, ref_left, ref_right, qk_qty, created_by_id)
               VALUES (isahl.gen_next_uid(517), $1, $2, $3, $4, $5, $6, $7)
               RETURNING id"#,
        )
        .bind(format!("ORD-INV-{}", consign_code))
        .bind(format!(
            "{}→{} 运力占用 {}吨",
            req.origin, req.dest, req.weight_ton
        ))
        .bind("委托运力占用")
        .bind(product_id)
        .bind(req.traffic_line_id)
        .bind(weight_scale_id)
        .bind(ctx.actor_user_id)
        .fetch_one(&mut *conn)
        .await?
    };

    // ── Step 5b: 委托交易凭证（com-voucher）——销售货架范例↓ → 履约范例↑ ──
    // 线路未配置容量池时跳过（无库存可扣，凭证流为附加事实）；
    // 小委托裁剪（D6）：可售凭证已由大委托下单扣减，MUST NOT 重复扣。
    if let (Some(pool_pid), false) = (pool_product, req.sub_consignment) {
        // 交易对象列（凭证族「物/货」列）运行期探测——本块三处（损坏防御读、com-voucher
        // 两条写入、守卫 rec 键）同取该列名：新模型 `fk_payload`，旧模型回落 `fk_production`。
        let title_col = trigger_registry::stock_materialization::voucher_title_column(&mut *conn)
            .await
            .map_err(|e| WriterError::Internal(format!("凭证交易对象列探测失败: {e}")))?;
        // 损坏凭证防御（qk_income/qk_outgo 标量引用不可解析 → 硬拒绝）
        let corrupt: (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(
            CORRUPT_VOUCHER_COUNT_SQL.replace("{title_col}", title_col),
        ))
        .bind(pool_pid)
        .fetch_one(&mut *conn)
        .await?;
        if corrupt.0 > 0 {
            return Err(WriterError::BadRequest(format!(
                "容量池产品 {} 存在损坏凭证（qk_income/qk_outgo 引用无效），拒绝创建委托以防超卖",
                pool_pid
            )));
        }
        let (dk_scene_id, dk_factor_id, dk_function_id) =
            resolve_coords(conn, WriterDk::TspTemplateLeg).await?;
        let (_, leg_tier) = WriterDk::TspTemplateLeg.form_type();
        let ts = format!("COM-{}", consign_code);
        if let Err(e) =
            trigger_registry::stock_materialization::ensure_voucher_idempotency_tx(conn).await
        {
            log::warn!("voucher idempotency 惰性自愈失败（降级继续）: {e}");
        }
        // 源池行（qk_outgo，STO-SHELF 货架↓）——出库凭证（下单扣可售）
        let comments_out = format!("委托交易 {} OUT（sales）", consign_code);
        let voucher_out_id: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(
            COM_VOUCHER_OUT_INSERT_SQL.replace("{title_col}", title_col),
        ))
        .bind(format!("{}-OUT", ts))
        .bind(format!("委托交易 {} OUT", consign_code))
        .bind(&comments_out)
        .bind(pool_pid)
        .bind(req.traffic_line_id)
        .bind(weight_scale_id)
        .bind(dk_scene_id)
        .bind(dk_factor_id)
        .bind(dk_function_id)
        .bind(leg_tier)
        .bind(ctx.actor_user_id)
        .fetch_optional(&mut *conn)
        .await?;
        // 目标池行（qk_income，STO-FULFILL 履约↑）
        let comments_in = format!("委托交易 {} IN（sales）", consign_code);
        sqlx::query(sqlx::AssertSqlSafe(
            COM_VOUCHER_IN_INSERT_SQL.replace("{title_col}", title_col),
        ))
        .bind(format!("{}-IN", ts))
        .bind(format!("委托交易 {} IN", consign_code))
        .bind(&comments_in)
        .bind(pool_pid)
        .bind(req.traffic_line_id)
        .bind(weight_scale_id)
        .bind(dk_scene_id)
        .bind(dk_factor_id)
        .bind(dk_function_id)
        .bind(leg_tier)
        .bind(ctx.actor_user_id)
        .execute(&mut *conn)
        .await?;
        // 守卫原语物化（锁内链尾判定）——只应用 OUT 行。
        // 不传 `__min`/`__max`：按用户口径「报量即容量」——承运商/线路自报的容量不作拒绝依据，
        // 下单不因线路容量池可售余量不足而失败（超卖下界已移除）。
        if let Some(voucher_out_id) = voucher_out_id {
            let mut rec = std::collections::HashMap::new();
            rec.insert("id".to_string(), serde_json::json!(voucher_out_id));
            // 物理列名即 HashMap 键（与 INSERT/链尾判定同列；守卫侧 get_title_id 兼容两键）
            rec.insert(title_col.to_string(), serde_json::json!(pool_pid));
            rec.insert("qk_outgo".to_string(), serde_json::json!(weight_scale_id));
            rec.insert(
                "fk_subj-storage".to_string(),
                serde_json::json!(req.traffic_line_id),
            );
            rec.insert(
                "__code".to_string(),
                serde_json::json!(format!("{}-OUT", ts)),
            );
            if let Err(e) =
                trigger_registry::stock_materialization::apply_guarded_voucher_tx(conn, &rec).await
            {
                return Err(WriterError::Internal(format!("出库凭证物化失败: {e}")));
            }
        }
    }

    // ── Step 5: 创建运输明细行 (deta-trade_order) ──
    // 五 FK 接线（契约 #3）：fk_demand=A 的 REQ、fk_deal=B 的 sales 实例（PRD）、
    // fk_biller=B、fk_counterparty=A；fk_goods=B 的 sales **范例**——优先 = 合同产品
    // （fix-wz-contract-order-product-chain G1：`PRD-{合同code}` 经 contract-writer 单源解析，
    // 契约 #3 范例语义的精确化），无合同/合同无产品桥 → 线路容量池（现状兜底）→ NULL。
    // 坐标三元组（§6.12）：明细行 dk 经静态绑定解析一次，本步骤两条 INSERT 复用（禁硬编码 ZUID）
    let goods_product: Option<i64> = match req.contract_id {
        Some(cid) => contract_writer::resolve_contract_sales_product_tx(&mut *conn, cid).await?,
        None => None,
    }
    .or(pool_product);
    let (dtl_dk_scene, dtl_dk_factor, dtl_dk_function) =
        resolve_coords(conn, WriterDk::TradeOrderDetail).await?;
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_deta-trade_order"
           (id, code, notice, comments, qk_qty, qk_amount, qk_w_qty, qk_v_qty, qk_price, fk_list, fk_goods, fk_deal, fk_demand,
            fk_biller, fk_counterparty, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)"#,
    )
    .bind(format!("DTL-{}", consign_code))
    .bind(format!("{} {}吨", req.cargo_desc, req.weight_ton))
    .bind(req.cargo_desc.clone())
    .bind(amount_scale_id)
    .bind(weight_scale_id)
    .bind(volume_scale_id)
    .bind(prd_price_id)
    .bind(consignment_id)
    .bind(goods_product)
    .bind(product_id)
    .bind(request_id)
    .bind(ctx.operator_org_id)
    .bind(ctx.customer_subject_id)
    .bind(ctx.actor_user_id)
    .bind(dtl_dk_scene)
    .bind(dtl_dk_factor)
    .bind(dtl_dk_function)
    .execute(&mut *conn)
    .await?;

    // ── Step 5b: 创建装载包装明细行（fk_deal NULL 仅指向销售实例；fk_goods=装载产品）──
    // 体积与货量/金额同式挂同标量：读侧聚合「有主明细则排除 DTL-LDG，仅 DTL-LDG 时保留」
    // （`waybill_detail.rs` / `consignment_detail` / `dispatch_core.rs` 同守卫）⇒ 双挂不重复计数，
    // 主明细缺失时装载行仍能承载体积真值。
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_deta-trade_order"
           (id, code, notice, fk_list, fk_goods, qk_w_qty, qk_amount, qk_v_qty, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
    )
    .bind(format!("DTL-LDG-{}", consign_code))
    .bind(format!("{} 装载包装", req.cargo_desc))
    .bind(consignment_id)
    .bind(loading_product_id)
    .bind(weight_scale_id)
    .bind(amount_scale_id)
    .bind(volume_scale_id)
    .bind(ctx.actor_user_id)
    .bind(dtl_dk_scene)
    .bind(dtl_dk_factor)
    .bind(dtl_dk_function)
    .execute(&mut *conn)
    .await?;

    // ── Step 5c: 关联货描标签到装载包装产品
    for tag_id in &req.goods_tag_ids {
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_prod-loading_r_goods-tag"
               (id, code, ref_left, ref_right, created_by_id)
               VALUES (isahl.gen_next_uid(892), $1, $2, $3, $4)"#,
        )
        .bind(format!("GT-{}-{}", consign_code, tag_id))
        .bind(loading_product_id)
        .bind(tag_id)
        .bind(ctx.actor_user_id)
        .execute(&mut *conn)
        .await?;
    }

    // ── Step 5d: 父子桥（zc_id_order_rr_demand）——`ref_left`=本单 / `ref_right`=父委托 ──
    // 拆分小委托（D6）挂大委托（总单守卫只禁 `ref_right` 以 `WB-` 开头 ⇒ CNS 小委托天然放行）；
    // 幂等：唯一键 (ref_left, ref_right, COALESCE(qk_period,-1)) 且本写 qk_period=NULL——
    // 已存在同 (ref_left, ref_right) 即跳过（NOT EXISTS + ON CONFLICT 双保险）。
    if let Some(parent_id) = req.parent_consignment_id {
        let parent_code: Option<String> = sqlx::query_scalar(
            r#"SELECT code FROM "isahl"."zc_id_orde-land"
               WHERE id = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(parent_id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten();
        let parent_code = parent_code.ok_or_else(|| {
            WriterError::NotFound(format!("父委托 {parent_id} 不存在或已删除，无法挂父子桥"))
        })?;
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_order_rr_demand"
               (id, code, notice, ref_left, ref_right, created_by_id, updated_by_id)
               SELECT isahl.gen_next_uid(467), $1, $2, $3, $4, $5, $5
               WHERE NOT EXISTS (
                 SELECT 1 FROM "isahl"."zc_id_order_rr_demand"
                 WHERE ref_left = $3 AND ref_right = $4 AND deleted_at IS NULL)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(format!("SPL-{}-{}", consign_code, parent_code))
        .bind(format!("拆分单 {} 挂总单 {}", consign_code, parent_code))
        .bind(consignment_id)
        .bind(parent_id)
        .bind(ctx.actor_user_id)
        .execute(&mut *conn)
        .await?;
    }

    Ok(CreateConsignmentOutput {
        consignment_id,
        product_id,
        loading_product_id,
        inventory_id,
        code: consign_code,
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// 编辑结构写路（change: migrate-consignment-fields-to-structures T10）
//
// 读侧已切结构（`logi-consignment/repositories/tasks.rs`：货物 ← 明细行、起讫 ← 停靠桥、
// 时段 ← `segm-date`、体积/货量/金额 ← 明细标量），`comments` 摘要**不再被解析**。
// 委托编辑（`PUT /service/isahl-db/consignments/{id}`）的业务字段 MUST 落结构载体，
// 否则保存即"写进无人读的列"。本段 = 该写路的**单一写件**（产品行经
// `contract_writer::insert_product_row_tx`，停靠桥形态同本文件 Step 4a 与
// `contract_writer::insert_product_stops_tx`，明细/标量形态同 Step 2/5）。
// ═══════════════════════════════════════════════════════════════════════════════

/// 编辑保存的结构字段（与 `identity-org` 的 `UpdateConsignmentRequest` 同形；
/// 全 `None` = 无结构写，调用方直接跳过）。
#[derive(Debug, Clone, Default)]
pub struct UpdateStructuresInput {
    /// 货物描述 → 明细主行 `notice`+`comments`（OA/读侧口径 `COALESCE(comments, notice)`）
    pub cargo: Option<String>,
    /// 体积 m³ → 明细 `qk_v_qty` → `zc_id_scal-volume`
    pub cbm: Option<f64>,
    /// 成交单价（元/吨）→ 明细 `qk_price` → `zc_id_scal-price`
    pub price: Option<f64>,
    /// 起运地（`zc_id_place` id 或名称首匹配）→ 停靠桥 `ST-DEPART` / `"over-seq"`=1
    pub origin: Option<String>,
    /// 目的地 → 停靠桥 `ST-ARRIVE` / `"over-seq"`=2
    pub dest: Option<String>,
    /// 预计提货（`YYYY-MM-DD` / `YYYY-MM-DDTHH:MM`，+08）→ 产品 `qk_period.date_st`
    pub pickup_time: Option<String>,
    /// 预计送达 → `date_ed`
    pub eta: Option<String>,
    /// 承运商（`zc_id_subjects` id 或名称首匹配）→ 产品 `"fk_subj-provider"`
    pub carrier: Option<String>,
    /// 货量（吨）→ 明细 `qk_w_qty` → `zc_id_scal-weight`（按入参原值落库，MUST NOT 取整）
    pub weight_ton: Option<f64>,
    /// 运费金额（元）→ 明细 `qk_amount` → `zc_id_scal-amount`（按分保留两位，同建单口径）
    pub amount: Option<f64>,
}

impl UpdateStructuresInput {
    /// 是否有任何结构字段（空 = 调用方零结构写，仍只走主表 `comments` 备注）
    pub fn is_empty(&self) -> bool {
        let blank = |v: &Option<String>| v.as_deref().map(str::trim).is_none_or(str::is_empty);
        self.cargo
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
            && self.cbm.is_none()
            && self.price.is_none()
            && self.weight_ton.is_none()
            && self.amount.is_none()
            && blank(&self.origin)
            && blank(&self.dest)
            && blank(&self.pickup_time)
            && blank(&self.eta)
            && blank(&self.carrier)
    }
}

/// 停靠侧（`"over-seq"` 1 = 起 / 2 = 讫；`ck_category` = ST-DEPART / ST-ARRIVE）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopSide {
    Origin,
    Dest,
}

impl StopSide {
    fn tag(self) -> &'static str {
        match self {
            Self::Origin => "D",
            Self::Dest => "A",
        }
    }
    fn notice(self) -> &'static str {
        match self {
            Self::Origin => "起点",
            Self::Dest => "终点",
        }
    }
    fn cate_code(self) -> &'static str {
        match self {
            Self::Origin => "ST-DEPART",
            Self::Dest => "ST-ARRIVE",
        }
    }
    fn over_seq(self) -> i16 {
        match self {
            Self::Origin => 1,
            Self::Dest => 2,
        }
    }
    fn field(self) -> &'static str {
        match self {
            Self::Origin => "origin",
            Self::Dest => "dest",
        }
    }
}

/// 委托行码（行码派生用）；无 code 回落 `ORD-{id}`
async fn consignment_code_tx(conn: &mut PgConnection, id: i64) -> Result<String, AliothError> {
    let code: Option<String> = sqlx::query_scalar(
        r#"SELECT code FROM "isahl"."zc_id_orde-land" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?
    .flatten();
    Ok(code
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| format!("ORD-{id}")))
}

/// 该委托的运输产品 id：明细 `fk_deal` 优先 → `fk_previous` 反查兜底——**与读侧
/// `tasks.rs` 起讫/时段取产品的 COALESCE 同序**（产品是停靠桥与时段的家园）。
async fn find_product_row_tx(
    conn: &mut PgConnection,
    consignment_id: i64,
) -> Result<Option<i64>, AliothError> {
    let found: Option<i64> = sqlx::query_scalar(
        r#"SELECT COALESCE(
             (SELECT fp.id FROM "isahl"."zc_id_deta-trade_order" d
                JOIN "isahl"."zc_id_prod-freight_road-sales" fp
                  ON fp.id = d.fk_deal AND fp.deleted_at IS NULL
               WHERE d.fk_list = $1 AND d.deleted_at IS NULL
               ORDER BY CASE WHEN d.code LIKE 'DTL-LDG-%' THEN 1 ELSE 0 END, d.id LIMIT 1),
             (SELECT fpc.id FROM "isahl"."zc_id_prod-freight_road-sales" fpc
               WHERE fpc.fk_previous = $1 AND fpc.deleted_at IS NULL
               ORDER BY fpc.id LIMIT 1))"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?
    .flatten();
    Ok(found)
}

/// 委托产品（缺失则经 `contract_writer::insert_product_row_tx` 单一写件补建：
/// `code = PRD-{委托code}`、`fk_previous` = 委托，同 Step 4 创建期形态）。
async fn ensure_product_row_tx(
    conn: &mut PgConnection,
    consignment_id: i64,
    user_id: i64,
) -> Result<i64, AliothError> {
    if let Some(existing) = find_product_row_tx(&mut *conn, consignment_id).await? {
        return Ok(existing);
    }
    let code = consignment_code_tx(&mut *conn, consignment_id).await?;
    let product_code = format!("PRD-{code}");
    // 同码产品已存在（软删不计）：复用而非撞唯一性校验
    if let Some(existing) = sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM "isahl"."zc_id_prod-freight_road-sales"
           WHERE code = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .bind(&product_code)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?
    {
        return Ok(existing);
    }
    // 主体对：买方 = 委托客户（fk_subject）、卖方 = 运营组织（fk_object）——
    // 均缺失时显式失败（无主体无法成行，静默丢产品会让起讫/时段无处可落）
    let subjects: Option<(i64, i64)> = sqlx::query_as(
        r#"SELECT COALESCE(fk_subject, 0), COALESCE(fk_object, 0)
           FROM "isahl"."zc_id_orde-land" WHERE id = $1"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    let (demand_subject, provider_subject) = match subjects {
        Some((d, p)) if d > 0 && p > 0 => (d, p),
        _ => {
            return Err(AliothError::Validation {
                field: "carrier".into(),
                message: format!(
                    "委托 {code} 缺客户/运营主体（fk_subject/fk_object）——无法补建运输产品以承载起讫/时段"
                ),
            })
        }
    };

    contract_writer::insert_product_row_tx(
        &mut *conn,
        &contract_writer::ProductRowInput {
            is_sales: true,
            code: &product_code,
            notice: &format!("委托 {code} 运输产品"),
            comments: "委托编辑补建（结构写路）",
            demand_subject,
            provider_subject,
            line_id: None,
            vehicle_form_id: None,
            price_id: None,
            weight_id: None,
            period_id: None,
            previous_id: Some(consignment_id),
            // 委托链坐标（本文件头注 "GC"/"FJA"/"↓_BE"）
            fn_code: "↓_BE",
            scene_code: "GC",
            factor_code: "FJA",
            user_id,
        },
    )
    .await
}

/// 该委托主明细行（读侧 `cargo` 取首条非 `DTL-LDG-%`）；缺失则按 Step 5 形态补建。
async fn ensure_detail_row_tx(
    conn: &mut PgConnection,
    consignment_id: i64,
    user_id: i64,
) -> Result<i64, AliothError> {
    if let Some(existing) = sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM "isahl"."zc_id_deta-trade_order"
           WHERE fk_list = $1 AND deleted_at IS NULL
           ORDER BY CASE WHEN code LIKE 'DTL-LDG-%' THEN 1 ELSE 0 END, id LIMIT 1"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?
    {
        return Ok(existing);
    }
    let code = consignment_code_tx(&mut *conn, consignment_id).await?;
    let product_id = find_product_row_tx(&mut *conn, consignment_id).await?;
    let (dk_scene, dk_factor, dk_function) =
        resolve_coords(&mut *conn, WriterDk::TradeOrderDetail).await?;
    sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO "isahl"."zc_id_deta-trade_order"
           (code, notice, qk_qty, fk_list, fk_goods, fk_deal, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, 1, $3, $4, $4, $5, $6, $7, $8)
           RETURNING id"#,
    )
    .bind(format!("DTL-{code}"))
    .bind(format!("委托 {code} 货物明细"))
    .bind(consignment_id)
    .bind(product_id)
    .bind(user_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)
}

/// 明细标量族静态 SQL（编译期固化：列名与表名均为字面量，正文单一来源）
struct DetailScalarSql {
    select_ref: &'static str,
    update_scalar: &'static str,
    insert_scalar: &'static str,
    set_column: &'static str,
}

macro_rules! detail_scalar_sql {
    ($column:literal, $scalar_table:literal) => {
        DetailScalarSql {
            select_ref: concat!(
                "SELECT ", $column, " FROM \"isahl\".\"zc_id_deta-trade_order\" WHERE id = $1"
            ),
            update_scalar: concat!(
                "UPDATE \"isahl\".\"", $scalar_table,
                "\" SET mark = $1, notice = $2, updated_by_id = $3, updated_at = NOW() WHERE id = $4"
            ),
            insert_scalar: concat!(
                "INSERT INTO \"isahl\".\"", $scalar_table,
                "\" (code, notice, mark, created_by_id) VALUES ($1, $2, $3, $4) RETURNING id"
            ),
            set_column: concat!(
                "UPDATE \"isahl\".\"zc_id_deta-trade_order\" SET ", $column,
                " = $1, updated_by_id = $2, updated_at = NOW() WHERE id = $3"
            ),
        }
    };
}

/// 体积（`qk_v_qty` → `zc_id_scal-volume`，码前缀 `VOL-`）
const DETAIL_SCALAR_VOLUME: DetailScalarSql = detail_scalar_sql!("qk_v_qty", "zc_id_scal-volume");
/// 单价（`qk_price` → `zc_id_scal-price`，码前缀 `PRC-`）
const DETAIL_SCALAR_PRICE: DetailScalarSql = detail_scalar_sql!("qk_price", "zc_id_scal-price");
/// 货量（`qk_w_qty` → `zc_id_scal-weight`，码前缀 `WT-`）
const DETAIL_SCALAR_WEIGHT: DetailScalarSql = detail_scalar_sql!("qk_w_qty", "zc_id_scal-weight");
/// 金额（`qk_amount` → `zc_id_scal-amount`，码前缀 `AMT-`）
const DETAIL_SCALAR_AMOUNT: DetailScalarSql = detail_scalar_sql!("qk_amount", "zc_id_scal-amount");

/// 明细标量落值（`qk_v_qty` / `qk_price` 同形）：空引用 → 建标量并回挂；有引用 → 更新标量真值。
/// 表名/列名由 [`DetailScalarSql`] 编译期固化（本文件静态字面量，非用户输入）。
async fn apply_detail_scalar_tx(
    conn: &mut PgConnection,
    consignment_id: i64,
    user_id: i64,
    sqls: &'static DetailScalarSql,
    scalar_prefix: &'static str,
    notice: String,
    mark: f64,
) -> Result<(), AliothError> {
    let detail_id = ensure_detail_row_tx(&mut *conn, consignment_id, user_id).await?;
    let cur: Option<i64> = sqlx::query_scalar(sqls.select_ref)
        .bind(detail_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?
        .flatten();

    match cur {
        Some(scale_id) => {
            sqlx::query(sqls.update_scalar)
                .bind(mark)
                .bind(notice)
                .bind(user_id)
                .bind(scale_id)
                .execute(&mut *conn)
                .await
                .map_err(AliothError::from_sqlx)?;
        }
        None => {
            let code = consignment_code_tx(&mut *conn, consignment_id).await?;
            let scale_id: i64 = sqlx::query_scalar(sqls.insert_scalar)
                .bind(format!("{scalar_prefix}{code}"))
                .bind(notice)
                .bind(mark)
                .bind(user_id)
                .fetch_one(&mut *conn)
                .await
                .map_err(AliothError::from_sqlx)?;
            sqlx::query(sqls.set_column)
                .bind(scale_id)
                .bind(user_id)
                .bind(detail_id)
                .execute(&mut *conn)
                .await
                .map_err(AliothError::from_sqlx)?;
        }
    }
    Ok(())
}

/// 地点解析：`zc_id_place` id（数字直取）或名称首匹配；未命中 → `Validation`（fail-visible）。
async fn resolve_place_tx(
    conn: &mut PgConnection,
    spec: &str,
    field: &str,
) -> Result<i64, AliothError> {
    let raw = spec.trim();
    if let Ok(id) = raw.parse::<i64>() {
        if id > 0 {
            let exists: Option<i64> = sqlx::query_scalar(
                r#"SELECT id FROM "isahl"."zc_id_place" WHERE id = $1 AND deleted_at IS NULL LIMIT 1"#,
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
            return exists.ok_or_else(|| AliothError::Validation {
                field: field.into(),
                message: format!("{field}: 地点 id {id} 不存在"),
            });
        }
    }
    let found: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_place"
           WHERE notice = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .bind(raw)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    found.ok_or_else(|| AliothError::Validation {
        field: field.into(),
        message: format!("{field}: 找不到停靠行「{raw}」（zc_id_place 无同名地点）"),
    })
}

/// 起讫 → 停靠桥 `zc_id_prod-transport_rr_stop`（`ref_left` = 委托产品、`ref_right` = 停靠行、
/// `ck_category` = ST-DEPART/ST-ARRIVE、`"over-seq"` = 1/2）。
///
/// 唯一键 `uq_zc_id_prod-transport_rr_stop_ref_left_ref_right_qk_period`
/// = (`ref_left`, `ref_right`, COALESCE(`qk_period`,-1)) **不含 `ck_category`** ⇒
/// 同一产品上「地点」只能出现一行，「起讫同址」时两侧共用一行——故写入次序为
/// **地点优先**（先认领/复用该地点已占用的行并改判据，再清掉本侧指向旧地点的陈旧行），
/// 否则「把讫点改成起点」会撞唯一键（真库探针实测 23505，端点 500）。
/// 保证每侧至多一行（读侧按 `ck_category` 取值，多行会取到任意一行）。
async fn apply_stop_tx(
    conn: &mut PgConnection,
    consignment_id: i64,
    user_id: i64,
    side: StopSide,
    spec: &str,
) -> Result<(), AliothError> {
    let product_id = ensure_product_row_tx(&mut *conn, consignment_id, user_id).await?;
    let place_id = resolve_place_tx(&mut *conn, spec, side.field()).await?;
    let cate_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_cate-traffic" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(side.cate_code())
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    let cate_id = cate_id.ok_or_else(|| {
        AliothError::Internal(format!(
            "停靠类别缺失（zc_id_cate-traffic {}）——无法落起讫停靠桥",
            side.cate_code()
        ))
    })?;

    let own: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_prod-transport_rr_stop"
           WHERE ref_left = $1 AND ck_category = $2 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .bind(product_id)
    .bind(cate_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;

    let occupied: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_prod-transport_rr_stop"
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
    )
    .bind(product_id)
    .bind(place_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;

    match (occupied, own) {
        // ① 地点已有行 → 认领（改判据为本侧）；本侧另有行指向旧地点则软删（防同侧双行）
        (Some(row_id), own) => {
            sqlx::query(
                r#"UPDATE "isahl"."zc_id_prod-transport_rr_stop"
                   SET ck_category = $1, ref_right = $2, notice = $3, "over-seq" = $4,
                       deleted_at = NULL, deleted_by_id = NULL,
                       updated_by_id = $5, updated_at = NOW()
                   WHERE id = $6"#,
            )
            .bind(cate_id)
            .bind(place_id)
            .bind(side.notice())
            .bind(side.over_seq())
            .bind(user_id)
            .bind(row_id)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
            if let Some(own_id) = own.filter(|own_id| *own_id != row_id) {
                sqlx::query(
                    r#"UPDATE "isahl"."zc_id_prod-transport_rr_stop"
                       SET deleted_at = NOW(), deleted_by_id = $1, updated_at = NOW() WHERE id = $2"#,
                )
                .bind(user_id)
                .bind(own_id)
                .execute(&mut *conn)
                .await
                .map_err(AliothError::from_sqlx)?;
            }
        }
        // ② 地点无行、本侧有行 → 改指向新地点
        (None, Some(own_id)) => {
            sqlx::query(
                r#"UPDATE "isahl"."zc_id_prod-transport_rr_stop"
                   SET ref_right = $1, notice = $2, "over-seq" = $3, updated_by_id = $4, updated_at = NOW()
                   WHERE id = $5"#,
            )
            .bind(place_id)
            .bind(side.notice())
            .bind(side.over_seq())
            .bind(user_id)
            .bind(own_id)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
        }
        // ③ 全无 → 补建桥行（行码同 contract-writer `STOP-{product_code}-{D|A}`）
        (None, None) => {
            let code = consignment_code_tx(&mut *conn, consignment_id).await?;
            sqlx::query(
                r#"INSERT INTO "isahl"."zc_id_prod-transport_rr_stop"
                   (code, notice, ref_left, ref_right, ck_category, "over-seq", created_by_id)
                   VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING"#,
            )
            .bind(format!("STOP-PRD-{code}-{}", side.tag()))
            .bind(side.notice())
            .bind(product_id)
            .bind(place_id)
            .bind(cate_id)
            .bind(side.over_seq())
            .bind(user_id)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
        }
    }
    Ok(())
}

/// 时间入参解析（+08 本地 → UTC），同本文件 `create_full` 的 `parse_local`
fn parse_local_ts(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let t = s.trim();
    chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M")
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
                .map(|d| d.and_time(chrono::NaiveTime::MIN))
        })
        .ok()
        .map(|n| n.and_utc() - chrono::Duration::hours(8))
}

/// 预计提货/送达 → 委托产品 `qk_period` → `zc_id_segm-date.date_st`/`date_ed`
/// （读侧 `pickup_display`/`eta_display` 取该时段）。只覆盖**传入**的一侧。
async fn apply_period_tx(
    conn: &mut PgConnection,
    consignment_id: i64,
    user_id: i64,
    pickup_time: Option<&str>,
    eta: Option<&str>,
) -> Result<(), AliothError> {
    let pickup_ts = pickup_time
        .filter(|s| !s.trim().is_empty())
        .map(parse_local_ts);
    let eta_ts = eta.filter(|s| !s.trim().is_empty()).map(parse_local_ts);
    if matches!(pickup_ts, Some(None)) {
        return Err(AliothError::Validation {
            field: "pickup_time".into(),
            message: "预计提货格式非法（接受 YYYY-MM-DD 或 YYYY-MM-DDTHH:MM）".into(),
        });
    }
    if matches!(eta_ts, Some(None)) {
        return Err(AliothError::Validation {
            field: "eta".into(),
            message: "预计送达格式非法（接受 YYYY-MM-DD 或 YYYY-MM-DDTHH:MM）".into(),
        });
    }
    let (pickup_ts, eta_ts) = (pickup_ts.flatten(), eta_ts.flatten());
    if pickup_ts.is_none() && eta_ts.is_none() {
        return Ok(());
    }

    let product_id = ensure_product_row_tx(&mut *conn, consignment_id, user_id).await?;
    let cur: Option<i64> = sqlx::query_scalar(
        r#"SELECT qk_period FROM "isahl"."zc_id_prod-freight_road-sales" WHERE id = $1"#,
    )
    .bind(product_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?
    .flatten();

    match cur {
        Some(seg_id) => {
            sqlx::query(
                r#"UPDATE "isahl"."zc_id_segm-date"
                   SET date_st = COALESCE($1, date_st), date_ed = COALESCE($2, date_ed),
                       updated_by_id = $3, updated_at = NOW()
                   WHERE id = $4"#,
            )
            .bind(pickup_ts)
            .bind(eta_ts)
            .bind(user_id)
            .bind(seg_id)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
        }
        None => {
            let code = consignment_code_tx(&mut *conn, consignment_id).await?;
            let seg_id: i64 = sqlx::query_scalar(
                r#"INSERT INTO "isahl"."zc_id_segm-date" (code, notice, date_st, date_ed, created_by_id)
                   VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
            )
            .bind(format!("PERIOD-{code}"))
            .bind("预计提货-预计送达")
            .bind(pickup_ts)
            .bind(eta_ts)
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
            sqlx::query(
                r#"UPDATE "isahl"."zc_id_prod-freight_road-sales"
                   SET qk_period = $1, updated_by_id = $2, updated_at = NOW() WHERE id = $3"#,
            )
            .bind(seg_id)
            .bind(user_id)
            .bind(product_id)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
        }
    }
    Ok(())
}

/// 承运商 → 委托产品 `"fk_subj-provider"`（读侧 `carrier_name` 的唯一结构来源）。
async fn apply_carrier_tx(
    conn: &mut PgConnection,
    consignment_id: i64,
    user_id: i64,
    spec: &str,
) -> Result<(), AliothError> {
    let raw = spec.trim();
    let subject_id: Option<i64> = match raw.parse::<i64>() {
        Ok(id) if id > 0 => sqlx::query_scalar(
            r#"SELECT id FROM "isahl"."zc_id_subjects" WHERE id = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?,
        _ => sqlx::query_scalar(
            r#"SELECT id FROM "isahl"."zc_id_subjects"
               WHERE notice = $1 AND deleted_at IS NULL ORDER BY id LIMIT 1"#,
        )
        .bind(raw)
        .fetch_optional(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?,
    };
    let subject_id = subject_id.ok_or_else(|| AliothError::Validation {
        field: "carrier".into(),
        message: format!("carrier: 找不到承运商主体「{raw}」"),
    })?;
    let product_id = ensure_product_row_tx(&mut *conn, consignment_id, user_id).await?;
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_prod-freight_road-sales"
           SET "fk_subj-provider" = $1, updated_by_id = $2, updated_at = NOW() WHERE id = $3"#,
    )
    .bind(subject_id)
    .bind(user_id)
    .bind(product_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    Ok(())
}

/// **委托编辑的结构写路单一实现**（`PUT /service/isahl-db/consignments/{id}`）：
/// 把编辑表单的业务字段落到结构载体（明细 / 停靠桥 / 时段 / 标量），`comments` 仅存展示摘要。
///
/// 事务由调用方管理（同 `create_full` 约定）；`input.is_empty()` 由调用方短路。
/// 幂等：全部「先查后写」，重复保存不增生结构行；前置缺失显式失败（fail-visible）。
pub async fn update_consignment_structures_tx(
    conn: &mut PgConnection,
    consignment_id: i64,
    input: &UpdateStructuresInput,
    user_id: i64,
) -> Result<(), AliothError> {
    // 主表存在性（不存在即 404 语义，与其他字段写路一致）
    let exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM "isahl"."zc_id_orde-land" WHERE id = $1 AND deleted_at IS NULL)"#,
    )
    .bind(consignment_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    if !exists {
        return Err(AliothError::NotFound(format!(
            "委托 {consignment_id} 不存在或已删除"
        )));
    }

    if let Some(cargo) = input
        .cargo
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let detail_id = ensure_detail_row_tx(&mut *conn, consignment_id, user_id).await?;
        sqlx::query(
            r#"UPDATE "isahl"."zc_id_deta-trade_order"
               SET notice = $1, comments = $1, updated_by_id = $2, updated_at = NOW()
               WHERE id = $3"#,
        )
        .bind(cargo)
        .bind(user_id)
        .bind(detail_id)
        .execute(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?;
    }
    if let Some(cbm) = input.cbm {
        apply_detail_scalar_tx(
            &mut *conn,
            consignment_id,
            user_id,
            &DETAIL_SCALAR_VOLUME,
            "VOL-",
            format!("{cbm} m³"),
            cbm,
        )
        .await?;
    }
    if let Some(price) = input.price {
        apply_detail_scalar_tx(
            &mut *conn,
            consignment_id,
            user_id,
            &DETAIL_SCALAR_PRICE,
            "PRC-",
            format!("{price} 元/吨"),
            price,
        )
        .await?;
    }
    // 货量（吨）：结构真值按**入参原值**落库（MUST NOT 取整——`mark` 为 numeric(30,10)，
    // 旧 `identity-org` 旁路的 `ROUND` 会篡改货量并与 notice 原文自相矛盾）
    if let Some(weight_ton) = input.weight_ton {
        apply_detail_scalar_tx(
            &mut *conn,
            consignment_id,
            user_id,
            &DETAIL_SCALAR_WEIGHT,
            "WT-",
            format!("{weight_ton}吨"),
            weight_ton,
        )
        .await?;
    }
    // 金额（元）：按分保留两位（与建单写路 `create_full` 同口径）
    if let Some(amount) = input.amount {
        let amount = (amount * 100.0).round() / 100.0;
        apply_detail_scalar_tx(
            &mut *conn,
            consignment_id,
            user_id,
            &DETAIL_SCALAR_AMOUNT,
            "AMT-",
            format!("¥{amount:.2}"),
            amount,
        )
        .await?;
    }
    if let Some(origin) = input
        .origin
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        apply_stop_tx(
            &mut *conn,
            consignment_id,
            user_id,
            StopSide::Origin,
            origin,
        )
        .await?;
    }
    if let Some(dest) = input
        .dest
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        apply_stop_tx(&mut *conn, consignment_id, user_id, StopSide::Dest, dest).await?;
    }
    if input.pickup_time.is_some() || input.eta.is_some() {
        apply_period_tx(
            &mut *conn,
            consignment_id,
            user_id,
            input.pickup_time.as_deref(),
            input.eta.as_deref(),
        )
        .await?;
    }
    if let Some(carrier) = input
        .carrier
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        apply_carrier_tx(&mut *conn, consignment_id, user_id, carrier).await?;
    }
    Ok(())
}
