//! # damage-writer — 损管/异常事件写链共享事务
//!
//! 单一事实源：`zc_id_even-accident`（损管/异常事件）主行 + `zc_id_event_rr_container`
//! （事件↔车辆容器桥）两张表的写事务抽取，供两类调用方共用（REUSE_FIRST，禁止第二套实现）：
//!
//! - **Gateway WZ**（logi-consignment `FjaRepository::create_damage`）：平台调度员创建损管；
//! - **OpenActivity 门户**（portal_write `insert_even_accident`）：承运异常上报 / 客户申报。
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

/// 事件↔车辆桥（`zc_id_event_rr_container`）：车牌 → `zc_id_identity`（`ck_category='plate'`）
/// 反查车辆实体 → INSERT 桥（`gen_next_uid(473)`、code `EVT-CNT-{id}-{vid}`）。
/// 车牌为空（或反查无车）→ no-op（与迁移前一致）。`notice` 语义由调用方给定。
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
    let vehicle_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT er.ref_left FROM "isahl"."zc_id_identity" i
           JOIN "isahl"."zc_id_entity_rr_identity" er
             ON er.ref_right = i.id AND er.deleted_at IS NULL
           WHERE i.identity = $1 AND i.deleted_at IS NULL
             AND i.ck_category = (SELECT id FROM "isahl"."zc_id_cate-identity"
                                  WHERE code = 'plate' AND deleted_at IS NULL LIMIT 1)
           ORDER BY er.id DESC LIMIT 1"#,
    )
    .bind(plate)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
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
