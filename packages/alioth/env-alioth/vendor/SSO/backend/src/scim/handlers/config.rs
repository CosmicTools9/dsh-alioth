//! SCIM 静态配置端点（Discovery）：ServiceProviderConfig / Schemas / ResourceTypes。

use actix_web::HttpResponse;
use serde_json::json;

// ── Discovery 端点 ────────────────────────────────────────────────────────────

/// GET /scim/v2/ServiceProviderConfig
pub async fn get_service_provider_config() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
        "documentationUri": "https://example.com/scim/docs",
        "patch": { "supported": true },
        "bulk": { "supported": false, "maxOperations": 0, "maxPayloadSize": 0 },
        "filter": { "supported": true, "maxResults": 200 },
        "changePassword": { "supported": false },
        "sort": { "supported": false },
        "etag": { "supported": false },
        "authenticationSchemes": [{
            "name": "Bearer Token",
            "description": "Static bearer token in Authorization header",
            "specUri": "http://www.rfc-editor.org/info/rfc6750",
            "documentationUri": "https://example.com/scim/auth",
            "type": "oauthbearertoken",
            "primary": true
        }],
        "meta": {
            "resourceType": "ServiceProviderConfig",
            "location": "/scim/v2/ServiceProviderConfig"
        }
    }))
}

/// GET /scim/v2/Schemas
pub async fn get_schemas() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0:ListResponse"],
        "totalResults": 2,
        "startIndex": 1,
        "itemsPerPage": 2,
        "Resources": [
            {
                "id": "urn:ietf:params:scim:schemas:core:2.0:User",
                "name": "User",
                "description": "User Account",
                "attributes": [
                    { "name": "userName", "type": "string", "required": true, "multiValued": false },
                    { "name": "name", "type": "complex", "multiValued": false },
                    { "name": "emails", "type": "complex", "multiValued": true },
                    { "name": "active", "type": "boolean", "multiValued": false },
                    { "name": "displayName", "type": "string", "multiValued": false }
                ],
                "meta": { "resourceType": "Schema", "location": "/scim/v2/Schemas/urn:ietf:params:scim:schemas:core:2.0:User" }
            },
            {
                "id": "urn:ietf:params:scim:schemas:core:2.0:Group",
                "name": "Group",
                "description": "Group",
                "attributes": [
                    { "name": "displayName", "type": "string", "required": true, "multiValued": false },
                    { "name": "members", "type": "complex", "multiValued": true }
                ],
                "meta": { "resourceType": "Schema", "location": "/scim/v2/Schemas/urn:ietf:params:scim:schemas:core:2.0:Group" }
            }
        ]
    }))
}

/// GET /scim/v2/ResourceTypes
pub async fn get_resource_types() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0:ListResponse"],
        "totalResults": 2,
        "startIndex": 1,
        "itemsPerPage": 2,
        "Resources": [
            {
                "id": "User",
                "name": "User",
                "endpoint": "/scim/v2/Users",
                "schema": "urn:ietf:params:scim:schemas:core:2.0:User",
                "meta": { "resourceType": "ResourceType", "location": "/scim/v2/ResourceTypes/User" }
            },
            {
                "id": "Group",
                "name": "Group",
                "endpoint": "/scim/v2/Groups",
                "schema": "urn:ietf:params:scim:schemas:core:2.0:Group",
                "meta": { "resourceType": "ResourceType", "location": "/scim/v2/ResourceTypes/Group" }
            }
        ]
    }))
}
