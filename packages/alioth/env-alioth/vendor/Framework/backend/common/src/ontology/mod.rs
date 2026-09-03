//! # 本体坐标缓存
//!
//! 应用启动时从 DB 全量加载 `zc_id_scene` / `zc_id_factor` / `zc_id_function` 的
//! `(code → id)` 映射到内存。handler 层收到前端传入的三个 code 后，在此处查表换为
//! `i64` ID 后写入 CREATE 请求的 DTO。
//!
//! 避免每次请求都查 DB。
//!
//! 前端 block.json 的 coordinates 块存的是 code：
//! ```json
//! "coordinates": {
//!     "scene":    { "code": "JC" },
//!     "factor":   { "code": "GID" },
//!     "function": { "code": "↑_DA" }
//! }
//! ```

use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::OnceLock;

/// 本体坐标 code → id 映射缓存。
pub struct OntologyCache {
    scenes: HashMap<String, i64>,
    factors: HashMap<String, i64>,
    functions: HashMap<String, i64>,
}

static CACHE: OnceLock<OntologyCache> = OnceLock::new();

impl OntologyCache {
    /// 初始化：全量加载三个维度表。应用启动时调用一次。
    pub async fn init(pool: &PgPool) -> Result<(), sqlx::Error> {
        let scenes: HashMap<String, i64> = sqlx::query_as::<_, (i64, String)>(
            "SELECT id, code FROM isahl.zc_id_scene WHERE deleted_at IS NULL",
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(id, code)| (code, id))
        .collect();

        let factors: HashMap<String, i64> = sqlx::query_as::<_, (i64, String)>(
            "SELECT id, code FROM isahl.zc_id_factor WHERE deleted_at IS NULL",
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(id, code)| (code, id))
        .collect();

        let functions: HashMap<String, i64> = sqlx::query_as::<_, (i64, String)>(
            "SELECT id, code FROM isahl.zc_id_function WHERE deleted_at IS NULL",
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(id, code)| (code, id))
        .collect();

        let _ = CACHE.set(OntologyCache {
            scenes,
            factors,
            functions,
        });
        Ok(())
    }

    /// 获取 scene code → id，返回 Option<i64>。
    pub fn scene_id(code: &str) -> Option<i64> {
        CACHE.get().and_then(|c| c.scenes.get(code).copied())
    }

    pub fn factor_id(code: &str) -> Option<i64> {
        CACHE.get().and_then(|c| c.factors.get(code).copied())
    }

    pub fn function_id(code: &str) -> Option<i64> {
        CACHE.get().and_then(|c| c.functions.get(code).copied())
    }

    /// 批量转换三个 code → id。任一转换失败则返回 Err。
    pub fn resolve(
        scene_code: &str,
        factor_code: &str,
        function_code: &str,
    ) -> Result<(i64, i64, Option<i64>), String> {
        let cache = CACHE.get().ok_or("OntologyCache not initialized")?;
        let s = cache
            .scenes
            .get(scene_code)
            .copied()
            .ok_or_else(|| format!("unknown scene code: {scene_code}"))?;
        let f = cache
            .factors
            .get(factor_code)
            .copied()
            .ok_or_else(|| format!("unknown factor code: {factor_code}"))?;
        let fn_ = cache.functions.get(function_code).copied();
        Ok((s, f, fn_))
    }
}

/// 检查是否已初始化。
pub fn is_initialized() -> bool {
    CACHE.get().is_some()
}
