//! 一式两份**订单/运单镜像**写件（委托链与派车链共用单一实现）。
//!
//! 语义（用户裁决 2026-09-11，矩阵 #2/#6/#7）：镜像单据落**同表同类别**（`zc_id_orde-land`，
//! 不翻转类别码）、甲/乙主体互换（`fk_subject` ↔ `fk_object`）、`code = {主code}-R`；
//! 互链走**叶子**桥 `zc_id_lifecycle_rr_form`（`ref_left`=主 / `ref_right`=镜像；
//! 非叶 `zc_id_lifecycle_rr_non_self` 82 子表不可直插）；镜像行与主行**同等待遇**——
//! 状态桥经 `sync_mirror_status_tx` 随主行同位（矩阵：镜像行 MUST 有状态桥）。
//!
//! 形态 `_f_`/`_t_` 由本模块经 `trigger_registry::lifecycle::derive_form_type(fn_code)` 派生后
//! 参数绑定（单一派生源；DB 无 `tf_bf_lifecycle__f__type` 触发器）。
//!
//! 边界：只承载镜像行与互链桥；镜像产品、单据扩展列与业务校验留在各调用方。

use sqlx::PgConnection;

use common::AliothError;
use trigger_registry::lifecycle::derive_form_type;

/// 镜像单据行入参（主体由调用方按「甲/乙互换」给定）。
#[derive(Debug, Clone)]
pub struct OrderMirrorInput<'a> {
    /// 镜像编号（约定 `{主code}-R`；编号是标签，识别一律以桥存在性为准）
    pub code: &'a str,
    pub notice: &'a str,
    pub comments: &'a str,
    /// 镜像 `fk_subject`（= 主单 `fk_object`；承运主体可缺省 → NULL）
    pub subject: Option<i64>,
    /// 镜像 `fk_object`（= 主单 `fk_subject`）
    pub object: Option<i64>,
    /// 日期标量引用（`qk_date`；委托链不写，派车运单链写）
    pub qk_date: Option<i64>,
    /// 形态派生源（职能码：`↓_BE` 等六前缀之一）
    pub fn_code: &'a str,
    /// 单据标签（"订单" / "运单"）——仅用于桥行人类可读文案
    pub kind_label: &'a str,
    pub user_id: i64,
}

/// 插入镜像单据行 + `zc_id_lifecycle_rr_form` 互链桥，返回镜像单据 id。
pub async fn insert_order_mirror_tx(
    conn: &mut PgConnection,
    main_order_id: i64,
    input: &OrderMirrorInput<'_>,
) -> Result<i64, AliothError> {
    let (form, tier) = derive_form_type(input.fn_code).ok_or_else(|| AliothError::Validation {
        field: "fnCode".into(),
        message: format!(
            "职能码 {} 无法派生 _f_/_t_（须为 !./!_/↑./↑_/↓./↓_ 六前缀之一）",
            input.fn_code
        ),
    })?;

    let (dk_scene, dk_factor, dk_function) =
        crate::coords::resolve_coords(conn, crate::coords::WriterDk::OrderDocument).await?;

    let mirror_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_orde-land"
           (id, code, notice, comments, fk_subject, fk_object, qk_date, created_by_id, "_f_", "_t_",
            dk_scene, dk_factor, dk_function, ak_permit_user)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, ARRAY[$7]::bigint[])
           RETURNING id"#,
    )
    .bind(input.code)
    .bind(input.notice)
    .bind(input.comments)
    .bind(input.subject)
    .bind(input.object)
    .bind(input.qk_date)
    .bind(input.user_id)
    .bind(form)
    .bind(tier)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;

    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_lifecycle_rr_form"
           (id, ref_left, ref_right, notice, code, comments, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, $2, '一式两份', $3, $4, $5)"#,
    )
    .bind(main_order_id)
    .bind(mirror_id)
    .bind(format!("MIR-{main_order_id}-{mirror_id}"))
    .bind(format!(
        "镜像单据：主{} {main_order_id} ↔ 反向{} {mirror_id}",
        input.kind_label, input.kind_label
    ))
    .bind(input.user_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;

    // 矩阵「镜像行与主行同等待遇：状态桥」：镜像行内即承载主行当前状态
    //（主行此刻无状态 → no-op，二者同为「新建」）。主行此后每次状态流转，
    // 由调用方在主行状态写后调用 `sync_mirror_status_tx` 同步（本 crate 不驻留状态机）。
    sync_mirror_status_tx(&mut *conn, main_order_id, input.user_id).await?;

    Ok(mirror_id)
}

/// 镜像行状态桥同步（矩阵「镜像行与主行同等待遇：状态桥」）。
///
/// 把主行当前主状态桥（`zc_id_lifecycle_r_primary-status`）原样复制到其镜像行——
/// 镜像行经**叶子**桥 `zc_id_lifecycle_rr_form` 的 `MIR-%` 行定位
///（识别一律以桥存在性为准，不用 code 前缀猜）。主行尚无状态 → no-op
///（镜像与主行同为「新建/无状态」，即同等待遇）。
///
/// 幂等：`ref_left` 上为全表唯一约束（含软删行），一律 `ON CONFLICT (ref_left)`
/// 原地更新/复活——与 `common::status::upsert_status_tx` 同范式。
/// 返回同步的镜像行数（无镜像或主行无状态 → 0）。
pub async fn sync_mirror_status_tx(
    conn: &mut PgConnection,
    main_order_id: i64,
    user_id: i64,
) -> Result<u64, AliothError> {
    let affected = sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_lifecycle_r_primary-status"
             (ref_left, ref_right, code, notice, status_date, created_by_id, updated_by_id)
           SELECT mir.ref_right, src.ref_right, src.code, '状态流转（镜像同步）',
                  src.status_date, $2, $2
           FROM "isahl"."zc_id_lifecycle_r_primary-status" src
           JOIN "isahl"."zc_id_lifecycle_rr_form" mir
             ON mir.ref_left = src.ref_left AND mir.code LIKE 'MIR-%' AND mir.deleted_at IS NULL
           JOIN "isahl"."zc_id_orde-land" o
             ON o.id = mir.ref_right AND o.deleted_at IS NULL
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
    .bind(main_order_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    Ok(affected.rows_affected())
}
