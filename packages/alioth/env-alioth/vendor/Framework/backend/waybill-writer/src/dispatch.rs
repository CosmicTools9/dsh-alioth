//! 派车核心事务（迁自 transport-dispatch `repositories/dispatch_core.rs`，单份实现）。

use common::operator_org::resolve_operator_org;
use common::AliothError as ApiError;
use consignment_writer::{insert_order_mirror_tx, sync_mirror_status_tx, OrderMirrorInput};
use rust_decimal::Decimal;

use crate::bom::{dispatch_vehicle_plate, ensure_default_capacity_pool};
use crate::models::DispatchParams;
use crate::ontology::{resolve_ontology_coords, DkEntity};

/// 委托层 B（运营主体）解析——供「非平台运营用户」发起 B 侧写动作时使用
/// （承运商自有系统走开放 API 服务令牌：服务主体 `auth_users.entity_id IS NULL`，
/// 无运营组织可解析，B 只能由委托派生；平台运营用户仍走绑定组织门禁）。
///
/// 来源优先级：
/// ① 委托明细 `fk_deal` → 产品 `fk_subj-provider`——`consignment-writer` 建单**不写**
///    产品 `fk_previous`，明细桥是唯一可用链路（实测 PRD 行 `fk_previous IS NULL`）；
/// ② 产品 `fk_previous` = 委托且 `code LIKE 'PRD-%'`（OA / 历史路径）。
///
/// 两者皆无 → `Ok(None)`，调用方回落系统运营主体。
pub async fn resolve_consignment_operator_org(
    conn: &mut sqlx::PgConnection,
    consignment_id: i64,
) -> Result<Option<i64>, ApiError> {
    let by_detail: Option<i64> = sqlx::query_scalar(
        r#"SELECT fp."fk_subj-provider"
           FROM "isahl"."zc_id_deta-trade_order" d
           JOIN "isahl"."zc_id_prod-freight_road-sales" fp
             ON fp.id = d.fk_deal AND fp.deleted_at IS NULL
           WHERE d.fk_list = $1 AND d.deleted_at IS NULL
             AND fp."fk_subj-provider" IS NOT NULL
           ORDER BY CASE WHEN d.code LIKE 'DTL-LDG-%' THEN 1 ELSE 0 END, d.id
           LIMIT 1"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    if by_detail.is_some() {
        return Ok(by_detail);
    }
    let by_previous: Option<i64> = sqlx::query_scalar(
        r#"SELECT fp."fk_subj-provider"
           FROM "isahl"."zc_id_prod-freight_road-sales" fp
           WHERE fp.fk_previous = $1 AND fp.deleted_at IS NULL
             AND fp.code LIKE 'PRD-%' AND fp."fk_subj-provider" IS NOT NULL
           ORDER BY fp.id LIMIT 1"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    Ok(by_previous)
}

/// dispatch_vehicles_tx 的可共享事务变体（测试复用，调用方负责提交/回滚）。
///
/// 批量语义（D4-a）：一次事务提交全部车辆，容量池只锁一次、可用量只校验一次，
/// 各车按 allocated_weight 分别扣减（创建独立数量标量），凭证链表达每车装载量。
///
/// `capacity_product_id = None` 时走围栏聚合路径（P1-3，设计文档 §5.2）：
/// 从委托起运地推断围栏，Σ 该围栏下全部线路容量池可用量校验，扣减凭证落委托自身线路容量池。
pub async fn dispatch_vehicles_tx_inner(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    params: DispatchParams<'_>,
) -> Result<Vec<(i64, i64, i64)>, ApiError> {
    let DispatchParams {
        consign_code,
        consignment_id,
        allocations,
        capacity_product_id,
        purchase_price,
        carrier_id,
        user_id,
    } = params;
    // ── Step 0-lock: 委托生命周期行锁 + 生命周期前置（fix-dispatch-inventory-reconciliation D2）──
    // FOR UPDATE 锁定状态行：同一委托并发派车批次在此串行——第二批读到第一批
    // 已提交的 TSP-DSP OUT，断言不穿透。终态/事故态拒绝；新建（无状态记录）可派。
    let lifecycle_code: Option<String> = sqlx::query_scalar(
        r#"SELECT s.code
               FROM "isahl"."zc_id_lifecycle_r_primary-status" ls
               JOIN "isahl"."zc_id_stus-trade" s ON s.id = ls.ref_right
               WHERE ls.ref_left = $1 AND ls.deleted_at IS NULL
               FOR UPDATE OF ls"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(code) = lifecycle_code.as_deref() {
        const DISPATCHABLE: [&str; 2] = ["ST-ACCEPTED", "ST-DISPATCHED"];
        if !DISPATCHABLE.contains(&code) {
            return Err(ApiError::BadRequest(format!(
                "委托状态 {code} 不可派车（仅新建/ST-ACCEPTED/ST-DISPATCHED 可派）"
            )));
        }
    }

    // ── Step 0e-0: reservation 只读断言基数（fix-dispatch-inventory-reconciliation D1）──
    // 下单扣减证据 = COM-{委托号}-OUT 守卫扣减凭证（qk_outgo 标量真值）；
    // ORD-INV 占用行下单时无条件写入，不构成证据。
    // 批注 52834cc1（用户裁决「运力池无需一次性满足委托需求」）：无 COM-OUT 凭证
    // （委托下单时线路未配容量池，如先建委托后建池）不再 fail-closed 硬拒——
    // 降级为以委托整单重量为 reservation 基数（见下方 Step 0e 聚合，防超派底线保留）；
    // 有凭证时仍以下单扣减量为准（防超卖语义不变）。
    let com_out_code = format!("COM-{}-OUT", consign_code);
    let reservation: Option<Decimal> = sqlx::query_scalar(
        r#"SELECT sm.mark
               FROM "isahl"."zc_id_stat-com-voucher" v
               JOIN "isahl"."zc_id_scale" sm ON sm.id = v.qk_outgo
               WHERE v.code = $1 AND v.deleted_at IS NULL
                 AND v."fk_subj-storage" IS NOT NULL
               LIMIT 1"#,
    )
    .bind(&com_out_code)
    .fetch_optional(&mut **tx)
    .await?;
    // Σ 既有派车批次（TSP-DSP OUT 凭证聚合）——批次间记账基数（N1 同源，:0e-0 读数在插入前）
    let dispatched_before: Decimal = sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(sm.mark), 0)
               FROM "isahl"."zc_id_stat-tsp-voucher" v
               JOIN "isahl"."zc_id_scale" sm ON sm.id = v.qk_outgo
               WHERE v.code LIKE 'TSP-DSP-' || $1 || '-%' AND v.code LIKE '%-OUT'
                 AND v.deleted_at IS NULL"#,
    )
    .bind(consign_code)
    .fetch_one(&mut **tx)
    .await?;
    // ── Step 0a: 从委托→产品（fk_previous）→ fk_line 获取线路 ──
    let traffic_line_id: Option<i64> = sqlx::query_scalar(
        // remove-comments-json-embedding 降级（D2）：traffic_line_id 曾读 comments JSON，改读产品真实列 fk_line
        r#"SELECT fp.fk_line
               FROM "isahl"."zc_id_orde-land" o
               JOIN "isahl"."zc_id_prod-freight_road-sales" fp
                 ON fp.fk_previous = o.id AND fp.deleted_at IS NULL AND fp.fk_line IS NOT NULL
               WHERE o.id = $1 AND o.deleted_at IS NULL
               LIMIT 1"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten();
    // ── Step 0b: 线路校验（仅显式路径）— capacity_product_id 必须与委托同线路 ──
    // remove-comments-json-embedding 降级（D3）：跨线路同围栏校验与车辆围栏校验
    // 原经线路/车辆 comments.fence_code/fence_zone，已文本化不可得——围栏推断
    // （Step 0b-0）与车辆围栏校验（Step 0c-veh）一并删除；线路校验降级为仅允许同线路
    // （fail-closed，不误放行跨线路）。
    if let (Some(pid), Some(tlid)) = (capacity_product_id, traffic_line_id) {
        // 容量池产品直接取 fk_line
        let prod_tlid: Option<i64> = sqlx::query_scalar(
            r#"SELECT fk_line
                   FROM "isahl"."zc_id_prod-freight_road-sales"
                   WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(pid)
        .fetch_optional(&mut **tx)
        .await?
        .flatten();

        match prod_tlid {
            Some(pt) if pt == tlid => { /* 同线路，通过 */ }
            Some(_) => {
                return Err(ApiError::BadRequest(
                    "容量池产品与委托单不在同一线路，禁止跨线路派车".into(),
                ));
            }
            None => {
                return Err(ApiError::BadRequest(
                    "容量池产品缺少 fk_line，无法校验线路一致性，禁止派车".into(),
                ));
            }
        }
    }
    // 兼容旧数据：无线路的委托跳过线路校验

    // ── Step 0d: 凭证归属池解析（不再参与可用量校验——可售扣减已在下单完成）──
    // 派车不再做池总量/余额校验（设计契约 §2.4：下单=可售↓、派车=形态迁移；原 transit_net
    // 读径的受理预留双计/字典缺失静默失效/负余额虚增问题随本 change 退役）。
    // effective_capacity_product_id 仅用于 Step 5b tsp 迁移凭证的 fk_production 归属（审计事实）。
    let effective_capacity_product_id: i64 = if let Some(pid) = capacity_product_id {
        pid
    } else {
        let deduction_pool: Option<i64> = sqlx::query_scalar(
                r#"SELECT r.ref_left
                   FROM "isahl"."zc_id_file_rr_url" r
                   LEFT JOIN "isahl"."zc_id_prod-freight_road-sales" pp
                     ON pp.id = r.ref_left AND pp.deleted_at IS NULL
                   WHERE r.ref_right = $1 AND r.deleted_at IS NULL
                     AND r.qk_p_capacity IS NOT NULL
                     -- remove-comments-json-embedding 降级（D3）：instance 类型标记曾存 comments JSON，
                     -- 已文本化不可得——不再区分实例/模板行（诚实降级）
                   ORDER BY CASE WHEN pp.code LIKE 'CAP-SALE-%' THEN 0
                                 WHEN pp.code LIKE 'CAP-TL-%' THEN 1
                                 WHEN pp.code LIKE 'CAP-LINE-%' THEN 2 ELSE 3 END,
                            r.ref_left
                   LIMIT 1"#
            )
            .bind(traffic_line_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten();

        // 用户裁决（2026-09-10）：容量池有默认容量（近乎无限），平台不关注承运商运力——
        // 线路未配池时自动建默认线路池（CAP-LINE-{line}），不再 fail-closed 拒派车。
        match deduction_pool {
            Some(pid) => pid,
            None => match traffic_line_id {
                Some(tlid) => ensure_default_capacity_pool(&mut **tx, tlid).await?,
                None => {
                    return Err(ApiError::BadRequest(
                        "委托无线路信息，无法生成派车迁移凭证".into(),
                    ))
                }
            },
        }
    };

    // 委托整单重量（分配上限校验）——批注轮 71 同款排除 DTL-LDG（装载包装行挂同标量
    // 致聚合 ×2：列表接口已排除，此处保持一致性，否则整单 3000 吨被算成 6000 吨）
    let consignment_weight: Option<Decimal> = sqlx::query_scalar(
        r#"SELECT agg.agg_w_total
               FROM "isahl"."zc_id_orde-land" o
               LEFT JOIN LATERAL (
                   SELECT COALESCE(SUM(qt.mark::numeric), 0) AS agg_qty,
                          COALESCE(SUM(am.mark::numeric), 0) AS agg_amount,
                          COALESCE(SUM(wt.mark::numeric), 0) AS agg_w_total,
                          COALESCE(SUM(vt.mark::numeric), 0) AS agg_v_total,
                          COALESCE(SUM(at.mark::numeric), 0) AS agg_a_total,
                          COALESCE(SUM(ts.mark::numeric), 0) AS agg_ts_total,
                          COALESCE(SUM(dt.mark::numeric), 0) AS agg_d_total
                   FROM "isahl"."zc_id_deta-trade_order" d
                   LEFT JOIN "isahl"."zc_id_scale" qt ON qt.id = d.qk_qty
                   LEFT JOIN "isahl"."zc_id_scal-amount" am ON am.id = d.qk_amount
                   LEFT JOIN "isahl"."zc_id_scal-weight" wt ON wt.id = d.qk_w_qty
                   LEFT JOIN "isahl"."zc_id_scal-volume" vt ON vt.id = d.qk_v_qty
                   LEFT JOIN "isahl"."zc_id_scal-area" at ON at.id = d.qk_a_qty
                   LEFT JOIN "isahl"."zc_id_scal-distance" ts ON ts.id = d.qk_ts_qty
                   LEFT JOIN "isahl"."zc_id_scal-distance" dt ON dt.id = d.qk_d_qty
                   WHERE d.fk_list = o.id AND d.deleted_at IS NULL
                     AND (d.code NOT LIKE 'DTL-LDG-%'
                          OR NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_deta-trade_order" d2
                                         WHERE d2.fk_list = o.id AND d2.code NOT LIKE 'DTL-LDG-%'
                                           AND d2.deleted_at IS NULL))
               ) agg ON true
               WHERE o.id = $1 AND o.deleted_at IS NULL"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten();
    // reservation 基数合并（批注 52834cc1）：有 COM-OUT 凭证用下单扣减量，
    // 无凭证（下单时未配池）降级为委托整单重量——防超派底线保留，不再 fail-closed 硬拒
    let reservation: Option<Decimal> = reservation.or(consignment_weight);

    // ── Step 0e: 分配校验（批量语义 D4-a）──
    if allocations.is_empty() {
        return Err(ApiError::BadRequest("派车车辆列表不能为空".into()));
    }
    let mut seen = std::collections::HashSet::new();
    let mut total_alloc = Decimal::ZERO;
    for alloc in allocations {
        if alloc.allocated_weight <= Decimal::ZERO {
            return Err(ApiError::BadRequest(format!(
                "车辆{} 分配重量必须大于 0",
                alloc.vehicle_id
            )));
        }
        if !seen.insert(alloc.vehicle_id) {
            return Err(ApiError::BadRequest(format!(
                "车辆{} 重复分配，同一车辆只能派一次",
                alloc.vehicle_id
            )));
        }
        // TOCTOU 复查（事务内）：service 层 check_vehicle_in_use 在事务外，
        // 并发派车同车辆可能双派——事务内同款明细链复查
        // （conveyance→sales→deta.fk_deal→fk_list=运单；CSALE fk_previous=委托不含运单）
        let in_transit: bool = sqlx::query_scalar(
            r#"SELECT EXISTS (
                       SELECT 1 FROM "isahl"."zc_id_prod-traffic_rr_conveyance" c
                       JOIN "isahl"."zc_id_prod-freight_road-sales" p
                         ON p.id = c.ref_left AND p.deleted_at IS NULL
                       JOIN "isahl"."zc_id_deta-trade_order" d
                         ON d.fk_deal = p.id AND d.deleted_at IS NULL
                       JOIN "isahl"."zc_id_orde-land" w
                         ON w.id = d.fk_list AND w.deleted_at IS NULL
                       JOIN "isahl"."zc_id_lifecycle_r_primary-status" ls
                         ON ls.ref_left = w.id AND ls.deleted_at IS NULL
                       JOIN "isahl"."zc_id_stus-trade" s ON s.id = ls.ref_right
                       WHERE c.ref_right = $1 AND c.deleted_at IS NULL
                         AND s.code IN ('ST-DISPATCHED','ST-ACCIDENT'))"#,
        )
        .bind(alloc.vehicle_id)
        .fetch_one(&mut **tx)
        .await?;
        if in_transit {
            return Err(ApiError::BadRequest(format!(
                "车辆{} 已被分配（在途运单），无法重复派车",
                alloc.vehicle_id
            )));
        }
        // 注（2026-09-11 用户裁决「报量即容量」）：不再比较车辆自报载重
        // （zc_id_stor-ctn-vehicle.qk_w_capacity）——承运商申报的容量不作为拒绝依据。
        total_alloc += alloc.allocated_weight;
    }
    if let Some(cw) = consignment_weight {
        if cw > Decimal::ZERO && total_alloc > cw {
            return Err(ApiError::BadRequest(format!(
                "分配总重量{}吨超出委托整单重量{}吨",
                total_alloc, cw
            )));
        }
    }
    // ── Step 0e-1: reservation 断言（只读，批次间记账）──
    // Σ(既有批次 TSP-DSP OUT) + 本批 ≤ reservation 基数（下单扣减量或委托整单重量）；
    // 只读不打物化，不触碰守卫原语。两者皆无（无凭证且无明细重量）→ 无账可记，硬拒。
    let Some(reservation) = reservation else {
        return Err(ApiError::BadRequest(
            "委托无下单扣减证据且无整单重量（COM-OUT 凭证缺失且委托无明细），禁止派车以防超卖"
                .into(),
        ));
    };
    if total_alloc + dispatched_before > reservation {
        return Err(ApiError::BadRequest(format!(
            "派车分配超 reservation：已派 {}吨 + 本批 {}吨 > 基数 {}吨",
            dispatched_before, total_alloc, reservation
        )));
    }
    // 注：不再做容量池可用量校验（可售扣减已在下单完成，见 create_consignment_inner Step 5b-0）

    // ── Step 4-info: 委托单信息（批量共享，提出循环）──
    let consignment_info: (
        i64,
        Option<i64>,
        rust_decimal::Decimal,
        Option<rust_decimal::Decimal>,
        Option<i64>,
    ) = sqlx::query_as(
        r#"SELECT o.fk_subject, tl.fk_trustee, agg.agg_w_total, agg.agg_amount, o.qk_date
               FROM "isahl"."zc_id_orde-land" o
               LEFT JOIN LATERAL (
                   SELECT COALESCE(SUM(qt.mark::numeric), 0) AS agg_qty,
                          COALESCE(SUM(am.mark::numeric), 0) AS agg_amount,
                          COALESCE(SUM(wt.mark::numeric), 0) AS agg_w_total,
                          COALESCE(SUM(vt.mark::numeric), 0) AS agg_v_total,
                          COALESCE(SUM(at.mark::numeric), 0) AS agg_a_total,
                          COALESCE(SUM(ts.mark::numeric), 0) AS agg_ts_total,
                          COALESCE(SUM(dt.mark::numeric), 0) AS agg_d_total
                   FROM "isahl"."zc_id_deta-trade_order" d
                   LEFT JOIN "isahl"."zc_id_scale" qt ON qt.id = d.qk_qty
                   LEFT JOIN "isahl"."zc_id_scal-amount" am ON am.id = d.qk_amount
                   LEFT JOIN "isahl"."zc_id_scal-weight" wt ON wt.id = d.qk_w_qty
                   LEFT JOIN "isahl"."zc_id_scal-volume" vt ON vt.id = d.qk_v_qty
                   LEFT JOIN "isahl"."zc_id_scal-area" at ON at.id = d.qk_a_qty
                   LEFT JOIN "isahl"."zc_id_scal-distance" ts ON ts.id = d.qk_ts_qty
                   LEFT JOIN "isahl"."zc_id_scal-distance" dt ON dt.id = d.qk_d_qty
                   WHERE d.fk_list = o.id AND d.deleted_at IS NULL
                     AND (d.code NOT LIKE 'DTL-LDG-%'
                          OR NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_deta-trade_order" d2
                                         WHERE d2.fk_list = o.id AND d2.code NOT LIKE 'DTL-LDG-%'
                                           AND d2.deleted_at IS NULL))
               ) agg ON true
               LEFT JOIN "isahl"."zc_id_stor-traffic_line" tl ON tl.id = $2
               WHERE o.id = $1 AND o.deleted_at IS NULL"#,
    )
    .bind(consignment_id)
    .bind(traffic_line_id.unwrap_or(0))
    .fetch_one(&mut **tx)
    .await?;

    let mut results = Vec::with_capacity(allocations.len());

    // 双层双方（D1）：运单 fk_subject=B（运营主体）、fk_object=C（承运商）。
    // B 语义：运单 B 与委托层 B 一致——取委托 PRD 销售实例 fk_subj-provider。
    // 派车发起方有两类：① 平台运营用户（绑定运营组织，保留门禁）；② 承运商自有系统
    // （开放 API 服务令牌，服务主体不承载运营组织，无绑定可解析）——后者 B 只能由委托派生。
    let caller_user_type: Option<String> =
        sqlx::query_scalar(r#"SELECT user_type FROM isahl_auth.auth_users WHERE id = $1"#)
            .bind(user_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten();
    let operator_org = if caller_user_type.as_deref() == Some("service") {
        match resolve_consignment_operator_org(&mut *tx, consignment_id).await? {
            Some(org) => org,
            // 委托层无 B 证据（老数据）——回落系统运营主体（SUBJ-SYSTEM），仍拒绝无运营主体派车
            None => resolve_operator_org(tx, common::SYSTEM_USER_ID).await?,
        }
    } else {
        // 门禁：平台用户未绑定运营组织 → OPERATOR_ORG_UNBOUND
        resolve_operator_org(tx, user_id).await?
    };

    // 委托 PRD 销售实例（起讫地 rr_stop 场所复用来源；历史数据缺失时跳过 rr_stop 复制）
    let consign_sales_id: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT fp.id
               FROM "isahl"."zc_id_prod-freight_road-sales" fp
               WHERE fp.fk_previous = $1 AND fp.deleted_at IS NULL AND fp.code LIKE 'PRD-%'
               LIMIT 1"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten();

    // 缺口修复：委托产品 segm-date（预计时间继承给运单产品）+ 委托金额/货量（运单金额分摊）
    let consign_seg_id: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT fp.qk_period
               FROM "isahl"."zc_id_prod-freight_road-sales" fp
               WHERE fp.fk_previous = $1 AND fp.deleted_at IS NULL AND fp.code LIKE 'PRD-%'
               LIMIT 1"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten();
    let consign_money: Option<rust_decimal::Decimal> = sqlx::query_scalar(
        r#"SELECT agg.agg_amount FROM "isahl"."zc_id_orde-land" o
               LEFT JOIN LATERAL (
                   SELECT COALESCE(SUM(qt.mark::numeric), 0) AS agg_qty,
                          COALESCE(SUM(am.mark::numeric), 0) AS agg_amount,
                          COALESCE(SUM(wt.mark::numeric), 0) AS agg_w_total,
                          COALESCE(SUM(vt.mark::numeric), 0) AS agg_v_total,
                          COALESCE(SUM(at.mark::numeric), 0) AS agg_a_total,
                          COALESCE(SUM(ts.mark::numeric), 0) AS agg_ts_total,
                          COALESCE(SUM(dt.mark::numeric), 0) AS agg_d_total
                   FROM "isahl"."zc_id_deta-trade_order" d
                   LEFT JOIN "isahl"."zc_id_scale" qt ON qt.id = d.qk_qty
                   LEFT JOIN "isahl"."zc_id_scal-amount" am ON am.id = d.qk_amount
                   LEFT JOIN "isahl"."zc_id_scal-weight" wt ON wt.id = d.qk_w_qty
                   LEFT JOIN "isahl"."zc_id_scal-volume" vt ON vt.id = d.qk_v_qty
                   LEFT JOIN "isahl"."zc_id_scal-area" at ON at.id = d.qk_a_qty
                   LEFT JOIN "isahl"."zc_id_scal-distance" ts ON ts.id = d.qk_ts_qty
                   LEFT JOIN "isahl"."zc_id_scal-distance" dt ON dt.id = d.qk_d_qty
                   WHERE d.fk_list = o.id AND d.deleted_at IS NULL
                     AND (d.code NOT LIKE 'DTL-LDG-%'
                          OR NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_deta-trade_order" d2
                                         WHERE d2.fk_list = o.id AND d2.code NOT LIKE 'DTL-LDG-%'
                                           AND d2.deleted_at IS NULL))
               ) agg ON true
               WHERE o.id = $1 AND o.deleted_at IS NULL"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten();
    let consign_ton: Option<rust_decimal::Decimal> = sqlx::query_scalar(
        r#"SELECT agg.agg_w_total FROM "isahl"."zc_id_orde-land" o
               LEFT JOIN LATERAL (
                   SELECT COALESCE(SUM(qt.mark::numeric), 0) AS agg_qty,
                          COALESCE(SUM(am.mark::numeric), 0) AS agg_amount,
                          COALESCE(SUM(wt.mark::numeric), 0) AS agg_w_total,
                          COALESCE(SUM(vt.mark::numeric), 0) AS agg_v_total,
                          COALESCE(SUM(at.mark::numeric), 0) AS agg_a_total,
                          COALESCE(SUM(ts.mark::numeric), 0) AS agg_ts_total,
                          COALESCE(SUM(dt.mark::numeric), 0) AS agg_d_total
                   FROM "isahl"."zc_id_deta-trade_order" d
                   LEFT JOIN "isahl"."zc_id_scale" qt ON qt.id = d.qk_qty
                   LEFT JOIN "isahl"."zc_id_scal-amount" am ON am.id = d.qk_amount
                   LEFT JOIN "isahl"."zc_id_scal-weight" wt ON wt.id = d.qk_w_qty
                   LEFT JOIN "isahl"."zc_id_scal-volume" vt ON vt.id = d.qk_v_qty
                   LEFT JOIN "isahl"."zc_id_scal-area" at ON at.id = d.qk_a_qty
                   LEFT JOIN "isahl"."zc_id_scal-distance" ts ON ts.id = d.qk_ts_qty
                   LEFT JOIN "isahl"."zc_id_scal-distance" dt ON dt.id = d.qk_d_qty
                   WHERE d.fk_list = o.id AND d.deleted_at IS NULL
                     AND (d.code NOT LIKE 'DTL-LDG-%'
                          OR NOT EXISTS (SELECT 1 FROM "isahl"."zc_id_deta-trade_order" d2
                                         WHERE d2.fk_list = o.id AND d2.code NOT LIKE 'DTL-LDG-%'
                                           AND d2.deleted_at IS NULL))
               ) agg ON true
               WHERE o.id = $1 AND o.deleted_at IS NULL"#,
    )
    .bind(consignment_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten();
    // 缺口修复：派车运单明细货描 = 委托 comments（fallback notice）
    // remove-comments-json-embedding 降级（D2）：委托 comments 已文本化，直接读整列
    let consign_cargo: String = sqlx::query_scalar(
        r#"SELECT COALESCE(o.comments, o.notice)
               FROM "isahl"."zc_id_orde-land" o
               WHERE o.id = $1 AND o.deleted_at IS NULL"#,
    )
    .bind(consignment_id)
    .fetch_one(&mut **tx)
    .await
    .unwrap_or_default();

    // 本体坐标解析（BACKEND_FRAMEWORK §7.3.3：API 静态绑定——每个 API 内 dk 三元组固定）
    let (dk_scene_id, dk_factor_id, dk_function_id) =
        resolve_ontology_coords(tx, DkEntity::TspInstanceLeg).await?;
    let (dispatch_form, dispatch_tier) = DkEntity::TspInstanceLeg.form_type();
    // 叶表坐标（§6.12 声明即必须，循环外解析一次）：运单 orde-land = TX/FJA/↓_EV；
    // 明细 deta-trade_order / 追踪 oper-transport_tracking / 追踪事件 even-tracking = TX/FJA/↓_GG
    let (waybill_scene, waybill_factor, waybill_function) =
        ontology_binding::resolve_conn(&mut **tx, ("TX", "FJA", "↓_EV")).await?;
    let (trade_scene, trade_factor, trade_function) =
        ontology_binding::resolve_conn(&mut **tx, ("TX", "FJA", "↓_GG")).await?;
    // 叶表坐标（§6.12 声明即必须，循环外解析一次）：装载服务叶表 prod-loading = GC/FJA/↓_GG
    let (loading_scene, loading_factor, loading_function) =
        ontology_binding::resolve_conn(&mut **tx, ("GC", "FJA", "↓_GG")).await?;

    // PUR（B 的采购实例）在循环后按委托创建一次；多承运商派车时取首个非空承运商为供应商
    // （多 C 混合派车为边缘场景，code 唯一性按委托保证）
    let mut pur_carrier: Option<i64> = None;
    for alloc in allocations {
        let vehicle_id = alloc.vehicle_id;
        let weight = alloc.allocated_weight;

        // ── Step 0f: 每车独立重量标量（qk_w_qty 引用目标，scal-weight）──
        let weight_scale_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_scal-weight" (id, code, notice, mark, created_by_id)
                   VALUES (isahl.gen_next_uid(), $1, $2, $3, 1)
                   RETURNING id"#,
        )
        .bind(format!("WT-{}-{}", consign_code, vehicle_id))
        .bind(format!("{}吨", weight))
        .bind(weight)
        .fetch_one(&mut **tx)
        .await?;

        // ── Step 0f-2: 数量标量（qk_qty → scal-common；qk_w_qty 保持 scal-weight）──
        // meta_fields 权威：deta-trade_order.qty → zc_id_scal-common
        let qty_scale_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_scal-common" (id, code, notice, mark, created_by_id)
                   VALUES (isahl.gen_next_uid(), $1, $2, $3, 1)
                   RETURNING id"#,
        )
        .bind(format!("QTY-{}-{}", consign_code, vehicle_id))
        .bind(format!("{}吨", weight))
        .bind(weight)
        .fetch_one(&mut **tx)
        .await?;

        // ── Step 1: 承运商解析 + C 的销售实例（{运单code}-CSALE）──
        // C 优先级（方案 A，change: add-dispatch-carrier-selection）：
        //   ① 派车请求 carrier_id（显式选择，业务动作点）
        //   ② 容量池产品 fk_subj-provider（报价/签约承运商）
        //   ③ 车辆 fk_trustee 优先、线路 fk_trustee 回退（既有语义兜底）
        let pool_provider: Option<i64> = match capacity_product_id {
            Some(pid) => sqlx::query_scalar::<_, Option<i64>>(
                r#"SELECT "fk_subj-provider"
                       FROM "isahl"."zc_id_prod-freight_road-purchase"
                       WHERE id = $1 AND deleted_at IS NULL AND "fk_subj-provider" IS NOT NULL"#,
            )
            .bind(pid)
            .fetch_optional(&mut **tx)
            .await?
            .flatten(),
            None => None,
        };
        let fallback: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
            r#"SELECT COALESCE(v.fk_trustee, tl.fk_trustee)
                   FROM "isahl"."zc_id_stor-ctn-vehicle" v
                   LEFT JOIN "isahl"."zc_id_stor-traffic_line" tl ON tl.id = $2
                   WHERE v.id = $1 AND v.deleted_at IS NULL"#,
        )
        .bind(vehicle_id)
        .bind(traffic_line_id.unwrap_or(0))
        .fetch_optional(&mut **tx)
        .await?
        .flatten();
        // C 优先级：派车请求 carrier_id > 容量池产品 fk_subj-provider > 车辆/线路 fk_trustee
        let carrier: Option<i64> = carrier_id.or(pool_provider).or(fallback);
        if pur_carrier.is_none() {
            pur_carrier = carrier;
        }

        // 批注（用户指示）：运单号改 WB-{时间}-{车辆ZUID后6位}（原 WB-{委托号}-{车辆ZUID}
        // 过长）——时间=派车创建时刻（+08，yyyyMMddHHmmss），车辆取 ZUID 后 6 位（字符串截取保前导 0）
        let vid_suffix = vehicle_id.to_string();
        let vid_suffix = &vid_suffix[vid_suffix.len().saturating_sub(6)..];
        let waybill_code = format!(
            "WB-{}-{}",
            chrono::Utc::now()
                .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).expect("+08"))
                .format("%Y%m%d%H%M%S"),
            vid_suffix
        );
        // CSALE：C 对 B 的销售实例（运单明细 fk_deal 目标；conveyance 照旧挂此产品；
        // comments 携带 traffic_line_id（补充契约：读径经 comments::json 解析线路））
        // 单价标量（G3）：成交单价 = 分摊金额/本车重量（与 wb_unit_price 同源，先建标量供 qk_price 引用）
        let csale_price_id: Option<i64> = match (consign_money, consign_ton) {
            (Some(money), Some(ton)) if ton > rust_decimal::Decimal::ZERO => {
                let unit_price = (money / ton).round_dp(2);
                Some(
                        sqlx::query_scalar(
                            r#"INSERT INTO "isahl"."zc_id_scal-price" (id, code, notice, mark, created_by_id)
                               VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1)
                               RETURNING id"#,
                        )
                        .bind(format!("PRC-CSALE-{}-{}", consign_code, vehicle_id))
                        .bind(format!("运单{} 成交单价", waybill_code))
                        .bind(unit_price)
                        .fetch_one(&mut **tx)
                        .await?,
                    )
            }
            _ => None,
        };
        let row: (i64,) = sqlx::query_as(
                r#"INSERT INTO "isahl"."zc_id_prod-freight_road-sales"
                   (id, code, notice, comments, fk_previous, "fk_subj-demand", "fk_subj-provider",
                    "fk_line",
                    dk_scene, dk_factor, dk_function, qk_period, qk_price, "_f_", "_t_", created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2,
                           $3::text,
                           $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, 1)
                   RETURNING id"#,
            )
            .bind(format!("{}-CSALE", waybill_code))
            .bind("承运商销售实例")
            // remove-comments-json-embedding 降级（D1）：comments 仅人类可读摘要
            .bind(format!(
                "派车销售实例：委托 {} 线路 {}",
                consignment_id,
                traffic_line_id.unwrap_or(0)
            ))
            .bind(consignment_id)
            .bind(operator_org)
            .bind(carrier)
            // remove-comments-json-embedding 修复：release_waybill_transit 降级后经
            // 运单明细 fk_deal→CSALE 产品 fk_line 解析线路/容量池——CSALE 必须落真实列 fk_line，
            // 否则取消回补静默空转（此前 fk_line 恒 NULL）
            .bind(traffic_line_id)
            .bind(dk_scene_id)
            .bind(dk_factor_id)
            .bind(dk_function_id)
            // 缺口修复：运单产品继承委托 segm-date（预计出发/到达时间）
            .bind(consign_seg_id)
            .bind(csale_price_id)
            .bind(dispatch_form).bind(dispatch_tier)
            .fetch_one(&mut **tx)
            .await?;
        let product_id = row.0;

        // ── Step 1a: CSALE 起讫地桥接（rr_stop×2，复用委托 PRD 同款场所）──
        // 补充契约：运单总览/详情读径按 fk_deal 单列解析起讫地，缺桥则丢值
        if let Some(prd_id) = consign_sales_id {
            for (code_suffix, cat_code) in [("D", "ST-DEPART"), ("A", "ST-ARRIVE")] {
                sqlx::query(
                    r#"INSERT INTO "isahl"."zc_id_prod-transport_rr_stop"
                           (id, code, ref_left, ref_right, ck_category, created_by_id)
                           SELECT isahl.gen_next_zuid(), $1, $2, s.ref_right, s.ck_category, 1
                           FROM "isahl"."zc_id_prod-transport_rr_stop" s
                           JOIN "isahl"."zc_id_cate-traffic" ct ON ct.id = s.ck_category
                           WHERE s.ref_left = $3 AND s.deleted_at IS NULL AND ct.code = $4
                           LIMIT 1"#,
                )
                .bind(format!("STOP-{}-{}", waybill_code, code_suffix))
                .bind(product_id)
                .bind(prd_id)
                .bind(cat_code)
                .execute(&mut **tx)
                .await?;
            }
        }

        // ── Step 1b: B 对 C 的 request 实例（{运单code}-REQ，运单明细 fk_demand）──
        let waybill_req_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_prod-freight_road-request"
                   (id, code, notice, comments, fk_previous, "fk_subj-demand", "fk_subj-provider",
                    dk_scene, dk_factor, dk_function, "_f_", "_t_", created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2,
                           $3::text,
                           $4, $5, $6, $7, $8, $9, $10, $11, 1)
                   RETURNING id"#,
        )
        .bind(format!("{}-REQ", waybill_code))
        .bind(format!("{} 对承运商诉求", waybill_code))
        // remove-comments-json-embedding 降级（D1）：comments 仅人类可读摘要
        .bind(format!(
            "派车诉求：委托 {} 线路 {}",
            consignment_id,
            traffic_line_id.unwrap_or(0)
        ))
        .bind(consignment_id)
        .bind(operator_org)
        .bind(carrier)
        .bind(dk_scene_id)
        .bind(dk_factor_id)
        .bind(dk_function_id)
        .bind(dispatch_form)
        .bind(dispatch_tier)
        .fetch_one(&mut **tx)
        .await?;

        // ── Step 1c: 承运商制造实例（DSP-*，运单明细 fk_delivery；改插 made 叶表）──
        let dsp_made_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_prod-freight_road-made"
                   (id, code, notice, comments, fk_previous, "fk_subj-demand", "fk_subj-provider",
                    dk_scene, dk_factor, dk_function, "_f_", "_t_", created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, '承运商履约实例', $3, $4, $5,
                           $6, $7, $8, $9, $10, 1)
                   RETURNING id"#,
        )
        .bind(format!("DSP-{}-{}", consign_code, vehicle_id))
        .bind(format!("公路运输履约 车辆{}", vehicle_id))
        .bind(consignment_id)
        .bind(operator_org)
        .bind(carrier)
        .bind(dk_scene_id)
        .bind(dk_factor_id)
        .bind(dk_function_id)
        .bind(dispatch_form)
        .bind(dispatch_tier)
        .fetch_one(&mut **tx)
        .await?;

        // （fix-wz-procure-contract-structure：selected 询盘 → 采购合同 matter.qk_price）。
        let mut pur_contract_id: Option<i64> = None;
        let pur_price_id: Option<i64> = if let Some(pp) = purchase_price.filter(|v| *v > 0.0) {
            Some(
                    sqlx::query_scalar(
                        r#"INSERT INTO "isahl"."zc_id_scal-price" (id, code, notice, mark, created_by_id)
                           VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1)
                           RETURNING id"#,
                    )
                    .bind(format!("PRC-PUR-{}-{}", consign_code, vehicle_id))
                    .bind(format!("运单{} 采购单价", waybill_code))
                    .bind(pp)
                    .fetch_one(&mut **tx)
                    .await?,
                )
        } else if let (Some(carrier_id), Some(line_id)) = (carrier, traffic_line_id) {
            // 招商价兜底：selected 询盘合同 matter 中标价
            match find_selected_contract_price(&mut **tx, carrier_id, line_id).await? {
                Some((contract_id, price_scalar_id)) => {
                    pur_contract_id = Some(contract_id);
                    Some(price_scalar_id)
                }
                None => {
                    log::warn!(
                        "运单{} 派车 (承运商{}, 线路{}) 无匹配招商合同——PUR 无价",
                        waybill_code,
                        carrier_id,
                        line_id
                    );
                    None
                }
            }
        } else {
            // C4：无线路上下文不做招商匹配（原 unwrap_or(0) 永不命中且静默）——显式 warn
            log::warn!(
                "运单{} 派车无线路上下文，跳过招商价兜底（PUR 无价）",
                waybill_code
            );
            None
        };
        let waybill_pur_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_prod-freight_road-purchase"
                   (id, code, notice, comments, fk_previous, "fk_subj-demand", "fk_subj-provider",
                    dk_scene, dk_factor, dk_function, qk_price, "_f_", "_t_", created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, '承运商采购实例', $3, $4, $5,
                           $6, $7, $8, $9, $10, $11, 1)
                   RETURNING id"#,
        )
        .bind(format!("{}-PUR", waybill_code))
        .bind(format!("{} 承运商采购", waybill_code))
        .bind(consignment_id)
        .bind(operator_org)
        .bind(carrier)
        .bind(dk_scene_id)
        .bind(dk_factor_id)
        .bind(dk_function_id)
        .bind(pur_price_id)
        .bind(dispatch_form)
        .bind(dispatch_tier)
        .fetch_one(&mut **tx)
        .await?;

        // 运单级采购订单挂合同（fix-wz-procure-contract-structure：order_rr_contract 桥）
        if let Some(ct_id) = pur_contract_id {
            sqlx::query(
                // NOT EXISTS 幂等守卫——表上唯一索引是 (ref_left, ref_right, COALESCE(qk_period))
                // 表达式索引，ON CONFLICT (ref_left, ref_right) 无匹配唯一约束（运行时必报错）
                r#"INSERT INTO "isahl"."zc_id_order_rr_contract"
                       (id, code, notice, ref_left, ref_right, created_by_id)
                       SELECT isahl.gen_next_zuid(), $1, $2, $3, $4, 1
                       WHERE NOT EXISTS (
                         SELECT 1 FROM "isahl"."zc_id_order_rr_contract"
                         WHERE ref_left = $3 AND ref_right = $4 AND deleted_at IS NULL)"#,
            )
            .bind(format!("ORC-{}-{}", consign_code, vehicle_id))
            .bind(format!("运单{} 采购合同挂接", waybill_code))
            .bind(consignment_id)
            .bind(ct_id)
            .execute(&mut **tx)
            .await?;
        }

        // Step 2（运单明细）移至 Step 4 运单创建之后——fk_list 必须引用 wb_id（契约 #3）

        // Step 3: 创建运具关联
        let row: (i64,) = sqlx::query_as(
            r#"INSERT INTO "isahl"."zc_id_prod-traffic_rr_conveyance"
                   (id, code, ref_left, ref_right, comments)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, '派车: 车辆已分配')
                   RETURNING id"#,
        )
        .bind(format!("CNV-{}-{}", consign_code, vehicle_id))
        .bind(product_id)
        .bind(vehicle_id)
        .fetch_one(&mut **tx)
        .await?;
        let conveyance_id = row.0;

        // 缺口修复：运单金额按装载量比例分摊委托金额（多车不共享全额）
        let wb_amount_id: Option<i64> = match (consign_money, consign_ton) {
            (Some(money), Some(ton)) if ton > rust_decimal::Decimal::ZERO => {
                let share = (money * weight / ton).round_dp(2);
                Some(
                        sqlx::query_scalar(
                            r#"INSERT INTO "isahl"."zc_id_scal-amount" (id, code, notice, mark, created_by_id)
                               VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1)
                               RETURNING id"#,
                        )
                        .bind(format!("AMT-WB-{}-{}", consign_code, vehicle_id))
                        .bind(format!("运单{} 该车运费", waybill_code))
                        .bind(share)
                        .fetch_one(&mut **tx)
                        .await?,
                    )
            }
            _ => None,
        };

        // Step 4: 创建运单（fix-wz-hardcoded-category-codes T10：ck_category 非语义码，不再写入）
        // qk_w_total 引用本车装载量标量（修正历史列序错位：原实现 qk_w_total 误绑 qk_amount）
        let // remove-comments-json-embedding 降级（D1）：运单 comments 仅人类可读摘要，
            // 结构化关联（consignment_id/vehicle_id）不再嵌入（读径已同步降级）
            waybill_comments = format!(
                "运单：委托 {} 车辆 {} 产品 {} 线路 {} 装载 {} 吨",
                consignment_id,
                vehicle_id,
                product_id,
                traffic_line_id.unwrap_or(0),
                weight
            );
        // D1 双层双方：fk_subject=B（绑定运营组织，后端注入）、fk_object=C（车辆/线路承运商）
        let wb_id: i64 = sqlx::query_scalar(
                r#"INSERT INTO "isahl"."zc_id_orde-land"
                   (id, code, notice, comments, fk_subject, fk_object, qk_date, created_by_id, "_f_", "_t_",
                    dk_scene, dk_factor, dk_function, ak_permit_user, ak_access_user)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3::text, $4, $5, $6, $7, $8, $9, $10, $11, $12, ARRAY[$13]::bigint[], ARRAY[$13]::bigint[])
                   RETURNING id"#,
            )
            .bind(&waybill_code)
            .bind(format!(
                "运单 {} 车辆:{} 装载{}吨",
                consign_code, vehicle_id, weight
            ))
            .bind(waybill_comments.to_string())
            .bind(operator_org)
            .bind(carrier)
            // 聚合由明细求和（orde-* 聚合列已删）：运单不再写 qk_total/qk_amount
            // qk_date 继承委托的日期标量引用（F-A 修复：qk_* 为 bigint 引用，禁止绑 NOW()）
            .bind(consignment_info.4)
            .bind(1i64)
            .bind(dispatch_form)
            .bind(dispatch_tier)
            .bind(waybill_scene)
            .bind(waybill_factor)
            .bind(waybill_function)
            // 行级权属（D5）：创建运单的操作者 uid（平台/承运服务身份）
            .bind(user_id)
            .fetch_one(&mut **tx)
            .await?;

        // ── 总单守卫（拆单↔总单铁律，判别只用编码前缀；ck_category 已退役）──
        // zc_id_order_rr_demand 的 ref_right（总单）MUST 是委托（CNS-*）——
        // 若为运单（WB-*）说明调用方把「拆分单↔总单」写成了「运单↔运单」，
        // 会污染经该桥反查委托的读径（运单列表货物/账单客户口径），拒绝写入。
        let total_code: Option<String> = sqlx::query_scalar(
            r#"SELECT code FROM "isahl"."zc_id_orde-land"
                   WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(consignment_id)
        .fetch_optional(&mut **tx)
        .await?;
        match total_code.as_deref() {
                Some(code) if !code.starts_with("WB-") => {}
                Some(code) => {
                    return Err(ApiError::BadRequest(format!(
                        "总单 {consignment_id}（code={code}）为运单，拆分单总单 MUST 是委托（CNS-*）——拒绝写入 zc_id_order_rr_demand"
                    )))
                }
                None => {
                    return Err(ApiError::NotFound(format!(
                        "总单 {consignment_id} 不存在，无法建立拆分单关联"
                    )))
                }
            }

        // ── 运单→委托从属（A3，2026-08-24 用户定夺）──
        // 结构化桥 zc_id_order_rr_demand（「订单↔上游需求」：ref_left=运单/订单、
        // ref_right=委托/上游需求，正式模型表）；替代 fk_previous（版本链字段，
        // 语义错误）与 comments.consignment_id（降级后已断）。
        sqlx::query(
            r#"INSERT INTO "isahl".zc_id_order_rr_demand
                   (ref_left, ref_right, created_by_id, updated_by_id)
                   VALUES ($1, $2, $3, $3)"#,
        )
        .bind(wb_id)
        .bind(consignment_id)
        .bind(1i64)
        .execute(&mut **tx)
        .await?;

        // ── 一式两份镜像运单（矩阵 #6/#7，用户裁决 2026-09-11）──
        // 同表同类别、甲/乙主体互换（fk_subject=B↔fk_object=C）、code=`{WB}-R`；
        // 镜像行 + **叶子**桥 zc_id_lifecycle_rr_form 下沉 consignment-writer（单一写件，
        // `_f_`/`_t_` 由职能码派生）；镜像行内已补 ak_permit_user（读侧行级权属可见）。
        // 总单关联仍由主运单 order_rr_demand 承载。
        let _mirror_wb_id = insert_order_mirror_tx(
            &mut **tx,
            wb_id,
            &OrderMirrorInput {
                code: &format!("{}-R", waybill_code),
                notice: &format!(
                    "运单 {} 车辆:{} 装载{}吨（镜像）",
                    consign_code, vehicle_id, weight
                ),
                comments: &waybill_comments,
                subject: carrier,
                object: Some(operator_org),
                qk_date: consignment_info.4,
                fn_code: "↓_BE",
                kind_label: "运单",
                user_id: 1,
            },
        )
        .await?;
        // ── 单价标量（元/吨 = 委托均价；有分摊金额时写入，与 CSALE 成交单价同源）──
        let wb_unit_price_id: Option<i64> = match (wb_amount_id, weight) {
            (Some(_amount_id), w) if w > rust_decimal::Decimal::ZERO => {
                let unit_price = (consign_money.unwrap_or_default()
                    / consign_ton.unwrap_or_default())
                .round_dp(2);
                Some(
                        sqlx::query_scalar(
                            r#"INSERT INTO "isahl"."zc_id_scal-price" (id, code, notice, mark, created_by_id)
                               VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1)
                               RETURNING id"#,
                        )
                        .bind(format!("PRC-WB-{}-{}", consign_code, vehicle_id))
                        .bind(format!("运单{} 单价", waybill_code))
                        .bind(unit_price)
                        .fetch_one(&mut **tx)
                        .await?,
                    )
            }
            _ => None,
        };

        // ── Step 2（移置）: 创建 deta-trade_order（运单矩阵，契约 #3）——fk_list=wb_id ──
        // fk_goods=C 的 sales 范例（该线路 C 属主销售池 CAP-SALE，无则 NULL）、
        // fk_deal=CSALE 实例、fk_demand=B 对 C 的 REQ 实例、fk_delivery=DSP made 实例、
        // fk_purchase={waybill}-PUR（B 对 C 采购实例，审视 G1 接线）、fk_biller=C、fk_counterparty=B
        let carrier_pool: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
            r#"SELECT r.ref_left
                   FROM "isahl"."zc_id_file_rr_url" r
                   JOIN "isahl"."zc_id_prod-freight_road-sales" p ON p.id = r.ref_left
                   WHERE r.ref_right = $1 AND r.qk_p_capacity IS NOT NULL
                     AND r.deleted_at IS NULL AND p.deleted_at IS NULL
                     AND p."fk_subj-provider" = $2
                   ORDER BY CASE WHEN p.code LIKE 'CAP-SALE-%' THEN 0 ELSE 1 END
                   LIMIT 1"#,
        )
        .bind(traffic_line_id.unwrap_or(0))
        .bind(carrier)
        .fetch_optional(&mut **tx)
        .await?
        .flatten();
        sqlx::query(
                r#"INSERT INTO "isahl"."zc_id_deta-trade_order" (
                       id, code, notice, comments, qk_qty, qk_w_qty, qk_amount, qk_price, fk_list, fk_goods, fk_deal, fk_demand,
                       fk_delivery, fk_purchase, fk_biller, fk_counterparty, dk_scene, dk_factor, dk_function
                   )
                   VALUES (isahl.gen_next_zuid(), $1, '货运明细',
                           -- remove-comments-json-embedding 降级（D1）：明细 comments 直接存货描纯文本
                           $2::text, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)"#,
            )
            .bind(format!("DET-{}-{}", consign_code, vehicle_id))
            // 缺口修复：明细写货描（改派页/结算读源）+ qk_qty=本车装载量标量
            .bind(&consign_cargo)
            .bind(qty_scale_id)
            .bind(weight_scale_id)
            .bind(wb_amount_id)
            .bind(wb_unit_price_id)
            .bind(wb_id)
            .bind(carrier_pool)
            .bind(product_id)
            .bind(waybill_req_id)
            .bind(dsp_made_id)
            .bind(waybill_pur_id)
            .bind(carrier)
            .bind(operator_org)
            .bind(trade_scene)
            .bind(trade_factor)
            .bind(trade_function)
            .execute(&mut **tx)
            .await?;

        // 设置运单生命周期状态为 ST-DISPATCHED（使用既有 upsert 模式）——
        // N1：派车即已派车，运单状态直接落到 ST-DISPATCHED（与委托 lifecycle 同码，
        // 读侧结算/应收统一按 stus-trade 码集读取），不再停在 ST-ACCEPTED。
        sqlx::query(
                r#"WITH upsert AS (
                    INSERT INTO "isahl"."zc_id_lifecycle_r_primary-status" (ref_left, ref_right, id, code)
                    SELECT $1, s.id, isahl.gen_next_uid(686), $2
                    FROM (SELECT id FROM "isahl"."zc_id_stus-trade" WHERE code = 'ST-DISPATCHED' ORDER BY id LIMIT 1) s
                    WHERE NOT EXISTS (
                        SELECT 1 FROM "isahl"."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1
                    )
                )
                UPDATE "isahl"."zc_id_lifecycle_r_primary-status"
                SET ref_right = (SELECT id FROM "isahl"."zc_id_stus-trade" WHERE code = 'ST-DISPATCHED' ORDER BY id LIMIT 1),
                    status_date = NOW()
                WHERE ref_left = $1"#,
            )
            .bind(wb_id)
            .bind(format!("LS-WB-{}-{}", consign_code, vehicle_id))
            .execute(&mut **tx)
            .await?;

        // 镜像运单状态桥同步（矩阵「镜像行与主行同等待遇：状态桥」）：主运单落
        // ST-DISPATCHED 后，镜像 `-R` 行同码同位（幂等，ref_left 唯一约束原地更新）。
        // 缺此步则一切按 lifecycle 状态过滤的读径对镜像行走空，而镜像行本应与主行并见。
        sync_mirror_status_tx(&mut **tx, wb_id, 1).await?;

        // ── Step 5b: 派车转换 tsp 凭证（设计契约 §2.4/D8，T7b：dispatch_deduction comments-JSON 退役）──
        // 制造范例在途 → 制造实例在途（源池 STO-TRANSIT qk_outgo / 目标池 STO-TRANSIT qk_income）
        let ts = format!("TSP-DSP-{}-{}", consign_code, vehicle_id);
        for (dir, qk_col, family, tier) in [
            ("OUT", "qk_outgo", "made", "template"),
            ("IN", "qk_income", "made", "instance"),
        ] {
            // 坐标按腿解析：范例腿 ↓.BE、实例腿 ↓_BE（_f_/_t_ 派生，禁止字面量直写）
            let leg_entity = if tier == "template" {
                DkEntity::TspTemplateLeg
            } else {
                DkEntity::TspInstanceLeg
            };
            let (leg_scene, leg_factor, leg_function) =
                resolve_ontology_coords(&mut **tx, leg_entity).await?;
            let (_, leg_tier) = leg_entity.form_type();
            // remove-comments-json-embedding 降级（D1）：凭证 comments 仅人类可读摘要
            let comments = format!(
                "派车转换 委托{} 车辆{} 方向{} 族{} 层{}",
                consignment_id, vehicle_id, dir, family, tier
            );
            let sql = if qk_col == "qk_outgo" {
                r#"INSERT INTO "isahl"."zc_id_stat-tsp-voucher"
                       (id, code, notice, comments, fk_production, "fk_subj-storage", "fk_obj-storage",
                        qk_outgo, "ck_sto-title", dk_scene, dk_factor, dk_function, _t_, created_by_id)
                       VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $5, $6,
                               (SELECT id FROM "isahl"."zc_id_cate-sto-title" WHERE code = 'STO-TRANSIT' LIMIT 1),
                               $7, $8, $9, $10, 1)"#
            } else {
                r#"INSERT INTO "isahl"."zc_id_stat-tsp-voucher"
                       (id, code, notice, comments, fk_production, "fk_subj-storage", "fk_obj-storage",
                        qk_income, "ck_sto-title", dk_scene, dk_factor, dk_function, _t_, created_by_id)
                       VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $5, $6,
                               (SELECT id FROM "isahl"."zc_id_cate-sto-title" WHERE code = 'STO-TRANSIT' LIMIT 1),
                               $7, $8, $9, $10, 1)"#
            };
            sqlx::query(sql)
                .bind(format!("{}-{}", ts, dir))
                .bind(format!(
                    "派车转换 {} 车辆{} {}吨",
                    consign_code, vehicle_id, weight
                ))
                .bind(&comments)
                .bind(effective_capacity_product_id)
                .bind(traffic_line_id)
                .bind(weight_scale_id)
                .bind(leg_scene)
                .bind(leg_factor)
                .bind(leg_function)
                .bind(leg_tier)
                .execute(&mut **tx)
                .await?;
        }

        // ── Step 5c: 记录履约库存实例 (production_rr_storage)，qk_qty 引用本车装载量标量 ──
        // comments.type='instance' 区分派车实例与履约模板（模板无 type 或非 instance）
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_file_rr_url"
                   (id, code, notice, comments, ref_left, ref_right, qk_qty, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, 1)"#,
        )
        .bind(format!("FULFILL-INST-{}-{}", consign_code, vehicle_id))
        .bind(format!("履约出库 {}吨", weight))
        // remove-comments-json-embedding 降级（D1）：comments 仅人类可读摘要
        .bind(format!("履约实例 车辆{} 装载{}吨", vehicle_id, weight))
        .bind(product_id)
        .bind(traffic_line_id)
        .bind(weight_scale_id)
        .execute(&mut **tx)
        .await?;

        // 注：扣减不再 UPDATE 容量标量 mark——新模型下总容量（qk_p_capacity → scal-common.mark）
        // 保持静态，已用 = stat-sto-voucher 扣减凭证 comments 聚合（凭证即扣减语义）

        // fk_operator = 司机（fix-wz-tms-data-linkage：派车显式指派 alloc.driver_id，
        // 校验 empl-natural 存活行后落库；不指派保持 NULL——与既有契约一致）。
        // E2E 实测：派车不建 tracking 行 → 追踪页无此运单 → 打卡/签收链路断。
        if let Some(driver_id) = alloc.driver_id {
            let driver_exists: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                           SELECT 1 FROM "isahl"."zc_id_empl-natural"
                           WHERE id = $1 AND deleted_at IS NULL)"#,
            )
            .bind(driver_id)
            .fetch_one(&mut **tx)
            .await?;
            if !driver_exists {
                return Err(ApiError::BadRequest(format!(
                    "派车失败：司机不存在（id={}，车辆 {}）",
                    driver_id, vehicle_id
                )));
            }
        }
        let tracking_id: i64 = sqlx::query_scalar(
                r#"INSERT INTO "isahl"."zc_id_oper-transport_tracking"
                   (id, code, notice, comments, fk_operator, fk_previous, created_by_id, updated_by_id,
                    dk_scene, dk_factor, dk_function)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3,
                           $4,
                           $5, 1, 1, $6, $7, $8)
                   RETURNING id"#,
            )
            .bind(waybill_code.clone())
            // 批注轮 65（53553d18）：追踪显示车牌（非车辆 id）——查 notice
            .bind(format!(
                "{} 在途追踪 车辆:{}",
                waybill_code,
                dispatch_vehicle_plate(&mut *tx, vehicle_id).await?
            ))
            // remove-comments-json-embedding 降级（D1）：comments 仅人类可读摘要
            .bind(format!(
                "在途追踪 车辆{} 运单{} 装载{}吨",
                dispatch_vehicle_plate(&mut *tx, vehicle_id).await?,
                wb_id, weight
            ))
            .bind(alloc.driver_id)
            .bind(consignment_id)
            .bind(trade_scene)
            .bind(trade_factor)
            .bind(trade_function)
            .fetch_one(&mut **tx)
            .await?;

        // 缺口修复：追踪链（even-tracking → event_rr_matter → prod-loading → 运单）
        // 追踪页/改派查询经此链反查委托/车辆——原实现只建 tracking 行，真实派车追踪断链。
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_prod-loading-request"
                   (id, code, notice, fk_previous, created_by_id, dk_scene, dk_factor, dk_function)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1, $4, $5, $6)"#,
        )
        .bind(format!("LDG-{}", waybill_code))
        .bind(format!("{} 装载服务", waybill_code))
        .bind(wb_id)
        .bind(loading_scene)
        .bind(loading_factor)
        .bind(loading_function)
        .execute(&mut **tx)
        .await?;
        let loading_id: i64 = sqlx::query_scalar(
            r#"SELECT id FROM "isahl"."zc_id_prod-loading"
                   WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(format!("LDG-{}", waybill_code))
        .fetch_one(&mut **tx)
        .await?;
        let event_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO "isahl"."zc_id_even-tracking"
                   (id, code, notice, fk_subject, created_by_id, dk_scene, dk_factor, dk_function)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1, $4, $5, $6)
                   RETURNING id"#,
        )
        .bind(format!("EV-{}", waybill_code))
        .bind(format!("{} 派车出发", waybill_code))
        .bind(tracking_id)
        .bind(trade_scene)
        .bind(trade_factor)
        .bind(trade_function)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_event_rr_matter"
                   (id, ref_left, ref_right, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, 1)"#,
        )
        .bind(event_id)
        .bind(loading_id)
        .execute(&mut **tx)
        .await?;

        // 批注 8adc86d4 的演示凭证预建已删除（2026-08-27 审查）——派车即预建 6 类
        // PROOF file-document 是 mock 数据（[NEVER] 铁律），且直接骗过三单/凭证链门禁
        // （proof-status 按实文件计数、T15 三单校验、A2 签收拦截全部失真）。
        // 真实凭证经 POST /waybills/{id}/proofs 上报落链（upload_proof）。

        results.push((vehicle_id, product_id, conveyance_id));
    }

    // ── Step 6: 物化 B 的 purchase 实例（{委托code}-PUR）+ prod-made_rr_prod-purchase 桥 ──
    // B 向 C 采购（fk_subj-demand=B/fk_subj-provider=C，契约 #4/#7）；
    // 桥 ref_left=受理时 B 的 made 实例（{委托code}-MADE）/ ref_right=PUR（契约 #5）；
    // 委托明细回填 fk_purchase（主运输明细行 DTL-{code}，审视 G2 实际落地——此前仅有注释无 UPDATE）。
    // fix-tms-consign-chain-data-integrity D1：委托级 PUR 幂等——分批派车（尤其多承运商）
    // 不得重复创建实例/覆盖委托明细采购引用；存在即复用，价格标量仅首批创建。
    // 委托级 PUR 幂等：存在即复用（分批派车不重复创建/不覆盖引用）
    let existing_pur: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM "isahl"."zc_id_prod-freight_road-purchase"
               WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(format!("{}-PUR", consign_code))
    .fetch_optional(&mut **tx)
    .await?;
    // 委托级招商合同 id（首批创建时匹配；挂 order_rr_contract 桥）
    let mut consign_contract_id: Option<i64> = None;
    let pur_id: i64 = match existing_pur {
        Some(id) => id,
        None => {
            // 首批创建（含价格标量；缺省经招商合同 matter 带价）
            let price_id: Option<i64> = if let Some(pp) = purchase_price.filter(|v| *v > 0.0) {
                Some(
                        sqlx::query_scalar(
                            r#"INSERT INTO "isahl"."zc_id_scal-price" (id, code, notice, mark, created_by_id)
                               VALUES (isahl.gen_next_zuid(), $1, $2, $3, 1)
                               RETURNING id"#,
                        )
                        .bind(format!("PRC-PUR-{}", consign_code))
                        .bind(format!("{} 采购单价", consign_code))
                        .bind(pp)
                        .fetch_one(&mut **tx)
                        .await?,
                    )
            } else if let (Some(carrier_id), Some(line_id)) = (pur_carrier, traffic_line_id) {
                // 招商价兜底：selected 询盘合同 matter 中标价（fix-wz-procure-contract-structure）
                match find_selected_contract_price(&mut **tx, carrier_id, line_id).await? {
                    Some((contract_id, price_scalar_id)) => {
                        consign_contract_id = Some(contract_id);
                        Some(price_scalar_id)
                    }
                    None => {
                        log::warn!(
                            "委托{} 派车 (承运商{}, 线路{}) 无匹配招商合同——PUR 无价",
                            consign_code,
                            carrier_id,
                            line_id
                        );
                        None
                    }
                }
            } else {
                log::warn!(
                    "委托{} 派车无线路上下文，跳过招商价兜底（PUR 无价）",
                    consign_code
                );
                None
            };
            sqlx::query_scalar(
                    r#"INSERT INTO "isahl"."zc_id_prod-freight_road-purchase"
                       (id, code, notice, comments, fk_previous, "fk_subj-demand", "fk_subj-provider",
                        dk_scene, dk_factor, dk_function, qk_price, "_f_", "_t_", created_by_id)
                       VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 1)
                       RETURNING id"#,
                )
                .bind(format!("{}-PUR", consign_code))
                .bind(format!("{} 供应商采购", consign_code))
                // remove-comments-json-embedding 降级（D1）：comments 仅人类可读摘要
                .bind(format!(
                    "供应商采购 委托 {} 线路 {}",
                    consignment_id,
                    traffic_line_id.unwrap_or(0)
                ))
                .bind(consignment_id)
                .bind(operator_org)
                .bind(pur_carrier)
                .bind(dk_scene_id)
                .bind(dk_factor_id)
                .bind(dk_function_id)
                .bind(price_id)
                .bind(dispatch_form).bind(dispatch_tier)
                .fetch_one(&mut **tx)
                .await?
        }
    };

    // 委托级采购订单挂合同（fix-wz-procure-contract-structure：order_rr_contract 桥；
    // 幂等——分批派车复用已有合同桥）
    if let Some(ct_id) = consign_contract_id {
        sqlx::query(
            // NOT EXISTS 幂等守卫——同上：表达式唯一索引与 ON CONFLICT (ref_left, ref_right) 不匹配
            r#"INSERT INTO "isahl"."zc_id_order_rr_contract"
                   (id, code, notice, ref_left, ref_right, created_by_id)
                   SELECT isahl.gen_next_zuid(), $1, $2, $3, $4, 1
                   WHERE NOT EXISTS (
                     SELECT 1 FROM "isahl"."zc_id_order_rr_contract"
                     WHERE ref_left = $3 AND ref_right = $4 AND deleted_at IS NULL)"#,
        )
        .bind(format!("ORC-{}", consign_code))
        .bind(format!("{} 采购合同挂接", consign_code))
        .bind(consignment_id)
        .bind(ct_id)
        .execute(&mut **tx)
        .await?;
    }

    // ── Step 6a: 回填委托明细 fk_purchase（审视 G2——注释声称回填但此前无 UPDATE 落地）──
    // 委托矩阵主运输明细 DTL-{code}：B 对 C 的采购实例引用，应收/应付与价格读径由此可达。
    // fix-tms-consign-chain-data-integrity D1：仅空引用时回填（分批派车不得末批覆盖）
    sqlx::query(
        r#"UPDATE "isahl"."zc_id_deta-trade_order"
               SET fk_purchase = $2
               WHERE fk_list = $1 AND code = $3 AND fk_purchase IS NULL AND deleted_at IS NULL"#,
    )
    .bind(consignment_id)
    .bind(pur_id)
    .bind(format!("DTL-{}", consign_code))
    .execute(&mut **tx)
    .await?;

    // 桥（仅当受理已物化 B 的 made 实例时接线；旧数据缺 made 则跳过，PUR 本身仍落库）
    let consign_made_id: Option<i64> = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT id FROM "isahl"."zc_id_prod-freight_road-made"
               WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(format!("{}-MADE", consign_code))
    .fetch_optional(&mut **tx)
    .await?
    .flatten();
    if let Some(made_id) = consign_made_id {
        sqlx::query(
            // 幂等守卫用 NOT EXISTS（ref_left/ref_right 活行）——表上唯一索引是
            // (ref_left, ref_right, COALESCE(qk_period)) 表达式索引，ON CONFLICT (code)
            // 无匹配唯一约束，运行时必报「没有匹配ON CONFLICT说明的唯一或者排除约束」
            r#"INSERT INTO "isahl"."zc_id_prod-made_rr_prod-purchase"
                   (code, notice, ref_left, ref_right, created_by_id)
                   SELECT $1, $2, $3, $4, 1
                   WHERE NOT EXISTS (
                     SELECT 1 FROM "isahl"."zc_id_prod-made_rr_prod-purchase"
                     WHERE ref_left = $3 AND ref_right = $4 AND deleted_at IS NULL)"#,
        )
        .bind(format!("REL-MP-{}-{}", made_id, pur_id))
        .bind("制造→采购关联")
        .bind(made_id)
        .bind(pur_id)
        .execute(&mut **tx)
        .await?;
    }

    // ── N1：委托生命周期在事务内同步推进（与运单同码，全或无）──
    // 用户 2026-08-19 状态规则：派车后剩余货量>0 → ST-DISPATCHED（读侧委托单上下文映射为
    // 「部分分配」）；剩余=0 → ST-PENDING_PAYMENT（「待付款」）。剩余=委托总重−Σ历史已派车−Σ本次派车。
    // fix-dispatch-inventory-reconciliation 3.4：已派车量实算 =
    // 0e-0 批次前读数（Σ 既有 TSP-DSP OUT）+ 本批次分配量；
    // 剩余 = 委托总重 − 实算已派。ST-PENDING_PAYMENT 分支复活。
    let remaining: Option<rust_decimal::Decimal> =
        consign_ton.map(|ton| ton - dispatched_before - total_alloc);
    let target_status = match remaining {
        Some(r) if r <= rust_decimal::Decimal::ZERO => "ST-PENDING_PAYMENT",
        _ => "ST-DISPATCHED",
    };
    sqlx::query(
            r#"WITH upsert AS (
                INSERT INTO "isahl"."zc_id_lifecycle_r_primary-status" (ref_left, ref_right, id, code)
                SELECT $1, s.id, isahl.gen_next_uid(686), $2
                FROM (SELECT id FROM "isahl"."zc_id_stus-trade" WHERE code = $2 ORDER BY id LIMIT 1) s
                WHERE NOT EXISTS (
                    SELECT 1 FROM "isahl"."zc_id_lifecycle_r_primary-status" WHERE ref_left = $1
                )
            )
            UPDATE "isahl"."zc_id_lifecycle_r_primary-status"
            SET ref_right = (SELECT id FROM "isahl"."zc_id_stus-trade" WHERE code = $2 ORDER BY id LIMIT 1),
                status_date = NOW()
            WHERE ref_left = $1"#,
        )
        .bind(consignment_id)
        .bind(target_status)
        .execute(&mut **tx)
        .await?;

    // 镜像委托状态桥同步（矩阵「镜像行与主行同等待遇：状态桥」）：主委托推进后，
    // 其 `-R` 镜像行同码同位（幂等）；镜像行由 `consignment-writer` 建单时落桥。
    sync_mirror_status_tx(&mut **tx, consignment_id, user_id).await?;

    Ok(results)
}

/// 派车招商价查询：按 (承运商, 线路) 匹配 selected 询盘的采购合同中标价
/// （matter C-SALES → 报价行 → 询盘 request 行 → 状态桥 selected → 线路）
///
/// 返回 (contract_id, price_scalar_id)；无匹配 → None（DTO purchase_price 显式值优先于本兜底）。
/// （迁自 `repositories/procure.rs`；唯一调用方 = 派车核心事务。）
pub async fn find_selected_contract_price<'e, E>(
    executor: E,
    carrier_id: i64,
    line_id: i64,
) -> Result<Option<(i64, i64)>, ApiError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let row: Option<(i64, i64)> = sqlx::query_as(
        r#"SELECT m.ref_left, m.qk_price
           FROM "isahl"."zc_id_contract_rr_matter" m
           JOIN "isahl"."zc_id_prod-freight_road-sales" q ON q.id = m.ref_right
           JOIN "isahl"."zc_id_prod-freight_road-request" r ON q.code LIKE 'QT-' || r.id || '-%'
           JOIN "isahl"."zc_id_lifecycle_r_primary-status" lrs ON lrs.ref_left = r.id
           JOIN "isahl"."zc_id_stus-prod-request" st ON st.id = lrs.ref_right AND st.code = 'selected'
           WHERE m.code = 'C-SALES' AND m.deleted_at IS NULL
             AND q."fk_subj-provider" = $1 AND q.deleted_at IS NULL
             AND r.fk_line = $2 AND r.deleted_at IS NULL
             AND lrs.deleted_at IS NULL
           ORDER BY m.id DESC LIMIT 1"#,
    )
    .bind(carrier_id)
    .bind(line_id)
    .fetch_optional(executor)
    .await
    .map_err(ApiError::from_sqlx)?;
    Ok(row)
}
