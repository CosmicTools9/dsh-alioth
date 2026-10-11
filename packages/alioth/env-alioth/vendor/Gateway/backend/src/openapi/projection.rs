//! 产品投影（L4 接线，change `wire-openapi-product-projection`）
//!
//! 规约：`openspec/specs/openapi-external-access :: product-projection-wiring`。
//!
//! 语义（唯一实现；文档面与 PEP **同源消费**，MUST NOT 各自复刻判定）：
//! - 产品行（`isahl."zc_id_prod-openapi-{sales,purchase,made,request}"`）的 `projection`
//!   = 文档路径前缀（`/api/service/` 之后，去前导斜杠），或 `*`（显式全量）；
//! - 投影集 = 非删除行的非空 projection 值集合；**空集或含 `*` ⇒ 全量候选**
//!   （无产品数据的 ns 行为与接线前逐字一致）；
//! - 匹配按**路径段边界**（`waybills` 不匹配 `waybill` / `waybills-x`）；
//! - 失败语义：产品表缺失 / 查询失败 ⇒ 按空集（全量）——该面是**收窄**功能，
//!   而非鉴权前提（scope + NGAC 仍为第一道判据），收窄缺失 MUST NOT 使服务不可用。

use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

/// 投影集缓存 TTL（60s——与订阅解析缓存同节奏，产品变更低频）
const PROJECTION_CACHE_TTL: Duration = Duration::from_secs(60);

/// 产品表投影查询（4 张表各一条**字面量** SQL：表名不拼接，避免动态表名面）
const SALES_PROJECTIONS_SQL: &str = r#"SELECT projection FROM isahl."zc_id_prod-openapi-sales"
     WHERE deleted_at IS NULL AND projection IS NOT NULL AND btrim(projection) <> ''"#;
const PURCHASE_PROJECTIONS_SQL: &str = r#"SELECT projection FROM isahl."zc_id_prod-openapi-purchase"
     WHERE deleted_at IS NULL AND projection IS NOT NULL AND btrim(projection) <> ''"#;
const MADE_PROJECTIONS_SQL: &str = r#"SELECT projection FROM isahl."zc_id_prod-openapi-made"
     WHERE deleted_at IS NULL AND projection IS NOT NULL AND btrim(projection) <> ''"#;
const REQUEST_PROJECTIONS_SQL: &str = r#"SELECT projection FROM isahl."zc_id_prod-openapi-request"
     WHERE deleted_at IS NULL AND projection IS NOT NULL AND btrim(projection) <> ''"#;

/// 管理面前缀（`/api/service/openapi/*`）——NGAC `openapi_admin:0` 已门控，
/// 不属「数据服务消费面」，投影判定 MUST NOT 影响之。
const MANAGEMENT_FACE_SEGMENT: &str = "openapi";

/// 业务数据面路径前缀（投影判定的作用域）。
const SERVICE_PATH_PREFIX: &str = "/api/service/";

/// 投影集（不可变值；克隆廉价）。
#[derive(Clone, Debug)]
pub struct ProductProjection {
    /// 生效前缀（段序列，无前导斜杠）——`full` 为真时忽略
    prefixes: Vec<String>,
    /// 全量候选（投影集为空，或含 `*`）
    full: bool,
}

impl Default for ProductProjection {
    fn default() -> Self {
        Self::full()
    }
}

impl ProductProjection {
    /// 全量候选（无产品数据 / 含 `*` / 判定不可用时的回落）。
    pub fn full() -> Self {
        Self {
            prefixes: Vec::new(),
            full: true,
        }
    }

    /// 由产品行 projection 值构建投影集（空串/NULL 行不参与）。
    pub fn from_projections<I, S>(rows: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut prefixes: Vec<String> = Vec::new();
        let mut full = false;
        for raw in rows {
            let trimmed = raw.as_ref().trim().trim_start_matches('/');
            let normalized = trimmed.trim_end_matches('/');
            if normalized.is_empty() {
                continue; // 空/NULL 行不参与投影集（避免半成品配置清空文档）
            }
            if normalized == "*" {
                full = true;
                continue;
            }
            if !prefixes.iter().any(|p| p == normalized) {
                prefixes.push(normalized.to_string());
            }
        }
        // 投影集为空 ⇒ 全量候选（向后兼容的不变式）
        if prefixes.is_empty() {
            full = true;
        }
        Self { prefixes, full }
    }

    /// 是否全量候选（投影判定被短路）。
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// 生效前缀条数（状态标注用）。
    pub fn prefix_count(&self) -> usize {
        self.prefixes.len()
    }

    /// 生效前缀（只读展示用）。
    pub fn prefixes(&self) -> &[String] {
        &self.prefixes
    }

    /// 该请求路径是否在投影集内（`full` ⇒ 恒真）。
    ///
    /// - 非 `/api/service/` 路径（文档端点、`/api/auth/*`、`/api/ngac/*` 等）⇒ 不受投影约束；
    /// - 管理面 `/api/service/openapi/*` ⇒ 不受投影约束（NGAC 门控）；
    /// - 其余业务路径 ⇒ 按段边界前缀匹配。
    pub fn allows(&self, path: &str) -> bool {
        if self.full {
            return true;
        }
        let Some(rest) = path.strip_prefix(SERVICE_PATH_PREFIX) else {
            return true;
        };
        let first = rest.split('/').next().unwrap_or("");
        if first == MANAGEMENT_FACE_SEGMENT {
            return true;
        }
        let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        self.prefixes.iter().any(|prefix| {
            let needle: Vec<&str> = prefix.split('/').filter(|s| !s.is_empty()).collect();
            !needle.is_empty()
                && segments.len() >= needle.len()
                && segments[..needle.len()] == needle[..]
        })
    }

    /// 生效口径说明（文档 `info.description` 追加行与前端状态条**同源文案**）。
    pub fn status_line(&self) -> String {
        if self.full {
            "无产品投影 ⇒ 文档与授权面全量候选（产品数据未生效）".to_string()
        } else {
            format!(
                "产品投影生效：{} 条前缀（文档与授权面按投影收窄）",
                self.prefixes.len()
            )
        }
    }

    /// 按投影集裁剪文档 paths（返回可见路径数）。`x-outbound-contracts` 不裁剪。
    pub fn filter_doc(&self, doc: &mut JsonValue) -> usize {
        if self.full {
            return doc
                .get("paths")
                .and_then(JsonValue::as_object)
                .map(|m| m.len())
                .unwrap_or(0);
        }
        let Some(paths) = doc.get_mut("paths").and_then(JsonValue::as_object_mut) else {
            return 0;
        };
        paths.retain(|path, _| self.allows(path));
        paths.len()
    }
}

/// 进程级缓存单例（`LazyLock`：初始化器随声明；文档面与 PEP 共享，MUST NOT 二次查询）。
static PROJECTION_CACHE: LazyLock<ProjectionCache> = LazyLock::new(ProjectionCache::new);

/// 取全局投影缓存（进程级；与 metering 的进程内状态同语义）。
pub fn global_cache() -> &'static ProjectionCache {
    &PROJECTION_CACHE
}

/// 进程内投影集缓存（60s TTL）。
pub struct ProjectionCache {
    slot: RwLock<(ProductProjection, Instant)>,
}

impl ProjectionCache {
    fn new() -> Self {
        // 初始为「过期」⇒ 首次访问即加载
        Self {
            slot: RwLock::new((
                ProductProjection::full(),
                Instant::now() - PROJECTION_CACHE_TTL,
            )),
        }
    }

    /// 取当前投影集（TTL 内直接返回缓存；过期则重查；查询失败 ⇒ 全量并 warn）。
    pub async fn get(&self, pool: &PgPool) -> ProductProjection {
        {
            let guard = self.slot.read().unwrap();
            if guard.1.elapsed() < PROJECTION_CACHE_TTL {
                return guard.0.clone();
            }
        }
        let loaded = load_product_projection(pool).await;
        let mut guard = self.slot.write().unwrap();
        *guard = (loaded.clone(), Instant::now());
        loaded
    }

    /// 主动失效（测试用；产品写径可选调用）。
    pub fn invalidate(&self) {
        let mut guard = self.slot.write().unwrap();
        guard.1 = Instant::now() - PROJECTION_CACHE_TTL;
    }
}

/// 查询 4 张产品表并合成投影集；表缺失/查询失败 ⇒ 空集（= 全量），MUST NOT 报错。
pub async fn load_product_projection(pool: &PgPool) -> ProductProjection {
    let mut rows: Vec<String> = Vec::new();
    for (table, sql) in [
        ("sales", SALES_PROJECTIONS_SQL),
        ("purchase", PURCHASE_PROJECTIONS_SQL),
        ("made", MADE_PROJECTIONS_SQL),
        ("request", REQUEST_PROJECTIONS_SQL),
    ] {
        match sqlx::query_scalar::<_, String>(sql).fetch_all(pool).await {
            Ok(found) => rows.extend(found),
            Err(e) => {
                // 表缺失（42P01）属正常形态（未启用产品管理面的 ns）⇒ 不升级为错误
                let missing_table = e
                    .as_database_error()
                    .map(|db| db.code().as_deref() == Some("42P01"))
                    .unwrap_or(false);
                if missing_table {
                    log::debug!(
                        "product-projection: table zc_id_prod-openapi-{} absent — empty set contributes",
                        table
                    );
                } else {
                    log::warn!(
                        "product-projection: load zc_id_prod-openapi-{} failed (fall back to full candidate set): {}",
                        table,
                        e
                    );
                }
            }
        }
    }
    ProductProjection::from_projections(rows)
}
