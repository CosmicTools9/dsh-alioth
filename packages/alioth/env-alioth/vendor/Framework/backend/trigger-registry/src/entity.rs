//! Entity Trigger Templates
//!
//! AFTER 触发器：当 fk_user 从无到有被设置后，自动创建默认通讯联络关系
//!（contact → info）。所有 SQL 委托 TemplateEngine。

use crate::{
    template::{
        TemplateEngine, TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef,
    },
    utils::*,
    SideEffect, TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// Entity 默认联络关系创建模板
///
/// 在 `fk_user` 首次被设置后，自动：
/// 1. 创建默认 `zc_id_contacts`
/// 2. 建立 `zc_id_entity_rr_contacts` 关联
/// 3. 创建默认 `zc_id_info-isahl`（站内信）
/// 4. 建立 `zc_id_contacts_rr_infos` 关联
pub struct EntityDefaultTemplate;

#[async_trait]
impl TriggerTemplate for EntityDefaultTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_af_ups_72_on_zc_id_entity".to_string(),
            applies_to: vec!["zc_id_entity".to_string()],
            operations: vec![TriggerOperationDef::Insert, TriggerOperationDef::Update],
            timing: TriggerTimingDef::After,
        }
    }

    async fn execute(
        &self,
        ctx: &TriggerContext,
        old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;

        let fk_user: Option<i64> = get_field(new, "fk_user");
        if fk_user.is_none() {
            return Ok(TriggerResult::new());
        }

        // Only run when fk_user was just set (old is None or different)
        let old_fk_user: Option<i64> = old_record.and_then(|r| get_field(r, "fk_user"));
        if old_fk_user.is_some() {
            return Ok(TriggerResult::new());
        }

        let entity_id: i64 = get_field(new, "id").unwrap_or(0);
        if entity_id == 0 {
            return Ok(TriggerResult::new());
        }

        let notice: Option<String> = get_field(new, "notice");
        let created_by_id: Option<i64> = get_field(new, "created_by_id");
        let updated_by_id: Option<i64> = get_field(new, "updated_by_id");
        let created_at: Option<chrono::DateTime<chrono::Utc>> = get_field(new, "created_at");
        let updated_at: Option<chrono::DateTime<chrono::Utc>> = get_field(new, "updated_at");

        // No pool → emit side effects for the executor to apply later
        if ctx.pool.is_none() {
            return Ok(fallback_side_effects(entity_id, notice));
        }

        let engine = TemplateEngine::new(ctx.pool.clone());

        // 1. Check if entity already has a default contact relation
        let exists: bool = engine
            .query_scalar(
                "SELECT EXISTS(SELECT 1 FROM isahl.zc_id_entity_rr_contacts WHERE ref_left = $1)",
                vec![Value::Number(entity_id.into())],
            )
            .await?
            .unwrap_or(false);

        if exists {
            return Ok(TriggerResult::new());
        }

        // 2. Query dimension IDs
        let dk_scene_id: Option<i64> = engine
            .query_scalar(
                "SELECT id FROM isahl.zc_id_scene WHERE notice = '通讯联络' LIMIT 1",
                vec![],
            )
            .await?;
        let dk_factor_id: Option<i64> = engine
            .query_scalar(
                "SELECT id FROM isahl.zc_id_factor WHERE notice = '通讯主体' LIMIT 1",
                vec![],
            )
            .await?;
        let dk_function_id: Option<i64> = engine
            .query_scalar(
                "SELECT id FROM isahl.zc_id_function WHERE notice = '通讯联系' LIMIT 1",
                vec![],
            )
            .await?;

        // 3. Insert default contact
        let binds_contact = vec![
            created_by_id
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
            updated_by_id
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
            created_at
                .map(|v| Value::String(v.to_rfc3339()))
                .unwrap_or(Value::Null),
            updated_at
                .map(|v| Value::String(v.to_rfc3339()))
                .unwrap_or(Value::Null),
            dk_scene_id
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
            dk_factor_id
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
            dk_function_id
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
        ];
        let contact_id: i64 = engine
            .query_scalar(
                r#"
                INSERT INTO isahl.zc_id_contacts (
                    created_by_id, updated_by_id, created_at, updated_at,
                    dk_scene, dk_factor, dk_function
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (number) DO UPDATE SET
                    updated_by_id = EXCLUDED.updated_by_id,
                    updated_at = EXCLUDED.updated_at
                RETURNING id
                "#,
                binds_contact,
            )
            .await?
            .ok_or_else(|| {
                TriggerError::ExecutionFailed("Contact insert returned no id".to_string())
            })?;

        // 4. Insert entity → contact relation
        engine
            .execute(
                r#"
                INSERT INTO isahl.zc_id_entity_rr_contacts (
                    ref_left, ref_right, default_contact, default_entity,
                    created_by_id, updated_by_id, created_at, updated_at
                )
                VALUES ($1, $2, TRUE, TRUE, $3, $4, $5, $6)
                ON CONFLICT (r_number) DO UPDATE SET
                    updated_by_id = EXCLUDED.updated_by_id,
                    updated_at = EXCLUDED.updated_at,
                    default_contact = TRUE,
                    default_entity = TRUE
                "#,
                vec![
                    Value::Number(entity_id.into()),
                    Value::Number(contact_id.into()),
                    created_by_id
                        .map(|v| Value::Number(v.into()))
                        .unwrap_or(Value::Null),
                    updated_by_id
                        .map(|v| Value::Number(v.into()))
                        .unwrap_or(Value::Null),
                    created_at
                        .map(|v| Value::String(v.to_rfc3339()))
                        .unwrap_or(Value::Null),
                    updated_at
                        .map(|v| Value::String(v.to_rfc3339()))
                        .unwrap_or(Value::Null),
                ],
            )
            .await?;

        // 5. Check if contact already has a default info relation
        let info_exists: bool = engine
            .query_scalar(
                "SELECT EXISTS(SELECT 1 FROM isahl.zc_id_contacts_rr_infos WHERE ref_left = $1)",
                vec![Value::Number(contact_id.into())],
            )
            .await?
            .unwrap_or(false);

        if !info_exists {
            // 6. Query dimension IDs for info
            let info_scene_id: Option<i64> = engine
                .query_scalar(
                    "SELECT id FROM isahl.zc_id_scene WHERE notice = '场所标识' LIMIT 1",
                    vec![],
                )
                .await?;
            let info_factor_id: Option<i64> = engine
                .query_scalar(
                    "SELECT id FROM isahl.zc_id_factor WHERE notice = '消息账号' LIMIT 1",
                    vec![],
                )
                .await?;
            let info_function_id: Option<i64> = engine
                .query_scalar(
                    "SELECT id FROM isahl.zc_id_function WHERE notice = '通讯联系' LIMIT 1",
                    vec![],
                )
                .await?;

            // 7. Insert default info (站内信)
            let _info_notice = format!(
                "alioth_{}",
                crc32_hex(notice.as_ref().unwrap_or(&String::new()))
            );
            let info_id: i64 = engine
                .query_scalar(
                    r#"
                    INSERT INTO isahl."zc_id_info-isahl" (
                        created_by_id, updated_by_id, created_at, updated_at,
                        dk_scene, dk_factor, dk_function
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, $7)
                    ON CONFLICT (notice) DO UPDATE SET
                        updated_by_id = EXCLUDED.updated_by_id,
                        updated_at = EXCLUDED.updated_at
                    RETURNING id
                    "#,
                    vec![
                        created_by_id
                            .map(|v| Value::Number(v.into()))
                            .unwrap_or(Value::Null),
                        updated_by_id
                            .map(|v| Value::Number(v.into()))
                            .unwrap_or(Value::Null),
                        created_at
                            .map(|v| Value::String(v.to_rfc3339()))
                            .unwrap_or(Value::Null),
                        updated_at
                            .map(|v| Value::String(v.to_rfc3339()))
                            .unwrap_or(Value::Null),
                        info_scene_id
                            .map(|v| Value::Number(v.into()))
                            .unwrap_or(Value::Null),
                        info_factor_id
                            .map(|v| Value::Number(v.into()))
                            .unwrap_or(Value::Null),
                        info_function_id
                            .map(|v| Value::Number(v.into()))
                            .unwrap_or(Value::Null),
                    ],
                )
                .await?
                .ok_or_else(|| {
                    TriggerError::ExecutionFailed("Info insert returned no id".to_string())
                })?;

            // 8. Insert contact → info relation
            engine
                .execute(
                    r#"
                    INSERT INTO isahl.zc_id_contacts_rr_infos (
                        ref_left, ref_right, default_info, default_contact,
                        created_by_id, updated_by_id, created_at, updated_at
                    )
                    VALUES ($1, $2, TRUE, TRUE, $3, $4, $5, $6)
                    ON CONFLICT (r_number) DO UPDATE SET
                        updated_by_id = EXCLUDED.updated_by_id,
                        updated_at = EXCLUDED.updated_at,
                        default_info = TRUE,
                        default_contact = TRUE
                    "#,
                    vec![
                        Value::Number(contact_id.into()),
                        Value::Number(info_id.into()),
                        created_by_id
                            .map(|v| Value::Number(v.into()))
                            .unwrap_or(Value::Null),
                        updated_by_id
                            .map(|v| Value::Number(v.into()))
                            .unwrap_or(Value::Null),
                        created_at
                            .map(|v| Value::String(v.to_rfc3339()))
                            .unwrap_or(Value::Null),
                        updated_at
                            .map(|v| Value::String(v.to_rfc3339()))
                            .unwrap_or(Value::Null),
                    ],
                )
                .await?;
        }

        Ok(TriggerResult::new())
    }
}

/// No-pool fallback: emit side effects for executor to apply
fn fallback_side_effects(entity_id: i64, notice: Option<String>) -> TriggerResult {
    let mut result = TriggerResult::new();
    result = result.with_side_effect(SideEffect::Insert {
        table: "zc_id_contacts".to_string(),
        values: {
            let mut values = HashMap::new();
            values.insert(
                "notice".to_string(),
                Value::String(notice.unwrap_or_else(|| "Contact".to_string())),
            );
            values.insert("fk_entity".to_string(), Value::Number(entity_id.into()));
            values
        },
    });
    result = result.with_side_effect(SideEffect::Insert {
        table: "zc_id_contact_infos".to_string(),
        values: {
            let mut values = HashMap::new();
            values.insert("notice".to_string(), Value::String("站内信".to_string()));
            values.insert(
                "fk_contact".to_string(),
                Value::String("LAST_INSERT_ID".to_string()),
            );
            values
        },
    });
    result
}
