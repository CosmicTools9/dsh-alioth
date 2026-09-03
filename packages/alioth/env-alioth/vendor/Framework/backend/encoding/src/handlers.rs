use actix_web::{web, HttpResponse};
use common::{AliothError, ApiResponse};
use sqlx::PgPool;

use crate::{
    models::*,
    service::EncodingService,
    zuid::{PeerType, ZuidGenerator},
};

async fn generate_zuid(
    pool: web::Data<PgPool>,
    req: web::Json<GenerateZuidRequest>,
) -> Result<HttpResponse, AliothError> {
    let service = build_service(&pool, &req).map_err(|e| AliothError::BadRequest(e.to_string()))?;

    let zuid = service.generate_zuid();
    let zuid_u64 = service.generate_zuid_u64();

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(GenerateZuidResponse {
            zuid,
            zuid_u64,
        })),
    )
}

fn build_service(
    pool: &PgPool,
    req: &GenerateZuidRequest,
) -> Result<EncodingService, crate::service::EncodingServiceError> {
    match (req.peer_type, req.idc, req.cluster, req.node) {
        (Some(pt), Some(idc), Some(cluster), Some(node)) => {
            let peer_type =
                PeerType::from_u8(pt).ok_or(crate::zuid::ZuidError::InvalidPeerType(pt))?;
            EncodingService::with_zuid(pool.clone(), peer_type, idc, cluster, node)
        }
        _ => EncodingService::new(pool.clone()),
    }
}

async fn extract_zuid(req: web::Json<ExtractZuidRequest>) -> Result<HttpResponse, AliothError> {
    let id = req.zuid;
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(ExtractZuidResponse {
            peer_type: ZuidGenerator::extract_peer_type(id).map(|p| p as u8),
            idc: ZuidGenerator::extract_idc(id),
            cluster: ZuidGenerator::extract_cluster(id),
            node: ZuidGenerator::extract_node(id),
            timestamp: ZuidGenerator::extract_timestamp(id),
            sequence: ZuidGenerator::extract_sequence(id),
        })),
    )
}

async fn generate_serial(
    pool: web::Data<PgPool>,
    req: web::Json<GenerateSerialRequest>,
) -> Result<HttpResponse, AliothError> {
    let service = EncodingService::new(pool.get_ref().clone())
        .map_err(|e| AliothError::Internal(e.to_string()))?;

    let pad_char = req.pad_char.chars().next().unwrap_or('0');
    let serial = service
        .generate_serial(&req.sequence_name, req.width, pad_char)
        .await
        .map_err(|e| AliothError::BadRequest(e.to_string()))?;

    Ok(HttpResponse::Ok().json(ApiResponse::success(GenerateSerialResponse { serial })))
}

async fn compute_crc32(req: web::Json<ComputeCrc32Request>) -> Result<HttpResponse, AliothError> {
    let data = req.data.as_bytes();
    let checksum = crate::crc32::compute_checksum(data);
    let checksum_hex = crate::crc32::compute_checksum_hex(data);
    Ok(
        HttpResponse::Ok().json(ApiResponse::success(ComputeCrc32Response {
            checksum,
            checksum_hex,
        })),
    )
}

async fn validate_crc32(req: web::Json<ValidateCrc32Request>) -> Result<HttpResponse, AliothError> {
    let data = req.data.as_bytes();
    let valid = crate::crc32::validate_checksum(data, req.checksum);
    Ok(HttpResponse::Ok().json(ApiResponse::success(ValidateCrc32Response { valid })))
}

async fn apply_rule(
    pool: web::Data<PgPool>,
    req: web::Json<ApplyRuleRequest>,
) -> Result<HttpResponse, AliothError> {
    let service = EncodingService::new(pool.get_ref().clone())
        .map_err(|e| AliothError::Internal(e.to_string()))?;

    let result = if req.use_db_sequences {
        service
            .apply_rule_with_sequences(&req.rule)
            .await
            .map_err(|e| AliothError::BadRequest(e.to_string()))?
    } else {
        service
            .apply_rule(&req.rule, &crate::rules::EncodingContext::default())
            .map_err(|e| AliothError::BadRequest(e.to_string()))?
    };

    Ok(
        HttpResponse::Ok().json(ApiResponse::success(ApplyRuleResponse {
            code: result.code,
            checksum: result.checksum,
            segments: result.segments,
        })),
    )
}

pub fn configure_encoding_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/encoding/zuid").route(web::post().to(generate_zuid)))
        .service(web::resource("/encoding/zuid/extract").route(web::post().to(extract_zuid)))
        .service(web::resource("/encoding/serial").route(web::post().to(generate_serial)))
        .service(web::resource("/encoding/crc32").route(web::post().to(compute_crc32)))
        .service(web::resource("/encoding/crc32/validate").route(web::post().to(validate_crc32)))
        .service(web::resource("/encoding/apply-rule").route(web::post().to(apply_rule)));
}
