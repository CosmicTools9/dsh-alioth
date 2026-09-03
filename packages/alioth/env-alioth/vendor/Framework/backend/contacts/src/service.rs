//! 联系人查询服务
//!
//! 聚合链:
//!   zc_id_entity → zc_id_entity_rr_contacts → zc_id_contacts
//!               → zc_id_contacts_rr_infos → zc_id_contact_infos
//!   叶表: zc_id_info-email, zc_id_info-telephone, zc_id_info-im,
//!          zc_id_info-isahl, zc_id_info-postal, zc_id_info-zipcode
//!
//! 使用 QueryBuilder::fetch_refs + OrderedJunction(ToMany) 替代 raw JOIN，
//! 避免 polymorphic rr + DISTINCT ON 导致的 email/phone 互斥丢失。
//! 返回 typed `infos` 数组供前端消费。

use crud::{entity::AliothDbEntity, pagination::PaginatedResponse, query_builder::QueryBuilder};
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{ContactInfo, ContactInfoValue, ContactsEntity};

/// 6 个 info 叶表类型与其 _refs key 的映射
const INFO_KINDS: &[(&str, &str)] = &[
    ("email", "email"),
    ("phone", "phone"),
    ("im", "im"),
    ("isahl", "isahl"),
    ("postal", "postal"),
    ("zipcode", "zipcode"),
];

pub struct ContactsService;

impl ContactsService {
    /// 获取联系人列表（含全部关联信息，支持分页）
    pub async fn list_contacts(
        pool: &PgPool,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<ContactInfo>, i64), String> {
        let builder = QueryBuilder::<ContactsEntity>::new(pool)
            .raw_filter("notice IS NOT NULL AND notice != ''".into());

        let paginated: PaginatedResponse<ContactsEntity> = builder
            .fetch_refs(page, page_size)
            .await
            .map_err(|e| format!("contacts query failed: {}", e))?;

        let contacts = paginated
            .items
            .into_iter()
            .map(Self::entity_to_info)
            .collect();

        Ok((contacts, paginated.total))
    }

    fn entity_to_info(entity: ContactsEntity) -> ContactInfo {
        let refs = &entity._refs;

        // 从 _refs 中收集全部 info 值为 typed infos 数组
        let mut infos: Vec<ContactInfoValue> = Vec::new();
        for (kind, refs_key) in INFO_KINDS {
            if let Some(arr) = refs
                .as_ref()
                .and_then(|v| v.get(*refs_key))
                .and_then(|v| v.as_array())
            {
                for item in arr {
                    let value = item.get("notice").and_then(|v| v.as_str()).unwrap_or("");
                    let is_default = item
                        .get("is_default")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    infos.push(ContactInfoValue {
                        kind: kind.to_string(),
                        value: value.to_string(),
                        is_default,
                    });
                }
            }
        }

        // 部门名称
        let department = refs
            .as_ref()
            .and_then(|v| v.get("department"))
            .and_then(|v| v.get("notice"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // 便捷字段：取默认邮箱和电话
        let email = infos
            .iter()
            .find(|i| i.kind == "email" && i.is_default)
            .or_else(|| infos.iter().find(|i| i.kind == "email"))
            .map(|i| i.value.clone());
        let phone = infos
            .iter()
            .find(|i| i.kind == "phone" && i.is_default)
            .or_else(|| infos.iter().find(|i| i.kind == "phone"))
            .map(|i| i.value.clone());

        ContactInfo {
            id: entity.id,
            name: entity.notice.unwrap_or_default(),
            email,
            phone,
            department,
            position: entity.position,
            avatar_url: entity.avatar_url,
            is_online: None,
            infos,
        }
    }

    /// 创建联系人（事务内完成 contact + info + rr 写入）
    pub async fn create_contact(
        pool: &PgPool,
        req: crate::models::CreateContactRequest,
    ) -> Result<ContactInfo, String> {
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| format!("begin tx failed: {}", e))?;

        // code 必填（DTO_DESIGN_SPEC：本体核心编码）——空缺时自动生成（与 subjects SUBJ- 同范式）
        let code = match req.code.as_deref().map(str::trim) {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => {
                let z: i64 = sqlx::query_scalar("SELECT isahl.gen_next_zuid()")
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| format!("generate contact code failed: {}", e))?;
                format!("CT-{:06}", z % 1_000_000)
            }
        };
        let contact_id: i64 = sqlx::query_scalar(
            "INSERT INTO isahl.zc_id_contacts (notice, code, comments) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(&req.name)
        .bind(&code)
        .bind(&req.comments)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| format!("create contact failed: {}", e))?;

        for info in &req.infos {
            Self::create_contact_info_direct(&mut tx, contact_id, info).await?;
        }

        tx.commit()
            .await
            .map_err(|e| format!("commit tx failed: {}", e))?;

        Self::get_contact_by_id(pool, contact_id)
            .await
            .and_then(|c| c.ok_or_else(|| "created contact not found".to_string()))
    }

    /// 更新联系人（事务内完成，替换全部 infos）
    pub async fn update_contact(
        pool: &PgPool,
        id: i64,
        req: crate::models::UpdateContactRequest,
        user_id: i64,
    ) -> Result<Option<ContactInfo>, String> {
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| format!("begin tx failed: {}", e))?;

        if req.name.is_some() || req.code.is_some() || req.comments.is_some() {
            sqlx::query(
                "UPDATE isahl.zc_id_contacts SET notice = COALESCE($1, notice), code = COALESCE($2, code), comments = COALESCE($3, comments) WHERE id = $4 AND deleted_at IS NULL",
            )
            .bind(&req.name)
            .bind(&req.code)
            .bind(&req.comments)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("update contact failed: {}", e))?;
        }

        if !req.infos.is_empty() {
            // 先收集旧值行 id（rr_infos 软删前），再同事务级联软删值行与关系行
            let old_info_ids: Vec<i64> = sqlx::query_scalar(
                r#"SELECT ref_right FROM isahl."zc_id_contacts_rr_infos" WHERE ref_left = $1 AND deleted_at IS NULL"#,
            )
            .bind(id)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| format!("collect old infos failed: {}", e))?;
            if !old_info_ids.is_empty() {
                // 父表 UPDATE（PG 继承语义落子表行），对齐 subjects.rs remove_subject_contact
                sqlx::query(
                    r#"UPDATE isahl.zc_id_contact_infos x SET deleted_at = now(), deleted_by_id = $2, updated_at = now() WHERE x.id = ANY($1) AND x.deleted_at IS NULL"#,
                )
                .bind(&old_info_ids)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("soft delete old infos failed: {}", e))?;
            }
            sqlx::query(
                r#"UPDATE isahl."zc_id_contacts_rr_infos" SET deleted_at = now(), deleted_by_id = $2, updated_at = now() WHERE ref_left = $1 AND deleted_at IS NULL"#,
            )
            .bind(id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("clear old infos failed: {}", e))?;

            for info in &req.infos {
                Self::create_contact_info_direct(&mut tx, id, info).await?;
            }
        }

        tx.commit()
            .await
            .map_err(|e| format!("commit tx failed: {}", e))?;

        Self::get_contact_by_id(pool, id).await
    }

    /// 删除联系人（同事务级联软删：entity_rr_contacts 挂接行 + info 值行 + rr_infos + contacts）
    pub async fn delete_contact(pool: &PgPool, id: i64, user_id: i64) -> Result<bool, String> {
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| format!("begin tx failed: {}", e))?;

        // 实体挂接行（ref_right = 联系人 id）
        sqlx::query(
            r#"UPDATE isahl."zc_id_entity_rr_contacts" SET deleted_at = now(), deleted_by_id = $2, updated_at = now() WHERE ref_right = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("delete entity contact links failed: {}", e))?;

        // 值行软删（先取 id——rr_infos 软删前）
        let info_ids: Vec<i64> = sqlx::query_scalar(
            r#"SELECT ref_right FROM isahl."zc_id_contacts_rr_infos" WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| format!("collect infos failed: {}", e))?;
        if !info_ids.is_empty() {
            sqlx::query(
                r#"UPDATE isahl.zc_id_contact_infos x SET deleted_at = now(), deleted_by_id = $2, updated_at = now() WHERE x.id = ANY($1) AND x.deleted_at IS NULL"#,
            )
            .bind(&info_ids)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| format!("soft delete infos failed: {}", e))?;
        }

        // 关系行
        sqlx::query(
            r#"UPDATE isahl."zc_id_contacts_rr_infos" SET deleted_at = now(), deleted_by_id = $2, updated_at = now() WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("delete info links failed: {}", e))?;

        // 联系人本体
        let affected = sqlx::query(
            r#"UPDATE isahl.zc_id_contacts SET deleted_at = now(), deleted_by_id = $2, updated_at = now() WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("delete contact failed: {}", e))?;

        tx.commit()
            .await
            .map_err(|e| format!("commit tx failed: {}", e))?;

        Ok(affected.rows_affected() > 0)
    }

    /// 按 ID 获取单条联系人
    async fn get_contact_by_id(pool: &PgPool, id: i64) -> Result<Option<ContactInfo>, String> {
        let refs_suffix = crud::reference::build_refs_select_suffix::<ContactsEntity>();
        let sql = format!(
            "SELECT {} {} FROM {} AS e WHERE e.id = $1 AND e.deleted_at IS NULL",
            ContactsEntity::SELECT_FIELDS,
            refs_suffix,
            ContactsEntity::table_name(),
        );
        let entity: Option<ContactsEntity> = sqlx::query_as(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("get contact by id failed: {}", e))?;
        let entity = match entity {
            Some(e) => e,
            None => return Ok(None),
        };
        Ok(Some(Self::entity_to_info(entity)))
    }

    /// 创建单条联系方式并关联到联系人（事务内使用）
    async fn create_contact_info_direct(
        executor: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        contact_id: i64,
        info: &crate::models::ContactInfoInput,
    ) -> Result<(), String> {
        // PostgreSQL inheritance: INSERT into child table auto-creates row visible in parent
        // 批注 6c9efdc1：前端 6 枚举（mobile/fax/wechat/qq）与后端白名单不一致
        // ——别名归一：mobile→telephone、fax→postal、wechat/qq→im
        let kind = match info.kind.as_str() {
            "mobile" => "telephone",
            "fax" => "postal",
            "wechat" | "qq" => "im",
            other => other,
        };
        let info_id = match kind {
            "email" => sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl."zc_id_info-email" (notice) VALUES ($1) RETURNING id"#,
            )
            .bind(&info.value)
            .fetch_one(&mut **executor)
            .await
            .map_err(|e| format!("create email info failed: {}", e))?,
            "phone" | "telephone" => sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl."zc_id_info-telephone" (notice) VALUES ($1) RETURNING id"#,
            )
            .bind(&info.value)
            .fetch_one(&mut **executor)
            .await
            .map_err(|e| format!("create phone info failed: {}", e))?,
            "im" => sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl."zc_id_info-im" (notice) VALUES ($1) RETURNING id"#,
            )
            .bind(&info.value)
            .fetch_one(&mut **executor)
            .await
            .map_err(|e| format!("create im info failed: {}", e))?,
            "isahl" => sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl."zc_id_info-isahl" (notice) VALUES ($1) RETURNING id"#,
            )
            .bind(&info.value)
            .fetch_one(&mut **executor)
            .await
            .map_err(|e| format!("create isahl info failed: {}", e))?,
            "postal" => sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl."zc_id_info-postal" (notice) VALUES ($1) RETURNING id"#,
            )
            .bind(&info.value)
            .fetch_one(&mut **executor)
            .await
            .map_err(|e| format!("create postal info failed: {}", e))?,
            "zipcode" => sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl."zc_id_info-zipcode" (notice) VALUES ($1) RETURNING id"#,
            )
            .bind(&info.value)
            .fetch_one(&mut **executor)
            .await
            .map_err(|e| format!("create zipcode info failed: {}", e))?,
            other => return Err(format!("unknown info kind: {}", other)),
        };

        // 关联到联系人
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_contacts_rr_infos" (ref_left, ref_right, default_info) VALUES ($1, $2, $3)"#,
        )
        .bind(contact_id)
        .bind(info_id)
        .bind(info.is_default)
        .execute(&mut **executor)
        .await
        .map_err(|e| format!("create info link failed: {}", e))?;

        Ok(())
    }
}
