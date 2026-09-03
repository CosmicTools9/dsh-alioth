//! 本体坐标静态绑定基础设施（BACKEND_FRAMEWORK §7.3.3，2026-08-12 裁定）
//!
//! 裁定：同表不同 dk → 接口语义必然不同 → 接口 API 必然不同；每个 API 内 dk 三元组固定，
//! 在 Service 实现时静态绑定（实体 → code 三元组）。值按 code 解析维度行 ZUID
//! （跨 dev/pre/prod 稳定），禁止前端传值 / 运行时推导 / 硬编码环境相关 ZUID。
//!
//! 本 crate 提供：
//! - [`DkBinding`]：实体 → 固定三元组 code 的绑定 trait（各 Service 声明自己的实体枚举）
//! - [`resolve`] / [`resolve_conn`]：code → ZUID 解析（pool / connection 两版，一处实现）
//!
//! 实体映射表保留在各 Service（不同 namespace 同实体名坐标合法不同，§7.3.2）——
//! 本 crate 不预置任何实体映射。

use sqlx::{PgConnection, PgPool, Row};

/// (scene_code, factor_code, function_code)——固定三元组 code
pub type Coords = (&'static str, &'static str, &'static str);

/// dk 静态绑定 trait：实体 → 固定三元组 code
///
/// 每个写 API 的实体实现本 trait（match 一分支一三元组）；语义校准只改对应分支。
pub trait DkBinding {
    fn coords(&self) -> Coords;
}

/// code → ZUID 解析（pool 版）。维度行缺失（未种子）→ None；列可空场景合法。
pub async fn resolve(
    pool: &PgPool,
    coords: Coords,
) -> Result<(Option<i64>, Option<i64>, Option<i64>), sqlx::Error> {
    let (scene, factor, function) = coords;
    let row = sqlx::query(
        r#"SELECT
            (SELECT id FROM "isahl"."zc_id_scene" WHERE code = $1 AND deleted_at IS NULL LIMIT 1),
            (SELECT id FROM "isahl"."zc_id_factor" WHERE code = $2 AND deleted_at IS NULL LIMIT 1),
            (SELECT id FROM "isahl"."zc_id_function" WHERE code = $3 AND deleted_at IS NULL LIMIT 1)"#,
    )
    .bind(scene)
    .bind(factor)
    .bind(function)
    .fetch_one(pool)
    .await?;
    Ok((row.try_get(0)?, row.try_get(1)?, row.try_get(2)?))
}

/// code → ZUID 解析（connection 版，用于事务内）
pub async fn resolve_conn(
    conn: &mut PgConnection,
    coords: Coords,
) -> Result<(Option<i64>, Option<i64>, Option<i64>), sqlx::Error> {
    let (scene, factor, function) = coords;
    let row = sqlx::query(
        r#"SELECT
            (SELECT id FROM "isahl"."zc_id_scene" WHERE code = $1 AND deleted_at IS NULL LIMIT 1),
            (SELECT id FROM "isahl"."zc_id_factor" WHERE code = $2 AND deleted_at IS NULL LIMIT 1),
            (SELECT id FROM "isahl"."zc_id_function" WHERE code = $3 AND deleted_at IS NULL LIMIT 1)"#,
    )
    .bind(scene)
    .bind(factor)
    .bind(function)
    .fetch_one(conn)
    .await?;
    Ok((row.try_get(0)?, row.try_get(1)?, row.try_get(2)?))
}
