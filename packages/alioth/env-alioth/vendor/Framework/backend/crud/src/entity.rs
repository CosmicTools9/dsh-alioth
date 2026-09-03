//! AliothDbEntity trait
//!
//! 声明实体元数据，是 QueryBuilder 和 AliothRepository 的入口约束。

use serde::Serialize;
use sqlx::FromRow;

/// 可标识 trait，用于有 ID 的实体
pub trait Identifiable {
    fn id(&self) -> i64;
}

/// 敏感列声明——DTO json_path + 物理列 + 可选标量引用表
///
/// `scalar_table` 非空时（如 `zc_id_scal-price`），`_sensitive` 值为标量解析结果
/// `{ "value": <mark>, "unit": <unit.notice> }`（对齐 DTO 标量值形态，见 common::scalar）；
/// 为 `None` 时返回物理列裸值。
pub struct SensitiveColumn {
    /// DTO json_path（前端字段名，如 "price"）
    pub dto: &'static str,
    /// 物理列名（与 SELECT_FIELDS 一致，如 "qk_price"）
    pub column: &'static str,
    /// 标量引用表（qk_* 指向的 zc_id_scal-* 表）；非标量引用为 None
    pub scalar_table: Option<&'static str>,
}

/// Alioth 标准实体 trait
///
/// 模块为每个 CRUD 实体实现此 trait，向框架声明物理表结构和行为约定。
///
/// # 约定
/// - `table_name()`: 完整物理表名，如 `"isahl.zc_id_production"` 或 `r#"isahl."zc_id_stor-plc-warehouse""#`
/// - `SELECT_FIELDS`: 显式字段列表，禁止 `*`
/// - `SOFT_DELETE`: 为 `true` 时，QueryBuilder 自动追加 `deleted_at IS NULL`
/// - `HAS_AUDIT`: 为 `true` 时，QueryBuilder 在软删除中自动注入 `updated_by_id`
/// - `SENSITIVE_COLUMNS`: 敏感列组——声明后 SELECT 恒带 `_sensitive` jsonb 投影列
///   （NGAC 列级裁剪通道，见 reference::build_sensitive_suffix）。
///   空 = 不启用列级裁剪，SELECT 形态与历史一致（零回归）。
///   声明后 struct MUST 含 `#[sqlx(default)] _sensitive: Option<serde_json::Value>` 字段。
pub trait AliothDbEntity:
    Identifiable + for<'r> FromRow<'r, sqlx::postgres::PgRow> + Send + Sync + Unpin + Serialize
{
    /// 完整物理表名
    fn table_name() -> &'static str;

    const SELECT_FIELDS: &'static str;
    const ENTITY_NAME: &'static str;
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
    const NGAC_RESOURCE_TYPE: &'static str = Self::ENTITY_NAME;
    const SENSITIVE_COLUMNS: &'static [SensitiveColumn] = &[];
    /// Optional coordinate discriminator WHERE clause for entities sharing a DB table.
    /// When set, this raw SQL fragment (without "AND") is appended to every list/get query.
    const COORDINATE_FILTER: &'static str = "";
    /// 本体坐标声明（回退坐标，REQ-DATA-002）：`X-Alioth-Coord` header 缺失时，
    /// 通用 CRUD handler 按此声明回填 `dk_*`（见 `handler::resolve_dk_ctx`）。
    /// 默认不声明 → `dk_*` 保持 NULL（不 fail-closed，兼容无坐标实体）。
    /// 声明值为坐标 **code**（scene/factor/function 维度 code，如 "YA"/"FBA"/"↓_EE"），
    /// 运行时经 ontology-binding 按 code 解析 ZUID（BACKEND_FRAMEWORK §7.3.3 2026-08-12
    /// 裁定：禁硬编码 ZUID；code 跨 dev/pre/prod 稳定）。
    const DK_SCENE: Option<&'static str> = None;
    const DK_FACTOR: Option<&'static str> = None;
    const DK_FUNCTION: Option<&'static str> = None;
    fn trigger_table_name() -> &'static str {
        let name = Self::table_name();
        let name = name
            .strip_prefix("isahl.")
            .or_else(|| name.strip_prefix("isahl_auth."))
            .unwrap_or(name);
        name.trim_matches('"')
    }

    /// DTO 字段名 → SQL 列名映射表
    /// 当前端 filter_field 使用 DTO 字段名（如 ck_cate_biz）而 SQL 列名含连字符（"ck_cate-biz"）时使用。
    fn column_renames() -> Vec<(&'static str, &'static str)> {
        vec![]
    }
}
