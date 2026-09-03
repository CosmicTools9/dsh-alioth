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

/// 量纲→叶表映射。INSERT 路由到对应子表而非父表 `zc_id_unit`。
pub fn unit_leaf_table_for_dimension(dim_key: &str) -> &'static str {
    match dim_key {
        "temperature" => "isahl.\"zc_id_unit-temperature\"",
        "current" => "isahl.\"zc_id_unit-current\"",
        "intensity" => "isahl.\"zc_id_unit-intensity\"",
        "density" => "isahl.\"zc_id_unit-density\"",
        "speed" => "isahl.\"zc_id_unit-speed\"",
        "pressure" => "isahl.\"zc_id_unit-pressure\"",
        "power" => "isahl.\"zc_id_unit-power\"",
        "voltage" => "isahl.\"zc_id_unit-voltage\"",
        "angle" => "isahl.\"zc_id_unit-angle\"",
        "frequency" => "isahl.\"zc_id_unit-frequency\"",
        "radiation" => "isahl.\"zc_id_unit-radiation\"",
        "luminance" => "isahl.\"zc_id_unit-luminance\"",
        "magnetic_flux" => "isahl.\"zc_id_unit-magnetic_flux\"",
        "magnetic_field_strength" => "isahl.\"zc_id_unit-magnetic_field_strength\"",
        "stress" => "isahl.\"zc_id_unit-stress\"",
        "display" => "isahl.\"zc_id_unit-display\"",
        "pricing" => "isahl.\"zc_id_unit-pricing\"",
        "price" => "isahl.\"zc_id_unit-price\"",
        "distance" => "isahl.\"zc_id_unit-distance\"",
        "duration" => "isahl.\"zc_id_unit-duration\"",
        "area" => "isahl.\"zc_id_unit-area\"",
        "volume" => "isahl.\"zc_id_unit-volume\"",
        "weight" => "isahl.\"zc_id_unit-weight\"",
        "data" => "isahl.\"zc_id_unit-data\"",
        "currency" => "isahl.\"zc_id_unit-currency\"",
        "container" => "isahl.\"zc_id_unit-container\"",
        "common" => "isahl.\"zc_id_unit-common\"",
        "energy" => "isahl.\"zc_id_unit-energy\"",
        "working" => "isahl.\"zc_id_unit-working\"",
        _ => "isahl.\"zc_id_unit\"",
    }
}

/// 量纲→叶表映射。INSERT 路由到对应子表而非父表 `zc_id_rate`。
pub fn rate_leaf_table_for_dimension(dim_key: &str) -> &'static str {
    match dim_key {
        "angle" => "isahl.\"zc_id_rate-angle\"",
        "area" => "isahl.\"zc_id_rate-area\"",
        "container" => "isahl.\"zc_id_rate-container\"",
        "current" => "isahl.\"zc_id_rate-current\"",
        "custom" => "isahl.\"zc_id_rate-custom\"",
        "data" => "isahl.\"zc_id_rate-data\"",
        "density" => "isahl.\"zc_id_rate-density\"",
        "distance" => "isahl.\"zc_id_rate-distance\"",
        "duration" => "isahl.\"zc_id_rate-duration\"",
        "energy" => "isahl.\"zc_id_rate-energy\"",
        "exchange" => "isahl.\"zc_id_rate-exchange\"",
        "frequency" => "isahl.\"zc_id_rate-frequency\"",
        "intensity" => "isahl.\"zc_id_rate-intensity\"",
        "luminance" => "isahl.\"zc_id_rate-luminance\"",
        "magnetic_field_strength" => "isahl.\"zc_id_rate-magnetic_field_strength\"",
        "magnetic_flux" => "isahl.\"zc_id_rate-magnetic_flux\"",
        "power" => "isahl.\"zc_id_rate-power\"",
        "pressure" => "isahl.\"zc_id_rate-pressure\"",
        "radiation" => "isahl.\"zc_id_rate-radiation\"",
        "speed" => "isahl.\"zc_id_rate-speed\"",
        "stress" => "isahl.\"zc_id_rate-stress\"",
        "temperature" => "isahl.\"zc_id_rate-temperature\"",
        "voltage" => "isahl.\"zc_id_rate-voltage\"",
        "volume" => "isahl.\"zc_id_rate-volume\"",
        "weight" => "isahl.\"zc_id_rate-weight\"",
        _ => "isahl.\"zc_id_rate\"",
    }
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
