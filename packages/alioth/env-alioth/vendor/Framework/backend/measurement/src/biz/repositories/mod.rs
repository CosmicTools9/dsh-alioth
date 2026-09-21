//! biz 面 Repository——共享内核（consolidate-duplicated-services A′）
//!
//! 生产逻辑从 WZ/Alioth measurement 提取：INSERT 按量纲路由到叶表、
//! id 依赖列默认 `gen_next_uid(table_code)`（MUST NOT gen_next_zuid）。

pub mod exchange_rate;
pub mod scalar_price;
pub mod unit;
pub mod unit_conversion_rate;

pub use exchange_rate::ExchangeRateRepository;
pub use scalar_price::ScalarPriceRepository;
pub use unit::MeasurementUnitRepository;
pub use unit_conversion_rate::UnitConversionRateRepository;

/// 量纲叶表静态句柄：表名与 INSERT 语句在**编译期**由 `concat!` 固化
/// （运行期不拼串、无 `AssertSqlSafe`；INSERT 正文单一来源，见两个族宏）。
pub struct DimensionLeafSql {
    /// 完全限定叶表名（如 `isahl."zc_id_unit-temperature"`）
    pub table: &'static str,
    /// INSERT 语句（列清单与实体结构一致）
    pub insert_sql: &'static str,
}

/// 单元叶表句柄（`zc_id_unit` 族）
macro_rules! unit_leaf_sql {
    ($table:literal) => {
        DimensionLeafSql {
            table: concat!("isahl.\"", $table, "\""),
            insert_sql: concat!(
                "INSERT INTO isahl.\"",
                $table,
                "\" AS e (notice, code, symbol, system, base, t_color_, created_by_id) ",
                "VALUES ($1, $2, $3, $4::isahl.zc_id_unit_system_enum, $5, $6, $7) ",
                "RETURNING id, notice AS name, code, symbol, ",
                "CASE WHEN e.tableoid = 'isahl.zc_id_unit'::regclass THEN NULL ",
                "ELSE replace((SELECT relname FROM pg_class WHERE oid = e.tableoid), ",
                "'zc_id_unit-', '') END AS dimension, ",
                "system::text AS system, base, t_color_, created_at, updated_at, deleted_at"
            ),
        }
    };
}

/// 换算率叶表句柄（`zc_id_rate` 族）
macro_rules! rate_leaf_sql {
    ($table:literal) => {
        DimensionLeafSql {
            table: concat!("isahl.\"", $table, "\""),
            insert_sql: concat!(
                "INSERT INTO isahl.\"",
                $table,
                "\" AS e (notice, ck_left, ck_right, multiply, division, precision_, intrinsic, ",
                "created_by_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ",
                "RETURNING id, notice AS name, ck_left AS left, ck_right AS right, ",
                "multiply, division, precision_, intrinsic, ",
                "CASE WHEN e.tableoid = 'isahl.zc_id_rate'::regclass THEN NULL ",
                "ELSE replace((SELECT relname FROM pg_class WHERE oid = e.tableoid), ",
                "'zc_id_rate-', '') END AS dimension, ",
                "created_at, updated_at, deleted_at"
            ),
        }
    };
}

/// 单元量纲 → 叶表句柄（键 = `dimension_key` 取值，与旧 match 臂逐一对应）。
/// 族成员新增 = 在此加一行；未知量纲回落 [`UNIT_LEAF_PARENT`]（与旧 `_` 臂同口径）。
const UNIT_LEAF_SQLS: &[(&str, DimensionLeafSql)] = &[
    ("temperature", unit_leaf_sql!("zc_id_unit-temperature")),
    ("current", unit_leaf_sql!("zc_id_unit-current")),
    ("intensity", unit_leaf_sql!("zc_id_unit-intensity")),
    ("density", unit_leaf_sql!("zc_id_unit-density")),
    ("speed", unit_leaf_sql!("zc_id_unit-speed")),
    ("pressure", unit_leaf_sql!("zc_id_unit-pressure")),
    ("power", unit_leaf_sql!("zc_id_unit-power")),
    ("voltage", unit_leaf_sql!("zc_id_unit-voltage")),
    ("angle", unit_leaf_sql!("zc_id_unit-angle")),
    ("frequency", unit_leaf_sql!("zc_id_unit-frequency")),
    ("radiation", unit_leaf_sql!("zc_id_unit-radiation")),
    ("luminance", unit_leaf_sql!("zc_id_unit-luminance")),
    ("magnetic_flux", unit_leaf_sql!("zc_id_unit-magnetic_flux")),
    (
        "magnetic_field_strength",
        unit_leaf_sql!("zc_id_unit-magnetic_field_strength"),
    ),
    ("stress", unit_leaf_sql!("zc_id_unit-stress")),
    ("display", unit_leaf_sql!("zc_id_unit-display")),
    ("pricing", unit_leaf_sql!("zc_id_unit-pricing")),
    ("price", unit_leaf_sql!("zc_id_unit-price")),
    ("distance", unit_leaf_sql!("zc_id_unit-distance")),
    ("duration", unit_leaf_sql!("zc_id_unit-duration")),
    ("area", unit_leaf_sql!("zc_id_unit-area")),
    ("volume", unit_leaf_sql!("zc_id_unit-volume")),
    ("weight", unit_leaf_sql!("zc_id_unit-weight")),
    ("data", unit_leaf_sql!("zc_id_unit-data")),
    ("currency", unit_leaf_sql!("zc_id_unit-currency")),
    ("container", unit_leaf_sql!("zc_id_unit-container")),
    ("common", unit_leaf_sql!("zc_id_unit-common")),
    ("energy", unit_leaf_sql!("zc_id_unit-energy")),
    ("working", unit_leaf_sql!("zc_id_unit-working")),
];

/// 换算率量纲 → 叶表句柄（键 = `dimension_key` 取值，与旧 match 臂逐一对应）。
const RATE_LEAF_SQLS: &[(&str, DimensionLeafSql)] = &[
    ("angle", rate_leaf_sql!("zc_id_rate-angle")),
    ("area", rate_leaf_sql!("zc_id_rate-area")),
    ("container", rate_leaf_sql!("zc_id_rate-container")),
    ("current", rate_leaf_sql!("zc_id_rate-current")),
    ("custom", rate_leaf_sql!("zc_id_rate-custom")),
    ("data", rate_leaf_sql!("zc_id_rate-data")),
    ("density", rate_leaf_sql!("zc_id_rate-density")),
    ("distance", rate_leaf_sql!("zc_id_rate-distance")),
    ("duration", rate_leaf_sql!("zc_id_rate-duration")),
    ("energy", rate_leaf_sql!("zc_id_rate-energy")),
    ("exchange", rate_leaf_sql!("zc_id_rate-exchange")),
    ("frequency", rate_leaf_sql!("zc_id_rate-frequency")),
    ("intensity", rate_leaf_sql!("zc_id_rate-intensity")),
    ("luminance", rate_leaf_sql!("zc_id_rate-luminance")),
    (
        "magnetic_field_strength",
        rate_leaf_sql!("zc_id_rate-magnetic_field_strength"),
    ),
    ("magnetic_flux", rate_leaf_sql!("zc_id_rate-magnetic_flux")),
    ("power", rate_leaf_sql!("zc_id_rate-power")),
    ("pressure", rate_leaf_sql!("zc_id_rate-pressure")),
    ("radiation", rate_leaf_sql!("zc_id_rate-radiation")),
    ("speed", rate_leaf_sql!("zc_id_rate-speed")),
    ("stress", rate_leaf_sql!("zc_id_rate-stress")),
    ("temperature", rate_leaf_sql!("zc_id_rate-temperature")),
    ("voltage", rate_leaf_sql!("zc_id_rate-voltage")),
    ("volume", rate_leaf_sql!("zc_id_rate-volume")),
    ("weight", rate_leaf_sql!("zc_id_rate-weight")),
];

/// 未知量纲回落：父表 `zc_id_unit`（无 `dimension_key` 时插入父表）
pub const UNIT_LEAF_PARENT: DimensionLeafSql = unit_leaf_sql!("zc_id_unit");

/// 未知量纲回落：父表 `zc_id_rate`（无 `dimension_key` 时插入父表）
pub const RATE_LEAF_PARENT: DimensionLeafSql = rate_leaf_sql!("zc_id_rate");

/// 量纲 → 单元叶表句柄（未知量纲回落父表，与旧 match `_` 臂一致）。
pub fn unit_leaf_sql_for_dimension(dim_key: &str) -> &'static DimensionLeafSql {
    UNIT_LEAF_SQLS
        .iter()
        .find(|(k, _)| *k == dim_key)
        .map(|(_, sql)| sql)
        .unwrap_or(&UNIT_LEAF_PARENT)
}

/// 量纲 → 换算率叶表句柄（未知量纲回落父表，与旧 match `_` 臂一致）。
pub fn rate_leaf_sql_for_dimension(dim_key: &str) -> &'static DimensionLeafSql {
    RATE_LEAF_SQLS
        .iter()
        .find(|(k, _)| *k == dim_key)
        .map(|(_, sql)| sql)
        .unwrap_or(&RATE_LEAF_PARENT)
}

/// 量纲→叶表映射。INSERT 路由到对应子表而非父表 `zc_id_unit`。
pub fn unit_leaf_table_for_dimension(dim_key: &str) -> &'static str {
    unit_leaf_sql_for_dimension(dim_key).table
}

/// 量纲→叶表映射。INSERT 路由到对应子表而非父表 `zc_id_rate`。
pub fn rate_leaf_table_for_dimension(dim_key: &str) -> &'static str {
    rate_leaf_sql_for_dimension(dim_key).table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_leaf_routing_known_dimensions() {
        assert_eq!(
            unit_leaf_table_for_dimension("temperature"),
            "isahl.\"zc_id_unit-temperature\""
        );
        assert_eq!(
            unit_leaf_table_for_dimension("currency"),
            "isahl.\"zc_id_unit-currency\""
        );
        assert_eq!(
            unit_leaf_table_for_dimension("unknown"),
            "isahl.\"zc_id_unit\""
        );
        assert_eq!(unit_leaf_table_for_dimension(""), "isahl.\"zc_id_unit\"");
    }

    #[test]
    fn rate_leaf_routing_known_dimensions() {
        assert_eq!(
            rate_leaf_table_for_dimension("exchange"),
            "isahl.\"zc_id_rate-exchange\""
        );
        assert_eq!(
            rate_leaf_table_for_dimension("temperature"),
            "isahl.\"zc_id_rate-temperature\""
        );
        assert_eq!(
            rate_leaf_table_for_dimension("bogus"),
            "isahl.\"zc_id_rate\""
        );
    }

    #[test]
    fn all_unit_dimensions_have_mapping() {
        // 全量纲覆盖：任何量纲 key 不得落入未知分支（父表兜底除外）
        for dim in [
            "temperature",
            "current",
            "intensity",
            "density",
            "speed",
            "pressure",
            "power",
            "voltage",
            "angle",
            "frequency",
            "radiation",
            "luminance",
            "magnetic_flux",
            "magnetic_field_strength",
            "stress",
            "display",
            "pricing",
            "price",
            "distance",
            "duration",
            "area",
            "volume",
            "weight",
            "data",
            "currency",
            "container",
            "common",
            "energy",
            "working",
        ] {
            let t = unit_leaf_table_for_dimension(dim);
            assert!(
                t.contains(&format!("zc_id_unit-{}", dim)) || t == "isahl.\"zc_id_unit\"",
                "量纲 {} 未映射: {}",
                dim,
                t
            );
        }
    }

    #[test]
    fn all_rate_dimensions_have_mapping() {
        for dim in [
            "angle",
            "area",
            "container",
            "current",
            "custom",
            "data",
            "density",
            "distance",
            "duration",
            "energy",
            "exchange",
            "frequency",
            "intensity",
            "luminance",
            "magnetic_field_strength",
            "magnetic_flux",
            "power",
            "pressure",
            "radiation",
            "speed",
            "stress",
            "temperature",
            "voltage",
            "volume",
            "weight",
        ] {
            let t = rate_leaf_table_for_dimension(dim);
            assert!(
                t.contains(&format!("zc_id_rate-{}", dim)) || t == "isahl.\"zc_id_rate\"",
                "量纲 {} 未映射: {}",
                dim,
                t
            );
        }
    }
}
