//! NGAC 图快照聚合（refactor-ngac-admin-nl-graph）。
//!
//! 唯一实现：HTTP 端点 `GET /api/admin/ngac/graph`（admin/handlers/ngac.rs）与
//! Gateway nl-assist（进程内调用，design D2）共用本模块。返回策略图全量：
//! 版本、策略类、UA（含持有者与认知派生标记）、OA（含展示名与模块归属）、
//! association/prohibition、access right 目录。
//!
//! 数据源复用：认知持有者 = `common::ngac_org::cognition_derived_holders_batch`（唯一实现）；
//! OA 展示名 = `display::meta_display_names` + `resolve_display_name`（§2.2.2 解析链）。

use std::collections::HashMap;

use serde::Serialize;
use sqlx::PgPool;

/// 持有者名单截断上限（design D7：holders top-20，`holder_count` 为全量计数）。
const HOLDERS_CAP: usize = 20;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct GraphPolicyClass {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    pub description: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct GraphUserAttributeRow {
    id: i64,
    o_name: String,
    fk_policy_class: Option<i64>,
    ancestor_ids: Vec<i64>,
    property: serde_json::Value,
}

/// UA 节点：`derived_from='cognition'` 即本体认知派生（position:/view:）。
#[derive(Debug, Serialize)]
pub struct GraphUserAttribute {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
    pub description: Option<String>,
    pub derived_from: Option<String>,
    /// 有效持有者总数（直接指派 ∪ 认知派生）。
    pub holder_count: i64,
    /// 持有者 username 截断 top-20。
    pub holders: Vec<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct GraphObjectAttributeRow {
    id: i64,
    o_name: String,
    fk_policy_class: Option<i64>,
    resource_type: Option<String>,
    fk_resource: Option<i64>,
    resource_identifier: Option<String>,
    ancestor_ids: Vec<i64>,
}

/// OA 节点：display_name/module_* 为读侧派生（§2.2.2 解析链），与 admin 列表端点同源。
#[derive(Debug, Serialize)]
pub struct GraphObjectAttribute {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub resource_type: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_resource: Option<i64>,
    pub resource_identifier: Option<String>,
    pub display_name: String,
    /// 页面预览（add-ngac-oa-preview，dev-only）：由 HTTP handler 用
    /// `display::load_preview_manifest` 合并；进程内调用方（nl-assist）为 null。
    pub preview: Option<super::display::OaPreviewInfo>,
    pub module_name: Option<String>,
    pub module_route: Option<String>,
    pub namespace: Option<String>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct GraphAssociation {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_user_attribute: Option<i64>,
    pub user_attribute: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_object_attribute: Option<i64>,
    pub object_attribute: Option<String>,
    pub resource_type: Option<String>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ak_access_rights: Vec<i64>,
    pub access_rights: Vec<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    pub conditions: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct GraphProhibition {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_user_attribute: Option<i64>,
    pub user_attribute: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_object_attribute: Option<i64>,
    pub object_attribute: Option<String>,
    pub resource_type: Option<String>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ak_access_rights: Vec<i64>,
    pub access_rights: Vec<String>,
    pub is_active: bool,
    pub conditions: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct GraphAccessRight {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    pub description: Option<String>,
    pub applicable_types: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct GraphSnapshot {
    /// `ngac_policy_version` 当前版本（提案-确认的陈旧检测用）。
    pub version: i64,
    pub policy_classes: Vec<GraphPolicyClass>,
    pub user_attributes: Vec<GraphUserAttribute>,
    pub object_attributes: Vec<GraphObjectAttribute>,
    pub associations: Vec<GraphAssociation>,
    pub prohibitions: Vec<GraphProhibition>,
    pub access_rights: Vec<GraphAccessRight>,
}

/// 聚合图快照（唯一实现）。顺序查询单连接；任一查询失败整体失败（fail-closed，
/// 不返回部分快照——半图比无图更危险）。
pub async fn graph_snapshot(pool: &PgPool) -> sqlx::Result<GraphSnapshot> {
    let version = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(version), 0) FROM isahl_auth.ngac_policy_version",
    )
    .fetch_one(pool)
    .await?;

    let policy_classes = sqlx::query_as::<_, GraphPolicyClass>(
        r#"
        SELECT id, o_name, description, is_active
        FROM isahl_auth.ngac_policy_class
        ORDER BY o_name
        "#,
    )
    .fetch_all(pool)
    .await?;

    // ── UA 基础行 + 持有者（直接指派 + 认知派生批量反解）──
    let ua_rows = sqlx::query_as::<_, GraphUserAttributeRow>(
        r#"
        SELECT id, o_name, fk_policy_class,
               COALESCE(ancestor_ids, '{}') AS ancestor_ids,
               property
        FROM isahl_auth.ngac_user_attribute
        WHERE deleted_at IS NULL
        ORDER BY o_name
        "#,
    )
    .fetch_all(pool)
    .await?;

    // (ua_id → (count, usernames))，直接指派 + 认知派生合并计入。
    let mut holder_map: HashMap<i64, (i64, Vec<String>)> = HashMap::new();
    let direct_rows = sqlx::query_as::<_, (i64, i64, Option<String>)>(
        r#"
        SELECT ura.fk_user_attribute, COUNT(*) AS cnt, u.username
        FROM isahl_auth.ngac_user_rr_attribute ura
        JOIN isahl_auth.auth_users u ON u.id = ura.fk_user
        WHERE ura.deleted_at IS NULL
          AND (ura.expires_at IS NULL OR ura.expires_at > NOW())
        GROUP BY ura.fk_user_attribute, u.username
        ORDER BY ura.fk_user_attribute, u.username NULLS LAST
        "#,
    )
    .fetch_all(pool)
    .await?;
    for (ua_id, cnt, username) in direct_rows {
        let entry = holder_map.entry(ua_id).or_insert((0, Vec::new()));
        entry.0 += cnt;
        if let Some(name) = username {
            if !entry.1.contains(&name) {
                entry.1.push(name);
            }
        }
    }
    // 认知派生持有者：o_name → UA id 对齐后并入
    let cognition_rows = common::ngac_org::cognition_derived_holders_batch(pool).await?;
    let ua_by_name: HashMap<&str, i64> =
        ua_rows.iter().map(|r| (r.o_name.as_str(), r.id)).collect();
    for (o_name, _user_id, username) in cognition_rows {
        if let Some(&ua_id) = ua_by_name.get(o_name.as_str()) {
            let entry = holder_map.entry(ua_id).or_insert((0, Vec::new()));
            entry.0 += 1;
            if let Some(name) = username {
                if !entry.1.contains(&name) {
                    entry.1.push(name);
                }
            }
        }
    }

    let user_attributes = ua_rows
        .into_iter()
        .map(|r| {
            let derived_from = r
                .property
                .get("derived_from")
                .and_then(|v| v.as_str())
                .map(String::from);
            // description 与 derived_from 同源：property JSONB（admin 列表端点
            // property->>'description' 语义，change refactor-ngac-admin-nl-graph 对齐）
            let description = r
                .property
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from);
            let (holder_count, holders) = holder_map
                .get(&r.id)
                .map(|(cnt, names)| (*cnt, names.clone()))
                .unwrap_or((0, Vec::new()));
            let mut holders = holders;
            holders.sort();
            holders.truncate(HOLDERS_CAP);
            GraphUserAttribute {
                id: r.id,
                o_name: r.o_name,
                fk_policy_class: r.fk_policy_class,
                ancestor_ids: r.ancestor_ids,
                description,
                derived_from,
                holder_count,
                holders,
            }
        })
        .collect();

    // ── OA 基础行 + 展示名/模块归属（display.rs 同源解析链）──
    let oa_rows = sqlx::query_as::<_, GraphObjectAttributeRow>(
        r#"
        SELECT id, o_name, fk_policy_class, resource_type, fk_resource,
               resource_identifier,
               COALESCE(ancestor_ids, '{}') AS ancestor_ids
        FROM isahl_auth.ngac_object_attribute
        WHERE deleted_at IS NULL
        ORDER BY resource_type, o_name
        "#,
    )
    .fetch_all(pool)
    .await?;
    let types: std::collections::HashSet<String> = oa_rows
        .iter()
        .filter_map(|r| r.resource_type.clone())
        .filter(|t| !t.is_empty())
        .collect();
    let meta_names = super::display::meta_display_names(pool, &types).await;
    let object_attributes = oa_rows
        .into_iter()
        .map(|r| {
            let rt = r.resource_type.as_deref().unwrap_or("");
            let display_name = super::display::resolve_display_name(
                r.fk_resource,
                r.resource_identifier.as_deref(),
                rt,
                &meta_names,
                &r.o_name,
            );
            let (module_name, module_route, namespace) = super::display::module_fields(rt);
            GraphObjectAttribute {
                id: r.id,
                o_name: r.o_name,
                fk_policy_class: r.fk_policy_class,
                resource_type: r.resource_type,
                fk_resource: r.fk_resource,
                resource_identifier: r.resource_identifier,
                display_name,
                preview: None,
                module_name: module_name.map(String::from),
                module_route: module_route.map(String::from),
                namespace: namespace.map(String::from),
                ancestor_ids: r.ancestor_ids,
            }
        })
        .collect();

    let associations = sqlx::query_as::<_, GraphAssociation>(
        r#"
        SELECT a.id, a.o_name, a.fk_user_attribute, ua.o_name AS user_attribute,
               a.fk_object_attribute, oa.o_name AS object_attribute, oa.resource_type,
               a.ak_access_rights,
               COALESCE(ARRAY(
                   SELECT ar.o_name FROM isahl_auth.ngac_access_right ar
                   WHERE ar.id = ANY(a.ak_access_rights)
               ), '{}') AS access_rights,
               a.fk_policy_class, a.conditions
        FROM isahl_auth.ngac_association a
        LEFT JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute
        LEFT JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute
        WHERE a.deleted_at IS NULL
        ORDER BY a.id
        "#,
    )
    .fetch_all(pool)
    .await?;

    let prohibitions = sqlx::query_as::<_, GraphProhibition>(
        r#"
        SELECT p.id, p.o_name, p.fk_user_attribute, ua.o_name AS user_attribute,
               p.fk_object_attribute, oa.o_name AS object_attribute, oa.resource_type,
               p.ak_access_rights,
               COALESCE(ARRAY(
                   SELECT ar.o_name FROM isahl_auth.ngac_access_right ar
                   WHERE ar.id = ANY(p.ak_access_rights)
               ), '{}') AS access_rights,
               p.is_active, p.conditions
        FROM isahl_auth.ngac_prohibition p
        LEFT JOIN isahl_auth.ngac_user_attribute ua ON ua.id = p.fk_user_attribute
        LEFT JOIN isahl_auth.ngac_object_attribute oa ON oa.id = p.fk_object_attribute
        WHERE p.deleted_at IS NULL
        ORDER BY p.id
        "#,
    )
    .fetch_all(pool)
    .await?;

    let access_rights = sqlx::query_as::<_, GraphAccessRight>(
        r#"
        SELECT id, o_name, description, applicable_types
        FROM isahl_auth.ngac_access_right
        ORDER BY o_name
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(GraphSnapshot {
        version,
        policy_classes,
        user_attributes,
        object_attributes,
        associations,
        prohibitions,
        access_rights,
    })
}
