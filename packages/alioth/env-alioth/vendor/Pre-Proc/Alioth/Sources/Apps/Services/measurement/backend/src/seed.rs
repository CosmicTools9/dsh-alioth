//! 单位制种子数据 — 公制 / 英制 / 数据 / 货币 预填充
//!
//! 供 `system-settings` 模块的单位制管理、汇率管理场景使用。
//! 幂等执行：若某量纲叶表已存在记录，则跳过该量纲整组数据。

use std::collections::HashMap;
use std::str::FromStr;

use common::error::AliothError;
use rust_decimal::{dec, Decimal};
use sqlx::{AssertSqlSafe, PgPool};

/// 单个待插入的单位种子。
struct SeedUnit {
    name: &'static str,
    code: &'static str,
    symbol: &'static str,
    system: &'static str,
    dimension: &'static str,
    base: bool,
    /// 相对同量纲 base 单位的倍数；为 None 表示模型无法表达（如温度偏移）。
    mult_to_base: Option<Decimal>,
}

impl SeedUnit {
    #[allow(clippy::too_many_arguments)]
    const fn new(
        name: &'static str,
        code: &'static str,
        symbol: &'static str,
        system: &'static str,
        dimension: &'static str,
        base: bool,
        mult_to_base: Option<Decimal>,
    ) -> Self {
        Self {
            name,
            code,
            symbol,
            system,
            dimension,
            base,
            mult_to_base,
        }
    }
}

fn all_seed_units() -> Vec<SeedUnit> {
    vec![
        // 公制基本单位（7 个）
        SeedUnit::new("米", "m", "m", "公制", "distance", true, Some(dec!(1))),
        SeedUnit::new(
            "千米",
            "km",
            "km",
            "公制",
            "distance",
            false,
            Some(dec!(1000)),
        ),
        SeedUnit::new(
            "厘米",
            "cm",
            "cm",
            "公制",
            "distance",
            false,
            Some(dec!(0.01)),
        ),
        SeedUnit::new(
            "毫米",
            "mm",
            "mm",
            "公制",
            "distance",
            false,
            Some(dec!(0.001)),
        ),
        SeedUnit::new(
            "微米",
            "um",
            "μm",
            "公制",
            "distance",
            false,
            Some(dec!(0.000001)),
        ),
        SeedUnit::new(
            "纳米",
            "nm",
            "nm",
            "公制",
            "distance",
            false,
            Some(dec!(0.000000001)),
        ),
        SeedUnit::new("千克", "kg", "kg", "公制", "weight", true, Some(dec!(1))),
        SeedUnit::new("克", "g", "g", "公制", "weight", false, Some(dec!(0.001))),
        SeedUnit::new(
            "毫克",
            "mg",
            "mg",
            "公制",
            "weight",
            false,
            Some(dec!(0.000001)),
        ),
        SeedUnit::new("吨", "t", "t", "公制", "weight", false, Some(dec!(1000))),
        SeedUnit::new("秒", "s", "s", "公制", "duration", true, Some(dec!(1))),
        SeedUnit::new(
            "分钟",
            "min",
            "min",
            "公制",
            "duration",
            false,
            Some(dec!(60)),
        ),
        SeedUnit::new(
            "小时",
            "h",
            "h",
            "公制",
            "duration",
            false,
            Some(dec!(3600)),
        ),
        SeedUnit::new("天", "d", "d", "公制", "duration", false, Some(dec!(86400))),
        SeedUnit::new("安培", "A", "A", "公制", "current", true, Some(dec!(1))),
        SeedUnit::new(
            "毫安",
            "mA",
            "mA",
            "公制",
            "current",
            false,
            Some(dec!(0.001)),
        ),
        SeedUnit::new(
            "开尔文",
            "K",
            "K",
            "公制",
            "temperature",
            true,
            Some(dec!(1)),
        ),
        SeedUnit::new("摄氏度", "degC", "°C", "公制", "temperature", false, None), // offset 无法表达
        SeedUnit::new("摩尔", "mol", "mol", "公制", "common", true, Some(dec!(1))),
        SeedUnit::new(
            "坎德拉",
            "cd",
            "cd",
            "公制",
            "intensity",
            true,
            Some(dec!(1)),
        ),
        // 公制导出单位
        SeedUnit::new("平方米", "m2", "m²", "公制", "area", true, Some(dec!(1))),
        SeedUnit::new(
            "平方千米",
            "km2",
            "km²",
            "公制",
            "area",
            false,
            Some(dec!(1000000)),
        ),
        SeedUnit::new("公顷", "ha", "ha", "公制", "area", false, Some(dec!(10000))),
        SeedUnit::new("立方米", "m3", "m³", "公制", "volume", true, Some(dec!(1))),
        SeedUnit::new("升", "L", "L", "公制", "volume", false, Some(dec!(0.001))),
        SeedUnit::new(
            "毫升",
            "mL",
            "mL",
            "公制",
            "volume",
            false,
            Some(dec!(0.000001)),
        ),
        SeedUnit::new("米每秒", "m_s", "m/s", "公制", "speed", true, Some(dec!(1))),
        SeedUnit::new(
            "千米每小时",
            "km_h",
            "km/h",
            "公制",
            "speed",
            false,
            Some(dec!(0.2777777777777778)),
        ),
        SeedUnit::new(
            "千克每立方米",
            "kg_m3",
            "kg/m³",
            "公制",
            "density",
            true,
            Some(dec!(1)),
        ),
        SeedUnit::new("牛顿", "N", "N", "公制", "common", false, Some(dec!(1))),
        SeedUnit::new(
            "帕斯卡",
            "Pa",
            "Pa",
            "公制",
            "pressure",
            true,
            Some(dec!(1)),
        ),
        SeedUnit::new(
            "千帕",
            "kPa",
            "kPa",
            "公制",
            "pressure",
            false,
            Some(dec!(1000)),
        ),
        SeedUnit::new(
            "兆帕",
            "MPa",
            "MPa",
            "公制",
            "pressure",
            false,
            Some(dec!(1000000)),
        ),
        SeedUnit::new("焦耳", "J", "J", "公制", "energy", true, Some(dec!(1))),
        SeedUnit::new(
            "千瓦时",
            "kWh",
            "kWh",
            "公制",
            "energy",
            false,
            Some(dec!(3600000)),
        ),
        SeedUnit::new("瓦特", "W", "W", "公制", "power", true, Some(dec!(1))),
        SeedUnit::new("千瓦", "kW", "kW", "公制", "power", false, Some(dec!(1000))),
        SeedUnit::new("伏特", "V", "V", "公制", "voltage", true, Some(dec!(1))),
        SeedUnit::new("赫兹", "Hz", "Hz", "公制", "frequency", true, Some(dec!(1))),
        SeedUnit::new(
            "千赫兹",
            "kHz",
            "kHz",
            "公制",
            "frequency",
            false,
            Some(dec!(1000)),
        ),
        SeedUnit::new(
            "兆赫兹",
            "MHz",
            "MHz",
            "公制",
            "frequency",
            false,
            Some(dec!(1000000)),
        ),
        SeedUnit::new(
            "吉赫兹",
            "GHz",
            "GHz",
            "公制",
            "frequency",
            false,
            Some(dec!(1000000000)),
        ),
        SeedUnit::new("弧度", "rad", "rad", "公制", "angle", true, Some(dec!(1))),
        SeedUnit::new(
            "度",
            "deg",
            "°",
            "公制",
            "angle",
            false,
            Some(dec!(0.017453292519943295)),
        ),
        SeedUnit::new(
            "韦伯",
            "Wb",
            "Wb",
            "公制",
            "magnetic_flux",
            false,
            Some(dec!(1)),
        ),
        SeedUnit::new(
            "特斯拉",
            "T",
            "T",
            "公制",
            "magnetic_field_strength",
            false,
            Some(dec!(1)),
        ),
        SeedUnit::new(
            "勒克斯",
            "lx",
            "lx",
            "公制",
            "luminance",
            false,
            Some(dec!(1)),
        ),
        // 英制单位
        SeedUnit::new(
            "英寸",
            "in",
            "in",
            "英制",
            "distance",
            false,
            Some(dec!(0.0254)),
        ),
        SeedUnit::new(
            "英尺",
            "ft",
            "ft",
            "英制",
            "distance",
            false,
            Some(dec!(0.3048)),
        ),
        SeedUnit::new(
            "码",
            "yd",
            "yd",
            "英制",
            "distance",
            false,
            Some(dec!(0.9144)),
        ),
        SeedUnit::new(
            "英里",
            "mi",
            "mi",
            "英制",
            "distance",
            false,
            Some(dec!(1609.344)),
        ),
        SeedUnit::new(
            "磅",
            "lb",
            "lb",
            "英制",
            "weight",
            false,
            Some(dec!(0.45359237)),
        ),
        SeedUnit::new(
            "盎司",
            "oz",
            "oz",
            "英制",
            "weight",
            false,
            Some(dec!(0.028349523125)),
        ),
        SeedUnit::new(
            "加仑",
            "gal",
            "gal",
            "英制",
            "volume",
            false,
            Some(dec!(0.00454609)),
        ),
        SeedUnit::new(
            "夸脱",
            "qt",
            "qt",
            "英制",
            "volume",
            false,
            Some(dec!(0.0011365225)),
        ),
        SeedUnit::new(
            "品脱",
            "pt",
            "pt",
            "英制",
            "volume",
            false,
            Some(dec!(0.00056826125)),
        ),
        SeedUnit::new("华氏度", "degF", "°F", "英制", "temperature", false, None), // offset 无法表达
        // 数据单位
        SeedUnit::new("字节", "B", "B", "公制", "data", true, Some(dec!(1))),
        SeedUnit::new(
            "千字节",
            "KB",
            "KB",
            "公制",
            "data",
            false,
            Some(dec!(1024)),
        ),
        SeedUnit::new(
            "兆字节",
            "MB",
            "MB",
            "公制",
            "data",
            false,
            Some(dec!(1048576)),
        ),
        SeedUnit::new(
            "吉字节",
            "GB",
            "GB",
            "公制",
            "data",
            false,
            Some(dec!(1073741824)),
        ),
        SeedUnit::new(
            "太字节",
            "TB",
            "TB",
            "公制",
            "data",
            false,
            Some(dec!(1099511627776)),
        ),
        // 货币单位
        SeedUnit::new(
            "人民币",
            "CNY",
            "¥",
            "公制",
            "currency",
            true,
            Some(dec!(1)),
        ),
        SeedUnit::new("美元", "USD", "$", "公制", "currency", false, Some(dec!(1))),
        SeedUnit::new("欧元", "EUR", "€", "公制", "currency", false, Some(dec!(1))),
        SeedUnit::new("日元", "JPY", "¥", "公制", "currency", false, Some(dec!(1))),
        SeedUnit::new("英镑", "GBP", "£", "公制", "currency", false, Some(dec!(1))),
        // 容器单位
        SeedUnit::new("件", "pcs", "pcs", "公制", "container", true, Some(dec!(1))),
        SeedUnit::new(
            "箱",
            "box",
            "box",
            "公制",
            "container",
            false,
            Some(dec!(1)),
        ),
        SeedUnit::new(
            "托",
            "plt",
            "plt",
            "公制",
            "container",
            false,
            Some(dec!(1)),
        ),
        // 显示单位
        SeedUnit::new("像素", "px", "px", "公制", "display", true, Some(dec!(1))),
        SeedUnit::new(
            "点",
            "pt_disp",
            "pt",
            "公制",
            "display",
            false,
            Some(dec!(1)),
        ),
    ]
}

/// 量纲种子静态句柄：叶表名与五条种子语句在**编译期**由 `concat!` 固化
/// （运行期零拼表名、零 `AssertSqlSafe`）。
struct DimensionSeedSql {
    /// 量纲幂等判据：该量纲单元叶表已有行？
    unit_count_sql: &'static str,
    /// 单元 INSERT（列清单与 bind 序列逐一对应）
    unit_insert_sql: &'static str,
    /// 单元 id/code/base 回读（换算率基数解析）
    unit_select_sql: &'static str,
    /// 换算率叶表裸名（`information_schema.tables` 存在性查询用；无 schema 前缀与引号）
    rate_leaf_name: &'static str,
    /// 量纲幂等判据：该量纲换算率叶表已有行？
    rate_count_sql: &'static str,
    /// 换算率 INSERT（列清单与 bind 序列逐一对应）
    rate_insert_sql: &'static str,
}

/// 量纲 → 种子语句（`$unit_leaf` / `$rate_leaf` = 叶表名尾段，如 `zc_id_unit-currency`）。
/// 量纲无独立换算率叶表时 `$rate_leaf` 传父表 `zc_id_rate`（与 Framework 路由同口径）。
macro_rules! dimension_seed_sql {
    ($unit_leaf:literal, $rate_leaf:literal) => {
        DimensionSeedSql {
            unit_count_sql: concat!(
                "SELECT COUNT(*) FROM isahl.\"",
                $unit_leaf,
                "\" WHERE deleted_at IS NULL"
            ),
            unit_insert_sql: concat!(
                "INSERT INTO isahl.\"",
                $unit_leaf,
                "\" (notice, code, symbol, system, base, created_by_id)\n                       VALUES ($1, $2, $3, $4::zc_id_unit_system_enum, $5, 0)"
            ),
            unit_select_sql: concat!(
                "SELECT id, code, base FROM isahl.\"",
                $unit_leaf,
                "\" WHERE deleted_at IS NULL ORDER BY id"
            ),
            rate_leaf_name: $rate_leaf,
            rate_count_sql: concat!(
                "SELECT COUNT(*) FROM isahl.\"",
                $rate_leaf,
                "\" WHERE ck_right = $1 AND deleted_at IS NULL"
            ),
            rate_insert_sql: concat!(
                "INSERT INTO isahl.\"",
                $rate_leaf,
                "\" (notice, ck_left, ck_right, multiply, division, precision_, intrinsic, created_by_id)\n               VALUES ($1, $2, $3, $4, NULL, NULL, true, 0)"
            ),
        }
    };
}

/// 量纲 → 种子语句句柄（键 = `all_seed_units()` 的 `SeedUnit.dimension` 闭集；
/// 新增量纲 = 在此加一行）。叶表路由与 Framework `*-leaf_table_for_dimension`
/// 的一致性由单测 `seed_sql_routes_to_framework_leafs` 守卫。
const DIMENSION_SEED_SQLS: &[(&str, DimensionSeedSql)] = &[
    (
        "angle",
        dimension_seed_sql!("zc_id_unit-angle", "zc_id_rate-angle"),
    ),
    (
        "area",
        dimension_seed_sql!("zc_id_unit-area", "zc_id_rate-area"),
    ),
    (
        "common",
        dimension_seed_sql!("zc_id_unit-common", "zc_id_rate"),
    ),
    (
        "container",
        dimension_seed_sql!("zc_id_unit-container", "zc_id_rate-container"),
    ),
    (
        "currency",
        dimension_seed_sql!("zc_id_unit-currency", "zc_id_rate"),
    ),
    (
        "current",
        dimension_seed_sql!("zc_id_unit-current", "zc_id_rate-current"),
    ),
    (
        "data",
        dimension_seed_sql!("zc_id_unit-data", "zc_id_rate-data"),
    ),
    (
        "density",
        dimension_seed_sql!("zc_id_unit-density", "zc_id_rate-density"),
    ),
    (
        "display",
        dimension_seed_sql!("zc_id_unit-display", "zc_id_rate"),
    ),
    (
        "distance",
        dimension_seed_sql!("zc_id_unit-distance", "zc_id_rate-distance"),
    ),
    (
        "duration",
        dimension_seed_sql!("zc_id_unit-duration", "zc_id_rate-duration"),
    ),
    (
        "energy",
        dimension_seed_sql!("zc_id_unit-energy", "zc_id_rate-energy"),
    ),
    (
        "frequency",
        dimension_seed_sql!("zc_id_unit-frequency", "zc_id_rate-frequency"),
    ),
    (
        "intensity",
        dimension_seed_sql!("zc_id_unit-intensity", "zc_id_rate-intensity"),
    ),
    (
        "luminance",
        dimension_seed_sql!("zc_id_unit-luminance", "zc_id_rate-luminance"),
    ),
    (
        "magnetic_field_strength",
        dimension_seed_sql!(
            "zc_id_unit-magnetic_field_strength",
            "zc_id_rate-magnetic_field_strength"
        ),
    ),
    (
        "magnetic_flux",
        dimension_seed_sql!("zc_id_unit-magnetic_flux", "zc_id_rate-magnetic_flux"),
    ),
    (
        "power",
        dimension_seed_sql!("zc_id_unit-power", "zc_id_rate-power"),
    ),
    (
        "pressure",
        dimension_seed_sql!("zc_id_unit-pressure", "zc_id_rate-pressure"),
    ),
    (
        "speed",
        dimension_seed_sql!("zc_id_unit-speed", "zc_id_rate-speed"),
    ),
    (
        "temperature",
        dimension_seed_sql!("zc_id_unit-temperature", "zc_id_rate-temperature"),
    ),
    (
        "voltage",
        dimension_seed_sql!("zc_id_unit-voltage", "zc_id_rate-voltage"),
    ),
    (
        "volume",
        dimension_seed_sql!("zc_id_unit-volume", "zc_id_rate-volume"),
    ),
    (
        "weight",
        dimension_seed_sql!("zc_id_unit-weight", "zc_id_rate-weight"),
    ),
];

/// 未登记量纲回落：父表 `zc_id_unit` / `zc_id_rate`
const PARENT_DIMENSION_SEED_SQL: DimensionSeedSql = dimension_seed_sql!("zc_id_unit", "zc_id_rate");

/// 量纲 → 种子语句句柄（未登记量纲回落父表，与 Framework 路由 `_` 臂同口径）
fn seed_sql_for_dimension(dim_key: &str) -> &'static DimensionSeedSql {
    DIMENSION_SEED_SQLS
        .iter()
        .find(|(k, _)| *k == dim_key)
        .map(|(_, sql)| sql)
        .unwrap_or(&PARENT_DIMENSION_SEED_SQL)
}

/// 向数据库预置标准单位种子数据及相对 base 单位的换算率。
///
/// 按量纲分组，若某量纲叶表已存在非删除记录，则跳过该量纲，保证幂等。
pub async fn seed_standard_units(pool: &PgPool) -> Result<usize, AliothError> {
    let units = all_seed_units();
    let mut by_dim: HashMap<&str, Vec<&SeedUnit>> = HashMap::new();
    for unit in &units {
        by_dim.entry(unit.dimension).or_default().push(unit);
    }

    let mut inserted = 0usize;

    for (dim_key, dim_units) in &by_dim {
        let dim_sql = seed_sql_for_dimension(dim_key);

        let count: (i64,) = sqlx::query_as(dim_sql.unit_count_sql)
            .fetch_one(pool)
            .await
            .map_err(AliothError::from)?;

        if count.0 > 0 {
            continue;
        }

        for unit in dim_units {
            sqlx::query(dim_sql.unit_insert_sql)
                .bind(unit.name)
                .bind(unit.code)
                .bind(unit.symbol)
                .bind(unit.system)
                .bind(unit.base)
                .execute(pool)
                .await
                .map_err(AliothError::from)?;

            inserted += 1;
        }

        seed_conversion_rates_for_dimension(pool, dim_sql, dim_units).await?;
    }

    Ok(inserted)
}

async fn seed_conversion_rates_for_dimension(
    pool: &PgPool,
    dim_sql: &DimensionSeedSql,
    dim_units: &[&SeedUnit],
) -> Result<(), AliothError> {
    // 若 rate 叶表不存在（如测试库 schema 不完整），跳过换算率插入。
    let table_exists: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'isahl' AND table_name = $1)"
    )
    .bind(dim_sql.rate_leaf_name)
    .fetch_one(pool)
    .await
    .map_err(AliothError::from)?;

    if !table_exists.0 {
        return Ok(());
    }

    // 查询该量纲叶表下所有刚插入的单位 id/code/base。
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(dim_sql.unit_select_sql)
        .fetch_all(pool)
        .await
        .map_err(AliothError::from)?;

    let id_by_code: HashMap<&str, i64> = rows
        .iter()
        .map(|(id, code, _)| (code.as_str(), *id))
        .collect();

    let base_id = rows.iter().find(|(_, _, base)| *base).map(|(id, _, _)| *id);

    let Some(base_id) = base_id else {
        return Ok(());
    };

    // 仅当该量纲尚无换算率时才插入。
    let existing_rate_count: (i64,) = sqlx::query_as(dim_sql.rate_count_sql)
        .bind(base_id)
        .fetch_one(pool)
        .await
        .map_err(AliothError::from)?;

    if existing_rate_count.0 > 0 {
        return Ok(());
    }

    for unit in dim_units {
        let Some(mult) = unit.mult_to_base else {
            continue;
        };

        let unit_id = id_by_code.get(unit.code).copied();
        let Some(unit_id) = unit_id else {
            continue;
        };

        // base 单位自身不需要 rate。
        if unit_id == base_id {
            continue;
        }

        sqlx::query(dim_sql.rate_insert_sql)
            .bind(format!("{} → base", unit.name))
            .bind(unit_id)
            .bind(base_id)
            .bind(mult)
            .execute(pool)
            .await
            .map_err(AliothError::from)?;
    }

    Ok(())
}

/// 货币种子与常用汇率对种子。
struct SeedCurrency {
    code: &'static str,
    name: &'static str,
    symbol: &'static str,
}

fn all_seed_currencies() -> Vec<SeedCurrency> {
    vec![
        SeedCurrency {
            code: "USD",
            name: "美元",
            symbol: "$",
        },
        SeedCurrency {
            code: "CNY",
            name: "人民币",
            symbol: "¥",
        },
        SeedCurrency {
            code: "EUR",
            name: "欧元",
            symbol: "€",
        },
        SeedCurrency {
            code: "GBP",
            name: "英镑",
            symbol: "£",
        },
        SeedCurrency {
            code: "JPY",
            name: "日元",
            symbol: "¥",
        },
        SeedCurrency {
            code: "HKD",
            name: "港币",
            symbol: "$",
        },
        SeedCurrency {
            code: "KRW",
            name: "韩元",
            symbol: "₩",
        },
        SeedCurrency {
            code: "AUD",
            name: "澳元",
            symbol: "$",
        },
        SeedCurrency {
            code: "CAD",
            name: "加元",
            symbol: "$",
        },
        SeedCurrency {
            code: "CHF",
            name: "瑞士法郎",
            symbol: "Fr",
        },
        SeedCurrency {
            code: "SGD",
            name: "新加坡元",
            symbol: "$",
        },
        SeedCurrency {
            code: "NZD",
            name: "新西兰元",
            symbol: "$",
        },
    ]
}

struct SeedExchangeRate {
    name: &'static str,
    left: &'static str,
    right: &'static str,
    bid: &'static str,
    ask: &'static str,
    source: &'static str,
}

fn all_seed_exchange_rates() -> Vec<SeedExchangeRate> {
    vec![
        SeedExchangeRate {
            name: "USD/CNY",
            left: "USD",
            right: "CNY",
            bid: "7.2345",
            ask: "7.2456",
            source: "PBOC",
        },
        SeedExchangeRate {
            name: "EUR/CNY",
            left: "EUR",
            right: "CNY",
            bid: "7.8234",
            ask: "7.8345",
            source: "PBOC",
        },
        SeedExchangeRate {
            name: "GBP/CNY",
            left: "GBP",
            right: "CNY",
            bid: "9.1234",
            ask: "9.1456",
            source: "PBOC",
        },
        SeedExchangeRate {
            name: "USD/EUR",
            left: "USD",
            right: "EUR",
            bid: "0.9234",
            ask: "0.9245",
            source: "ECB",
        },
        SeedExchangeRate {
            name: "USD/JPY",
            left: "USD",
            right: "JPY",
            bid: "151.23",
            ask: "151.45",
            source: "FED",
        },
    ]
}

/// 插入 ISO 4217 货币单位到 `zc_id_unit-currency`，并插入常用汇率对到 `zc_id_rate-exchange`。
/// 幂等：若对应表已存在非删除记录，则跳过。
pub async fn seed_currencies_and_rates(pool: &PgPool) -> Result<(usize, usize), AliothError> {
    let currency_count: (i64,) = sqlx::query_as(AssertSqlSafe(
        "SELECT COUNT(*) FROM isahl.\"zc_id_unit-currency\" WHERE deleted_at IS NULL",
    ))
    .fetch_one(pool)
    .await
    .map_err(AliothError::from)?;

    let mut inserted_currencies = 0usize;
    if currency_count.0 == 0 {
        for c in all_seed_currencies() {
            sqlx::query(AssertSqlSafe(
                r#"INSERT INTO isahl."zc_id_unit-currency" (notice, code, symbol, system, base, created_by_id)
                   VALUES ($1, $2, $3, '公制', true, 0)"#
            ))
            .bind(c.name)
            .bind(c.code)
            .bind(c.symbol)
            .execute(pool)
            .await
            .map_err(AliothError::from)?;
            inserted_currencies += 1;
        }
    }

    let rate_count: (i64,) = sqlx::query_as(AssertSqlSafe(
        "SELECT COUNT(*) FROM isahl.\"zc_id_rate-exchange\" WHERE deleted_at IS NULL",
    ))
    .fetch_one(pool)
    .await
    .map_err(AliothError::from)?;

    let mut inserted_rates = 0usize;
    if rate_count.0 == 0 {
        let rows: Vec<(i64, String)> = sqlx::query_as(AssertSqlSafe(
            "SELECT id, code FROM isahl.\"zc_id_unit-currency\" WHERE deleted_at IS NULL",
        ))
        .fetch_all(pool)
        .await
        .map_err(AliothError::from)?;
        let id_by_code: std::collections::HashMap<String, i64> =
            rows.into_iter().map(|(id, code)| (code, id)).collect();

        for r in all_seed_exchange_rates() {
            let Some(left_id) = id_by_code.get(r.left).copied() else {
                continue;
            };
            let Some(right_id) = id_by_code.get(r.right).copied() else {
                continue;
            };
            let bid = Decimal::from_str(r.bid).unwrap_or(Decimal::ZERO);
            let ask = Decimal::from_str(r.ask).unwrap_or(Decimal::ZERO);
            sqlx::query(AssertSqlSafe(
                r#"INSERT INTO isahl."zc_id_rate-exchange" (notice, ck_left, ck_right, multiply, division, code, date, created_by_id)
                   VALUES ($1, $2, $3, $4, $5, $6, NOW(), 0)"#,
            ))
            .bind(r.name)
            .bind(left_id)
            .bind(right_id)
            .bind(bid)
            .bind(ask)
            .bind(r.source)
            .execute(pool)
            .await
            .map_err(AliothError::from)?;
            inserted_rates += 1;
        }
    }

    Ok((inserted_currencies, inserted_rates))
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::testing::{connect_test_db, setup_test_schema_light};

    /// 静态句柄 ↔ Framework 路由一致性（防两处漂移）：登记量纲的叶表必须与
    /// `measurement::biz_repositories::{unit,rate}_leaf_table_for_dimension` 同口径；
    /// 且所有种子量纲均已登记（未登记会落 `PARENT_DIMENSION_SEED_SQL` 回落）。
    #[test]
    fn seed_sql_routes_to_framework_leafs() {
        use measurement::biz_repositories::{
            rate_leaf_table_for_dimension as rate_leaf, unit_leaf_table_for_dimension as unit_leaf,
        };
        for (dim, sql) in DIMENSION_SEED_SQLS {
            assert!(
                sql.unit_count_sql.contains(unit_leaf(dim)),
                "{dim}: unit 叶表与 Framework 路由不一致（静态 {:?} vs 框架 {:?}）",
                sql.unit_count_sql,
                unit_leaf(dim)
            );
            assert_eq!(
                rate_leaf(dim),
                format!("isahl.\"{}\"", sql.rate_leaf_name),
                "{dim}: rate 叶表与 Framework 路由不一致"
            );
        }
        for u in all_seed_units() {
            assert!(
                DIMENSION_SEED_SQLS.iter().any(|(k, _)| *k == u.dimension),
                "量纲 {} 未登记静态句柄（会落父表回落，与 Framework 路由漂移）",
                u.dimension
            );
        }
    }

    #[tokio::test]
    async fn test_seed_standard_units_idempotent() {
        let pool = connect_test_db().await;
        setup_test_schema_light(&pool).await.unwrap();
        // 共享测试库：单位族（zc_id_unit*）承载**模型级种子**（Framework/seed/seed-dimensions.sql）
        // 与其他域消费（SE 预算等）——MUST NOT 通配清空（曾致全族 319→2，跨 ns 用例连锁失败）。
        // 幂等断言改为「不依赖空起点 + 域内在册」：二次运行必须零新增、且域内确有行。
        let first = seed_standard_units(&pool).await.unwrap();
        let second = seed_standard_units(&pool).await.unwrap();
        assert_eq!(second, 0, "should be idempotent on second run");
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM isahl.zc_id_unit")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            total > 0,
            "单位域应非空（模型级种子或本次 seed 供给），首次新增 {} 行",
            first
        );
    }
}
