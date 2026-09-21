use async_trait::async_trait;
use sqlx::{Error, PgPool};
use system_config::{
    CreateSystemConfigRequest, SystemConfig, SystemConfigRepository, UpdateSystemConfigRequest,
};

/// 前端请求分类（`_f_` 请求参数，业务枚举 llm/email/...）→ 目标表。
/// 注意：这是「请求参数 → 表」的翻译，与物理 `_f_` 列无关——活库
/// `zc_id_prot-*_config` 的 `_f_`/`_t_` 列由 `dk_function.code` 前缀自动派生
/// （ALIOTH_ONTOLOGY_SPEC §4.3），业务层禁止显式赋值。
/// 未知分类返回 None（由 service validate_request 兜底）。
///
/// 单族配置表投影（`_f_` 由分类字面量派生；正文单一来源）
macro_rules! live_fields {
    ($category:literal) => {
        concat!(
            "id, notice, code, '",
            $category,
            "' AS \"_f_\", settings->>'provider' AS \"_t_\", comments, \
             enc_fields AS credentials, settings, \
             COALESCE((settings->>'enabled')::boolean, false) AS enabled, \
             COALESCE((settings->>'is_default')::boolean, false) AS is_default, \
             settings->>'domain_' AS domain_, \
             COALESCE((settings->>'public')::boolean, false) AS public, \
             created_at, updated_at, created_by_id, updated_by_id, deleted_at"
        )
    };
}

/// 六族配置表 → 分类 + 编译期 SQL（表名与投影同源；新增族 = 加一行）
///
/// 投影语义（见 [`live_fields`]）：`_f_` 由分类字面量派生（勿读物理列）；
/// `_t_`（provider）读 `settings->>'provider'`；enc_fields → credentials；
/// settings 内嵌 enabled/is_default/domain_/public。
struct ConfigFamilySql {
    category: &'static str,
    select_by_code: &'static str,
    select_by_id: &'static str,
    insert_sql: &'static str,
    update_sql: &'static str,
    soft_delete_sql: &'static str,
}

macro_rules! config_family {
    ($category:literal, $table:literal) => {
        ConfigFamilySql {
            category: $category,
            select_by_code: concat!(
                "SELECT ",
                live_fields!($category),
                " FROM isahl.\"",
                $table,
                "\" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"
            ),
            select_by_id: concat!(
                "SELECT ",
                live_fields!($category),
                " FROM isahl.\"",
                $table,
                "\" WHERE id = $1 AND deleted_at IS NULL LIMIT 1"
            ),
            insert_sql: concat!(
                "INSERT INTO isahl.\"",
                $table,
                "\" \
                 (notice, code, comments, enc_fields, settings, created_by_id) \
                 VALUES ($1, $2, $3, $4, $5, $6) RETURNING ",
                live_fields!($category)
            ),
            update_sql: concat!(
                "UPDATE isahl.\"",
                $table,
                "\" SET \
                 notice = COALESCE($2, notice), code = COALESCE($3, code), \
                 comments = COALESCE($4, comments), enc_fields = COALESCE($5, enc_fields), \
                 settings = $6, updated_at = NOW() \
                 WHERE id = $1 AND deleted_at IS NULL RETURNING ",
                live_fields!($category)
            ),
            soft_delete_sql: concat!(
                "UPDATE isahl.\"",
                $table,
                "\" \
                 SET deleted_at = NOW(), updated_at = NOW() \
                 WHERE id = $1 AND deleted_at IS NULL"
            ),
        }
    };
}

const CONFIG_FAMILIES: &[ConfigFamilySql] = &[
    config_family!("llm", "zc_id_prot-llm_config"),
    config_family!("email", "zc_id_prot-email_config"),
    config_family!("im", "zc_id_prot-im_config"),
    config_family!("webhook", "zc_id_prot-webhook_config"),
    config_family!("storage", "zc_id_prot-oss_config"),
    config_family!("sms", "zc_id_prot-sms_config"),
];

/// 六族全量 UNION 列表 SQL（编译期固化；族成员与投影同源）
macro_rules! live_config_union_sql {
    ($first_c:literal, $first_t:literal $(, $c:literal, $t:literal)* $(,)?) => {
        concat!(
            "SELECT * FROM (SELECT ", live_fields!($first_c), " FROM isahl.\"", $first_t,
            "\" WHERE deleted_at IS NULL",
            $(
                " UNION ALL SELECT ", live_fields!($c), " FROM isahl.\"", $t,
                "\" WHERE deleted_at IS NULL"
            ),*
            , ") c ORDER BY updated_at DESC LIMIT $1 OFFSET $2"
        )
    };
}

const CONFIG_LIST_SQL: &str = live_config_union_sql!(
    "llm",
    "zc_id_prot-llm_config",
    "email",
    "zc_id_prot-email_config",
    "im",
    "zc_id_prot-im_config",
    "webhook",
    "zc_id_prot-webhook_config",
    "storage",
    "zc_id_prot-oss_config",
    "sms",
    "zc_id_prot-sms_config",
);

/// 分类 code → 族（`_f_` 值域）
fn family_of_category(category: &str) -> Option<&'static ConfigFamilySql> {
    CONFIG_FAMILIES.iter().find(|f| f.category == category)
}

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
        for fam in CONFIG_FAMILIES {
            let row = sqlx::query_as::<_, SystemConfig>(fam.select_by_code)
                .bind(code)
                .fetch_optional(&self.pool)
                .await?;
            if let Some(mut cfg) = row {
                cfg._f_ = Some(fam.category.to_string());
                return Ok(Some(cfg));
            }
        }
        Ok(None)
    }

    async fn insert(&self, req: &CreateSystemConfigRequest) -> Result<SystemConfig, Error> {
        let Some(fam) = req._f_.as_deref().and_then(family_of_category) else {
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
        sqlx::query_as::<_, SystemConfig>(fam.insert_sql)
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
        for fam in CONFIG_FAMILIES {
            let row = sqlx::query_as::<_, SystemConfig>(fam.select_by_id)
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
            if let Some(mut cfg) = row {
                cfg._f_ = Some(fam.category.to_string());
                return Ok(Some(cfg));
            }
        }
        Ok(None)
    }

    async fn list(&self, limit: i64, offset: i64) -> Result<Vec<SystemConfig>, Error> {
        // 六族全量 UNION ALL（分类 = 编译期字面量；provider 读 settings->>'provider'）
        sqlx::query_as::<_, SystemConfig>(CONFIG_LIST_SQL)
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
        let Some(fam) = family_of_category(f) else {
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
        let row = sqlx::query_as::<_, SystemConfig>(fam.update_sql)
            .bind(id)
            .bind(&req.notice)
            .bind(&req.code)
            .bind(&req.comments)
            .bind(&req.credentials)
            .bind(&settings)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(mut cfg) = row {
            cfg._f_ = Some(fam.category.to_string());
            Ok(Some(cfg))
        } else {
            Ok(None)
        }
    }

    async fn soft_delete(&self, id: i64) -> Result<u64, Error> {
        // 逐族表尝试软删（命中的表返回 1）
        for fam in CONFIG_FAMILIES {
            let n = sqlx::query(fam.soft_delete_sql)
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
