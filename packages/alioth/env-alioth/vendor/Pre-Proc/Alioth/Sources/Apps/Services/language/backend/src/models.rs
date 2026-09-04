//! 语言模型 — L2 DTO 层
//!
//! 语言包清单存储在 isahl.zc_id_prot-env_config，通过 code LIKE 'lang:%' 过滤。
//! 元数据（locale/enabled/coverage）存于 settings JSONB 列，
//! 字段名 notice 以保持前端 toFrontend() 兼容。

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// 语言实体 — 映射 `isahl.zc_id_prot-env_config`，code 前缀 `lang:`
/// 元数据（locale/enabled/coverage）从 settings JSONB 展开为顶层字段。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Language {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    pub locale: Option<String>,
    pub enabled: Option<bool>,
    #[serde(with = "rust_decimal::serde::float_option")]
    pub coverage: Option<Decimal>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}
