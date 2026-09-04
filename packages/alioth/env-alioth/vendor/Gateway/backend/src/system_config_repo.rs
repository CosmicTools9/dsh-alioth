use async_trait::async_trait;
use sqlx::{AssertSqlSafe, Error, PgPool};
use system_config::{
    CreateSystemConfigRequest, SystemConfig, SystemConfigRepository, UpdateSystemConfigRequest,
};

/// 前端请求分类（`_f_` 请求参数，业务枚举 llm/email/...）→ 目标表名。
/// 注意：这是「请求参数 → 表」的翻译，与物理 `_f_` 列无关——活库
/// `zc_id_prot-*_config` 的 `_f_`/`_t_` 列由 `dk_function.code` 前缀自动派生
/// （ALIOTH_ONTOLOGY_SPEC §4.3），业务层禁止显式赋值。
/// 未知分类返回 None（由 service validate_request 兜底）。
fn table_for(category: &str) -> Option<&'static str> {
    match category {
        "llm" => Some("zc_id_prot-llm_config"),
        "email" => Some("zc_id_prot-email_config"),
        "im" => Some("zc_id_prot-im_config"),
        "webhook" => Some("zc_id_prot-webhook_config"),
        "storage" => Some("zc_id_prot-oss_config"),
        "sms" => Some("zc_id_prot-sms_config"),
        _ => None,
    }
}

/// 分类 code → DTO `_f_` 值（表名推导，与物理列无关）。
fn category_of_table(table: &str) -> &'static str {
    match table {
        "zc_id_prot-llm_config" => "llm",
        "zc_id_prot-email_config" => "email",
        "zc_id_prot-im_config" => "im",
        "zc_id_prot-webhook_config" => "webhook",
        "zc_id_prot-oss_config" => "storage",
        "zc_id_prot-sms_config" => "sms",
        _ => "unknown",
    }
}

/// 六族表探测常量（分类, 表名）。
const FAMILY_TABLES: [(&str, &str); 6] = [
    ("llm", "zc_id_prot-llm_config"),
    ("email", "zc_id_prot-email_config"),
    ("im", "zc_id_prot-im_config"),
    ("webhook", "zc_id_prot-webhook_config"),
    ("storage", "zc_id_prot-oss_config"),
    ("sms", "zc_id_prot-sms_config"),
];

/// 活库族表 SELECT 投影 → SystemConfig。
/// - `_f_` 由表名派生（`{category}` 占位，勿读物理列）；
/// - `_t_`（provider）读 `settings->>'provider'`（勿读物理列）；
/// - enc_fields → credentials，settings 内嵌 enabled/is_default/domain_/public。
const LIVE_FIELDS: &str = r#"
    id, notice, code, '{category}' AS "_f_", settings->>'provider' AS "_t_", comments,
    enc_fields AS credentials, settings,
    COALESCE((settings->>'enabled')::boolean, false) AS enabled,
    COALESCE((settings->>'is_default')::boolean, false) AS is_default,
    settings->>'domain_' AS domain_, COALESCE((settings->>'public')::boolean, false) AS public,
    created_at, updated_at, created_by_id, updated_by_id, deleted_at
"#;

/// 从请求合并 provider/enabled/is_default/public/domain_ 进 settings。
/// `_t_` 请求字段（provider 业务枚举）并入 settings，不写物理 `_t_` 列。
fn merge_request_flags(
    settings: &mut serde_json::Value,
    provider: Option<&str>,
    enabled: bool,
    is_default: bool,
    public: bool,
    domain_: Option<&str>,
) {
    if settings.as_object().is_none() {
        *settings = serde_json::json!({});
    }
    let obj = settings.as_object_mut().expect("settings 已规范为 object");
    if let Some(p) = provider {
        obj.insert("provider".into(), serde_json::json!(p));
    }
    obj.insert("enabled".into(), serde_json::json!(enabled));
    obj.insert("is_default".into(), serde_json::json!(is_default));
    obj.insert("public".into(), serde_json::json!(public));
    if let Some(d) = domain_ {
        obj.insert("domain_".into(), serde_json::json!(d));
    }
}

#[derive(Clone)]
pub struct SystemConfigRepo {
    pool: PgPool,
}

impl SystemConfigRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SystemConfigRepository for SystemConfigRepo {
    async fn find_by_code(&self, code: &str) -> Result<Option<SystemConfig>, Error> {
        // 跨全族表查找（未知 code 时逐表探测；通常行数极少）
        for (category, table) in FAMILY_TABLES {
            let fields = LIVE_FIELDS.replace("{category}", category);
            let sql = format!(
                r#"SELECT {fields} FROM isahl."{table}"
                   WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
                fields = fields,
                table = table
            );
            let row = sqlx::query_as::<_, SystemConfig>(AssertSqlSafe(sql.as_str()))
                .bind(code)
                .fetch_optional(&self.pool)
                .await?;
            if let Some(mut cfg) = row {
                cfg._f_ = Some(category.to_string());
                return Ok(Some(cfg));
            }
        }
        Ok(None)
    }

    async fn insert(&self, req: &CreateSystemConfigRequest) -> Result<SystemConfig, Error> {
        let Some(table) = req._f_.as_deref().and_then(table_for) else {
            return Err(Error::Protocol(format!("不支持的配置分类: {:?}", req._f_)));
        };
        let mut settings = req
            .settings
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        merge_request_flags(
            &mut settings,
            req._t_.as_deref(),
            req.enabled,
            req.is_default,
            req.public,
            req.domain_.as_deref(),
        );
        // 不写 `_f_`/`_t_` 物理列（lifecycle 自动维度，业务禁止赋值）。
        let fields = LIVE_FIELDS.replace("{category}", category_of_table(table));
        let sql = format!(
            r#"INSERT INTO isahl."{table}" (
                notice, code, comments, enc_fields, settings, created_by_id
            ) VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING {fields}"#,
            table = table,
            fields = fields
        );
        sqlx::query_as::<_, SystemConfig>(AssertSqlSafe(sql.as_str()))
            .bind(&req.notice)
            .bind(&req.code)
            .bind(&req.comments)
            .bind(&req.credentials) // service 层已加密 → enc_fields
            .bind(&settings)
            .bind(None::<i64>) // created_by_id 由 handler 层后续补充（现状一致）
            .fetch_one(&self.pool)
            .await
    }

    async fn find_by_id(&self, id: i64) -> Result<Option<SystemConfig>, Error> {
        // id 跨族表探测（id 为全局 ZUID，可跨表）
        for (category, table) in FAMILY_TABLES {
            let fields = LIVE_FIELDS.replace("{category}", category);
            let sql = format!(
                r#"SELECT {fields} FROM isahl."{table}"
                   WHERE id = $1 AND deleted_at IS NULL LIMIT 1"#,
                fields = fields,
                table = table
            );
            let row = sqlx::query_as::<_, SystemConfig>(AssertSqlSafe(sql.as_str()))
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
            if let Some(mut cfg) = row {
                cfg._f_ = Some(category.to_string());
                return Ok(Some(cfg));
            }
        }
        Ok(None)
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<SystemConfig>, Error> {
        // UNION ALL 六族表（分类 = 表名常量；provider 读 settings->>'provider'）
        let mut sql = String::from("SELECT * FROM (");
        for (i, (category, table)) in FAMILY_TABLES.iter().enumerate() {
            if i > 0 {
                sql.push_str(" UNION ALL ");
            }
            let fields = LIVE_FIELDS.replace("{category}", category);
            sql.push_str(&format!(
                r#"SELECT {fields} FROM isahl."{table}" WHERE deleted_at IS NULL"#,
                fields = fields,
                table = table
            ));
        }
        sql.push_str(
            r#") c
            ORDER BY updated_at DESC
            LIMIT $1 OFFSET $2"#,
        );
        sqlx::query_as::<_, SystemConfig>(AssertSqlSafe(sql.as_str()))
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
    }

    async fn update(
        &self,
        id: i64,
        req: &UpdateSystemConfigRequest,
    ) -> Result<Option<SystemConfig>, Error> {
        let Some(f) = req._f_.as_deref() else {
            return Err(Error::Protocol("更新请求缺少 _f_ 分类".into()));
        };
        let Some(table) = table_for(f) else {
            return Err(Error::Protocol(format!("不支持的配置分类: {}", f)));
        };
        // 现有行（取回 settings 以合并标志/provider）
        let existing = self.find_by_id(id).await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        let mut settings = req.settings.clone().unwrap_or_else(|| {
            existing
                .settings
                .clone()
                .unwrap_or_else(|| serde_json::json!({}))
        });
        let enabled = req.enabled.or(existing.enabled).unwrap_or(false);
        let is_default = req.is_default.or(existing.is_default).unwrap_or(false);
        let domain_ = req.domain_.clone().or(existing.domain_.clone());
        let provider = req._t_.clone().or_else(|| {
            existing
                .settings
                .as_ref()
                .and_then(|s| s.get("provider").and_then(|v| v.as_str()).map(String::from))
        });
        {
            if settings.as_object().is_none() {
                settings = serde_json::json!({});
            }
            let obj = settings.as_object_mut().expect("settings 已规范为 object");
            if let Some(p) = &provider {
                obj.insert("provider".into(), serde_json::json!(p));
            }
            obj.insert("enabled".into(), serde_json::json!(enabled));
            obj.insert("is_default".into(), serde_json::json!(is_default));
            if let Some(d) = &domain_ {
                obj.insert("domain_".into(), serde_json::json!(d));
            }
            if let Some(p) = req.public.or(existing.public) {
                obj.insert("public".into(), serde_json::json!(p));
            }
        }
        let fields = LIVE_FIELDS.replace("{category}", category_of_table(table));
        let sql = format!(
            r#"UPDATE isahl."{table}" SET
                notice = COALESCE($2, notice),
                code = COALESCE($3, code),
                comments = COALESCE($4, comments),
                enc_fields = COALESCE($5, enc_fields),
                settings = $6,
                updated_at = NOW()
            WHERE id = $1 AND deleted_at IS NULL
            RETURNING {fields}"#,
            table = table,
            fields = fields
        );
        let row = sqlx::query_as::<_, SystemConfig>(AssertSqlSafe(sql.as_str()))
            .bind(id)
            .bind(&req.notice)
            .bind(&req.code)
            .bind(&req.comments)
            .bind(&req.credentials)
            .bind(&settings)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(mut cfg) = row {
            cfg._f_ = Some(f.to_string());
            Ok(Some(cfg))
        } else {
            Ok(None)
        }
    }

    async fn soft_delete(&self, id: i64) -> Result<u64, Error> {
        // 逐族表尝试软删（命中的表返回 1）
        for (_, table) in FAMILY_TABLES {
            let sql = format!(
                r#"UPDATE isahl."{table}"
                   SET deleted_at = NOW(), updated_at = NOW()
                   WHERE id = $1 AND deleted_at IS NULL"#
            );
            let n = sqlx::query(AssertSqlSafe(sql.as_str()))
                .bind(id)
                .execute(&self.pool)
                .await?
                .rows_affected();
            if n > 0 {
                return Ok(n);
            }
        }
        Ok(0)
    }
}
