//! 委托写链事务（create_full）——迁移自 transport-dispatch
//! `repositories::FjdRepository::create_consignment_inner`（原 600+ 行表级全链，行为等价迁移）：
//!
//! Step 顺序：货描校验 → TrafficLine 校验 → scal-weight/amount → orde-land 主档（+rr_contract 幂等桥）
//! → segm-date 时间窗 → scal-price（成交单价）→ freight_road-sales（+request + prod-sales 桥）
//! → rr_stop×2（ST-DEPART/ARRIVE）→ prod-loading → storage 运力占用 → com-voucher 凭证（守卫）
//! → deta-trade_order（DTL + DTL-LDG）→ 货描标签桥。
//!
//! 主体语义参数化：`customer_id`/运营组织解析外置（WriteContext），事务内不再按 user 解析组织。

use crate::contracts::{insert_order_contracts_tx, OrderContractsInput};
use crate::coords::{resolve_coords, WriterDk};
use crate::mirror::{insert_order_mirror_tx, OrderMirrorInput};
use crate::models::{CreateConsignmentInput, CreateConsignmentOutput, WriteContext};
use common::AliothError as WriterError;

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

    // ── Step 2: 创建标量 (weight + volume + price) ──
    // 货量按入参原值落库：`zc_id_scal-weight.mark` = numeric(30,10)，小数货量是常态
    // （取整会篡改委托货量，并与同一标量 notice 的原文自相矛盾）；金额按分保留两位。
    let weight_scale_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_scal-weight" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_uid(), $1, $2, $3::numeric, $4)
           RETURNING id"#,
    )
    .bind(format!("WT-{}", chrono::Utc::now().timestamp()))
    .bind(format!("{}吨", req.weight_ton))
    .bind(req.weight_ton)
    .bind(ctx.actor_user_id)
    .fetch_one(&mut *conn)
    .await?;

    let amount_scale_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_scal-amount" (id, code, notice, mark, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, ROUND($3::numeric, 2), $4)
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
    let consign_notice = format!("{}→{} {}", origin_name, dest_name, req.cargo_desc);
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
                   SELECT isahl.gen_next_zuid(), $1, $2, $3, $4, $5
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

    // ── 下单合同对（用户裁决 2026-09-11）：诉求合同 + 销售合同 及其一式两份镜像，
    //    并挂 `zc_id_order_rr_contract` 桥（合同侧形态 = 实现·范例，由 ↓.GG 派生）──
    // 小委托裁剪（D6）：合同足迹已由选商（矩阵 #5 分单）落库 ⇒ MUST NOT 重复建下单合同对；
    // 但**镜像订单照建**（矩阵 #2「镜像行与主行同等待遇」；规约 §3 小委托「不建」清单为
    // 封闭枚举，未含镜像订单——小委托的对方账副本必须完整）。
    if !req.sub_consignment {
        let (_dmd, _dmd_mirror, _sales, _sales_mirror) = insert_order_contracts_tx(
            conn,
            consignment_id,
            &OrderContractsInput {
                consign_code: &consign_code,
                buyer: ctx.customer_subject_id,
                seller: ctx.operator_org_id,
                fn_code: "↓.GG",
                user_id: ctx.actor_user_id,
            },
        )
        .await?;
    }

    // ── 一式两份镜像订单（矩阵 #2，用户裁决 2026-09-11）──
    // 同表同类别、甲/乙主体互换（fk_subject↔fk_object）、code=`{code}-R`；
    // 互链走**叶子**桥 `zc_id_lifecycle_rr_form`（非叶 `zc_id_lifecycle_rr_non_self` 82 子表不可直插）。
    // 镜像行 + 桥统一经 `mirror::insert_order_mirror_tx` 单源（与派车运单链同实现）；
    // 镜像行内已补 ak_permit_user（读侧行级权属可见）；主行已有状态时镜像即时同位。
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
    let eta_seg_id: Option<i64> = match (pickup_ts, eta_ts) {
        (None, None) => None,
        (p, e) => Some(
            sqlx::query_scalar(
                r#"INSERT INTO "isahl"."zc_id_segm-date"
                   (id, code, notice, date_st, date_ed, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5)
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
    };
    // 成交单价标量（元/吨；重量为零或无金额时 NULL）
    let prd_price_id: Option<i64> = if req.price_amount > 0.0 && req.weight_ton > 0.0 {
        let unit_price = rust_decimal::Decimal::try_from(req.price_amount / req.weight_ton)
            .unwrap_or_default()
            .round_dp(2);
        Some(
            sqlx::query_scalar(
                r#"INSERT INTO "isahl"."zc_id_scal-price" (id, code, notice, mark, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4)
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
                       VALUES (isahl.gen_next_zuid(), $1, $2, $3::numeric, $4)
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
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6)"#,
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
    let pool_product: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT r.ref_left FROM "isahl"."zc_id_file_rr_url" r
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
    // 坐标三元组（§6.12/§4.3.3）：装载包装 dk 经静态绑定解析（禁硬编码 ZUID）
    let (ldg_dk_scene, ldg_dk_factor, ldg_dk_function) =
        resolve_coords(conn, WriterDk::LoadingPackage).await?;
    let loading_product_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_prod-loading-request"
           (id, code, notice, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6)
           RETURNING id"#,
    )
    .bind(format!("LDG-{}", consign_code))
    .bind(&req.cargo_desc)
    .bind(ctx.actor_user_id)
    .bind(ldg_dk_scene)
    .bind(ldg_dk_factor)
    .bind(ldg_dk_function)
    .fetch_one(&mut *conn)
    .await?;

    // ── Step 5: 创建委托运力占用记录 (production_rr_storage)，qk_qty = 委托重量 ──
    // 小委托裁剪（D6）：同一批货量大委托下单时已扣运力占用，MUST NOT 重复扣减 → inventory_id = 0。
    let inventory_id: i64 = if req.sub_consignment {
        0
    } else {
        sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_file_rr_url"
               (id, code, notice, comments, ref_left, ref_right, qk_qty, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7)
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
        // 损坏凭证防御（qk_income/qk_outgo 标量引用不可解析 → 硬拒绝）
        let corrupt: (i64,) = sqlx::query_as(
            r#"SELECT COUNT(*) FROM "isahl"."zc_id_stat-sto-voucher" v
               LEFT JOIN "isahl"."zc_id_scale" si ON si.id = v.qk_income
               LEFT JOIN "isahl"."zc_id_scale" so ON so.id = v.qk_outgo
               WHERE v.fk_production = $1 AND v.deleted_at IS NULL
                 AND ((v.qk_income IS NOT NULL AND si.id IS NULL)
                   OR (v.qk_outgo IS NOT NULL AND so.id IS NULL))"#,
        )
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
        let voucher_out_id: Option<i64> = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_stat-com-voucher"
               (id, code, notice, comments, fk_production, "fk_subj-storage", "fk_obj-storage",
                qk_outgo, "ck_sto-title", dk_scene, dk_factor, dk_function, _t_, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $5, $6,
                       (SELECT id FROM "isahl"."zc_id_cate-sto-title" WHERE code = 'STO-SHELF' LIMIT 1),
                       $7, $8, $9, $10, $11)
               ON CONFLICT (code) WHERE deleted_at IS NULL DO NOTHING
               RETURNING id"#,
        )
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
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_stat-com-voucher"
               (id, code, notice, comments, fk_production, "fk_subj-storage", "fk_obj-storage",
                qk_income, "ck_sto-title", dk_scene, dk_factor, dk_function, _t_, created_by_id)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $5, $6,
                       (SELECT id FROM "isahl"."zc_id_cate-sto-title" WHERE code = 'STO-FULFILL' LIMIT 1),
                       $7, $8, $9, $10, $11)
               ON CONFLICT (code) WHERE deleted_at IS NULL DO NOTHING"#,
        )
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
            rec.insert("fk_production".to_string(), serde_json::json!(pool_pid));
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
    // 五 FK 接线（契约 #3）：fk_demand=A 的 REQ、fk_goods=B 的 sales 范例（无则 NULL）、
    // fk_deal=B 的 sales 实例（PRD）、fk_biller=B、fk_counterparty=A
    // 坐标三元组（§6.12）：明细行 dk 经静态绑定解析一次，本步骤两条 INSERT 复用（禁硬编码 ZUID）
    let (dtl_dk_scene, dtl_dk_factor, dtl_dk_function) =
        resolve_coords(conn, WriterDk::TradeOrderDetail).await?;
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_deta-trade_order"
           (id, code, notice, comments, qk_qty, qk_amount, qk_w_qty, qk_price, fk_list, fk_goods, fk_deal, fk_demand,
            fk_biller, fk_counterparty, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)"#,
    )
    .bind(format!("DTL-{}", consign_code))
    .bind(format!("{} {}吨", req.cargo_desc, req.weight_ton))
    .bind(req.cargo_desc.clone())
    .bind(amount_scale_id)
    .bind(weight_scale_id)
    .bind(prd_price_id)
    .bind(consignment_id)
    .bind(pool_product)
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
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_deta-trade_order"
           (id, code, notice, fk_list, fk_goods, qk_w_qty, qk_amount, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
    )
    .bind(format!("DTL-LDG-{}", consign_code))
    .bind(format!("{} 装载包装", req.cargo_desc))
    .bind(consignment_id)
    .bind(loading_product_id)
    .bind(weight_scale_id)
    .bind(amount_scale_id)
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
               SELECT isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $5
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
