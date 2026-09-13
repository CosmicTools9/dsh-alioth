// ontology_binding（split 自 repository.rs 内嵌模块，④ 候选）
use common::AliothError as ApiError;
use sqlx::PgPool;

/// (scene_code, factor_code, function_code)
pub type Coords = (&'static str, &'static str, &'static str);

pub fn coords_for_entity(entity: &str) -> Result<Coords, ApiError> {
    match entity {
        "Identity" => Ok(("JE", "FJA", "↑_DA")),
        "Environment" => Ok(("JC", "GEC", "↑_DA")),
        "License" => Ok(("JC", "GID", "↑_DA")),
        "Consignment" => Ok(("GC", "FJA", "↓_GG")),
        "Vehicle" => Ok(("GC", "FJA", "↓_GG")),
        "NaturalPerson" => Ok(("GC", "FJA", "↓_GG")),
        "TransportTracking" => Ok(("GC", "FJA", "↓_GG")),
        "TradeOrder" => Ok(("GC", "FJA", "↓_GG")),
        "BillCheck" => Ok(("GC", "FJA", "↓_GD")),
        "DetaBillCheck" => Ok(("GC", "FJA", "↓_GD")),
        "Invoice" => Ok(("GC", "FJA", "↓_GD")),
        "InvoiceDetail" => Ok(("GC", "FJA", "↓_GD")),
        "Payment" => Ok(("TX", "FJA", "↓_EV")),
        "SettlementBank" => Ok(("TX", "FJA", "↓_EV")),
        "SettlementCash" => Ok(("TX", "FJA", "↓_EV")),
        "SettlementChannel" => Ok(("TX", "FJA", "↓_EV")),
        "TrafficLine" => Ok(("GC", "FJA", "↑_GG")),
        "FreightProduct" => Ok(("GC", "FJA", "↓_GG")),
        "PricingAgreement" => Ok(("GC", "FJA", "↓_GG")),
        "Contract" => Ok(("GC", "FJA", "↓_GG")),
        // 铅封叶表：与 transport-operations 三单录入同款三元组（TX/FJA/↓_GG，
        // transport_ops.rs insert_three_info 与 wz-outgo-waybill-enrich.sql 一致）
        "Seal" => Ok(("TX", "FJA", "↓_GG")),
        // strengthen-identity-org 主体域叶表（scene/factor/function 均经 DB 维度表核实）：
        // ZB=组织架构 ZJ=行政人事 ZH=主体管理 UB=主权管理；LNC=权责主体 LNK=经营主体；
        // ↓_DA=权责复检 ↓_EH=人事劳动
        "SubjectGroup" => Ok(("ZB", "LNC", "↓_DA")),
        "SubjectEmployee" => Ok(("ZJ", "LNC", "↓_EH")),
        "EmploymentAgent" => Ok(("ZJ", "LNC", "↓_EH")),
        "SubjectCountry" => Ok(("UB", "LNC", "↓_DA")),
        "SubjectBank" => Ok(("ZH", "LNK", "↓_DA")),
        "SubjectMinistry" => Ok(("UB", "LNC", "↓_DA")),
        "SubjectSovereign" => Ok(("UB", "LNC", "↓_DA")),
        "SubjectSupranational" => Ok(("UB", "LNC", "↓_DA")),
        _ => Err(ApiError::Internal(format!(
            "no ontology coordinates for entity: {entity}"
        ))),
    }
}

pub async fn resolve(pool: &PgPool, entity: &str) -> Result<(i64, i64, Option<i64>), ApiError> {
    let coords = coords_for_entity(entity)?;
    let (scene_id, factor_id, function_id) = ontology_binding::resolve(pool, coords)
        .await
        .map_err(|e| ApiError::Internal(format!("resolve {:?}: {e}", coords)))?;
    let scene_id =
        scene_id.ok_or_else(|| ApiError::Internal(format!("resolve scene {}", coords.0)))?;
    let factor_id =
        factor_id.ok_or_else(|| ApiError::Internal(format!("resolve factor {}", coords.1)))?;
    Ok((scene_id, factor_id, function_id))
}
