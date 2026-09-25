//! # damage-writer — 损管/异常事件写链共享事务
//!
//! 单一事实源：`zc_id_even-accident`（损管/异常事件）主行 + `zc_id_event_rr_container`
//! （事件↔车辆容器桥）+ `zc_id_event_rr_matter`（事件↔交付事项桥）三张表的写事务抽取，
//! 供两类调用方共用（REUSE_FIRST，禁止第二套实现）：
//!
//! - **Gateway WZ**（logi-consignment `FjaRepository::create_damage`）：平台调度员创建损管；
//! - **OpenActivity 门户**（portal_write `insert_even_accident`）：承运异常上报 / 客户申报。
//!
//! ## 事件↔订单链（`wire-damage-event-order-chain`）
//!
//! 报损表单选定订单后，事件与订单 MUST 经**交付事项**相遇（禁借 `lk_risk`——模型声明 `lk_*` = 等级引用）：
//!
//! ```text
//! even-accident a ──(ref_left)──▶ zc_id_event_rr_matter ──(ref_right)──▶ zc_id_production 交付事项
//!                                                                              ▲
//!                                    zc_id_deta-trade_order d.fk_delivery ──────┘
//!                                    d.fk_list ──▶ 运单（zc_id_stat-trade_order 族）──▶ 委托
//! ```
//!
//! `resolve_delivery_matter_tx` 由订单 code 解析交付事项，`link_event_matter_tx` 落桥；
//! 解析不到（未受理 / 无主运输明细）⇒ 调用方 MUST 不落桥并如实降级（不阻断建单）。
//!
//! 两应用禁止 HTTP 互调（平台端点鉴权为 JWT + `require_resource_access`，服务身份化需
//! Gateway noauth 白名单 + 密钥补偿），故写事务以共享 crate 单份实现。
//!
//! ## 设计要点
//!
//! - **主体语义参数化**：`DamageWriteContext { fk_subject, actor_user_id }`——组织解析外置。
//!   平台内部写 `fk_subject = None`（读侧经 `lk_risk`/委托链取数）；门户 `Some(绑定 org)`
//!   （门户读侧谓词 `fk_subject = $org` 必需，平台内部写不落该列时门户读不到）。
//! - **事务语义**：`&mut PgConnection`（调用方管理 begin/commit/rollback），行为与迁移前逐列一致；
//!   标量先行（`zc_id_scal-date`）由调用方在事务内自行完成（门户专有）。
//! - **文案参数化**：桥行 `notice` 由调用方给定（平台「损管事件→车辆」/门户「异常事件→车辆」）。
//!
//! ## 禁用事项
//!
//! - `zc_id_even-accident` 为类契约表（`zc_id_lifecycle` 子表族，含 `_f_`/`_t_`）：
//!   两处调用方迁移前即未写类列，本 crate 保持「不写 `_f_`/`_t_`」的既有行为
//!   （归线键见 `scripts/check/.leaf-insert-baseline.txt`），不在本次收敛中扩散修改。

use sqlx::{PgConnection, Row};

/// 写上下文：主体语义参数化（组织解析外置——调用方注入）。
#[derive(Debug, Clone, Copy)]
pub struct DamageWriteContext {
    /// 事件主体（`fk_subject`）：门户 = 绑定组织（读侧谓词锚）；平台 = `None`（沿用不落列）
    pub fk_subject: Option<i64>,
    /// 写入人（`created_by_id` / `updated_by_id` 与桥行 `created_by_id`）
    pub actor_user_id: i64,
}

/// 事件主行输入（业务字段；主体/操作者见 [`DamageWriteContext`]）。
#[derive(Debug, Clone)]
pub struct EventAccidentRow<'a> {
    /// 事件编号（平台 `WZ-E2E-AC-{ts}` / 门户 `EXT-AC-{ts}`）
    pub code: &'a str,
    /// 事件类型/标题（`notice`；平台可空，门户必填）
    pub notice: Option<&'a str>,
    /// 纯文本摘要（`comments`；禁 JSON 嵌入）
    pub comments: &'a str,
    /// 严重等级引用（`zc_id_leve-severity` id；由调用方解析/校验）
    pub lk_severity: Option<i64>,
    /// 关联地点（`fk_place`）
    pub fk_place: Option<i64>,
    /// 关联委托明细（`lk_risk`；门户不写）
    pub lk_risk: Option<i64>,
    /// 发生日期标量引用（`qk_date`；标量行由调用方先行创建）
    pub qk_date: Option<i64>,
}

/// `INSERT zc_id_even-accident` 主行（事务由调用方持有）。返回新行 id。
pub async fn insert_event_accident_tx(
    conn: &mut PgConnection,
    ctx: &DamageWriteContext,
    row: &EventAccidentRow<'_>,
) -> Result<i64, sqlx::Error> {
    // 写件侧兜底校验（用户 2026-09-16 要求：空载荷不得建单）——平台与门户两处调用方共享本写件，
    // 在入口拒绝「无编号 / 无标题且无描述」的行，避免各字段皆空的垃圾事件行落库。
    if row.code.trim().is_empty()
        || (row.notice.map(str::trim).unwrap_or("").is_empty() && row.comments.trim().is_empty())
    {
        return Err(sqlx::Error::Protocol(
            "insert_event_accident_tx: code 与 (notice|comments) 均不得为空".into(),
        ));
    }
    // 叶表坐标（§6.12）：事故行 dk 经静态绑定解析（事务内 resolve_conn；坐标 JE/FRA/↓_EZ
    // 与迁移前平台内联实现一致，门户与平台两处调用方共享）。
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve_conn(conn, ("JE", "FRA", "↓_EZ")).await?;
    let inserted = sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_even-accident"
           (id, code, notice, comments, lk_severity, fk_place, lk_risk, qk_date,
            fk_subject, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $9, $10, $11, $12)
           RETURNING id"#,
    )
    .bind(row.code)
    .bind(row.notice)
    .bind(row.comments)
    .bind(row.lk_severity)
    .bind(row.fk_place)
    .bind(row.lk_risk)
    .bind(row.qk_date)
    .bind(ctx.fk_subject)
    .bind(ctx.actor_user_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&mut *conn)
    .await?;
    inserted.try_get("id")
}

/// 事件↔车辆桥（`zc_id_event_rr_container`）：车牌 → 车辆实体反查 → INSERT 桥
/// （`gen_next_uid(473)`、code `EVT-CNT-{id}-{vid}`）。
///
/// 反查口径（change `align-vehicle-plate-identity`，2026-09-23 模型裁定）：
/// 车牌号存于 `zc_id_identity.identity`（分类字典 `zc_id_cate-identity.code = 'plate'`），
/// 经实体↔身份桥 `zc_id_entity_rr_identity` 关联车辆（`ref_left` = 车辆）。
/// 车辆 `notice`（描述信息）/`code`（序列号）**不承载号牌** ⇒ 无桥行即无车（不臆造关联）。
///
/// 历史沿革：早期实现按 `stor-ctn-vehicle.notice = 车牌` 反查（当年车辆建档把车牌写 notice），
/// 另有一级「身份桥」兜底；本次模型对齐后**只保留身份桥这一级**（旧 notice 口径作废）。
///
/// 车牌为空（或无桥行命中）→ no-op（不臆造关联）。`notice` 语义由调用方给定。
pub async fn link_event_container_tx(
    conn: &mut PgConnection,
    ctx: &DamageWriteContext,
    accident_id: i64,
    plate: &str,
    notice: &str,
) -> Result<(), sqlx::Error> {
    let plate = plate.trim();
    if plate.is_empty() {
        return Ok(());
    }
    // 车牌 → 车辆：**唯一口径** = 实体↔身份桥（`zc_id_identity.identity` 命中且分类为 `plate`）。
    // 车辆 `notice`/`code` MAY NOT 承载号牌（change align-vehicle-plate-identity，2026-09-23 模型裁定；
    // 车辆 ⊂ `zc_id_carrier` ⊂ `zc_id_entity` ⇒ 桥为模型声明正用，见 model-center-requests.md §R12）。
    let vehicle_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT v.id FROM "isahl"."zc_id_stor-ctn-vehicle" v
           JOIN "isahl"."zc_id_entity_rr_identity" er
             ON er.ref_left = v.id AND er.deleted_at IS NULL
           JOIN "isahl"."zc_id_identity" i
             ON i.id = er.ref_right AND i.deleted_at IS NULL
           JOIN "isahl"."zc_id_cate-identity" c
             ON c.id = i.ck_category AND c.deleted_at IS NULL
          WHERE v.deleted_at IS NULL AND c.code = 'plate'
            AND upper(i.identity) = upper($1)
          ORDER BY er.id DESC LIMIT 1"#,
    )
    .bind(plate)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(vid) = vehicle_id {
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_event_rr_container"
               (id, code, notice, ref_left, ref_right, created_by_id)
               VALUES (isahl.gen_next_uid(473), $1, $2, $3, $4, $5)"#,
        )
        .bind(format!("EVT-CNT-{accident_id}-{vid}"))
        .bind(notice)
        .bind(accident_id)
        .bind(vid)
        .bind(ctx.actor_user_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// 交付事项解析——事件↔订单链的**订单侧锚点**（`wire-damage-event-order-chain` D2）。
///
/// 入参二选一（调用方各持其一）：`order_code`（平台报损表单的运单号 `WB-*` / 委托号 `CNS-*`）
/// 或 `order_id`（门户上报属权校验后的运单 id）。两者都是 `zc_id_stat-trade_order` 族行的定位键。
///
/// 口径：订单行 → 主运输明细（排除装载明细 `DTL-LDG-%`）→ `fk_delivery`（交付事项，`zc_id_production` 族行）。
/// 用**统一父表** `zc_id_stat-trade_order` 而非具体叶（`zc_id_orde-land`/`orde-traffic`…）：
/// 平台表单选自 `orde-land`、门户属权运单校验在 `orde-traffic`，父表两侧都覆盖。
///
/// 未受理（`fk_delivery IS NULL`——该列由 `dispatch_core::accept_consignment` 回填）/ 无主运输明细
/// / 入参全空 ⇒ `None`：调用方 MUST 据此**不落桥**并如实降级（`D3`，不阻断建单、不臆造关联）。
pub async fn resolve_delivery_matter_tx(
    conn: &mut PgConnection,
    order_code: Option<&str>,
    order_id: Option<i64>,
) -> Result<Option<i64>, sqlx::Error> {
    let code = order_code.map(str::trim).filter(|s| !s.is_empty());
    if code.is_none() && order_id.is_none() {
        return Ok(None);
    }
    let matter: Option<i64> = sqlx::query_scalar(
        r#"SELECT d.fk_delivery
             FROM "isahl"."zc_id_deta-trade_order" d
             JOIN "isahl"."zc_id_stat-trade_order" st
               ON st.id = d.fk_list AND st.deleted_at IS NULL
            WHERE d.deleted_at IS NULL
              AND d.fk_delivery IS NOT NULL
              AND d.code NOT LIKE 'DTL-LDG-%'
              AND (($1::text IS NOT NULL AND st.code = $1)
                OR ($2::bigint IS NOT NULL AND st.id = $2))
            ORDER BY d.id LIMIT 1"#,
    )
    .bind(code)
    .bind(order_id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    Ok(matter)
}

/// 事件↔交付事项桥（`zc_id_event_rr_matter`）：把事件挂到订单的**交付事项**上，读径据此再经
/// `zc_id_deta-trade_order.fk_delivery` 回到订单/运单（链见 crate 文档）。`notice` 由调用方给定。
///
/// 幂等：同 `(ref_left, ref_right)` 存活的桥行已存在则零写入（`WHERE NOT EXISTS`，不依赖唯一约束——
/// 该桥同时承载装载事项（`ref_right = prod-loading`），本函数只写交付事项面）。
pub async fn link_event_matter_tx(
    conn: &mut PgConnection,
    ctx: &DamageWriteContext,
    accident_id: i64,
    matter_id: i64,
    notice: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_event_rr_matter"
           (id, code, notice, ref_left, ref_right, comments, created_by_id)
           SELECT isahl.gen_next_uid(333), $1, $2, $3, $4, 'damage-event-order-chain', $5
            WHERE NOT EXISTS (
                SELECT 1 FROM "isahl"."zc_id_event_rr_matter"
                 WHERE ref_left = $3 AND ref_right = $4 AND deleted_at IS NULL)"#,
    )
    .bind(format!("EVT-M-{accident_id}-{matter_id}"))
    .bind(notice)
    .bind(accident_id)
    .bind(matter_id)
    .bind(ctx.actor_user_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
