//! Business-Specific Trigger Templates
//!
//! 本模块已从“传统 Trigger trait 实现集合”深化为“TriggerTemplate 模板集合 +
//! BusinessRegistryLoader 注册表加载器”。
//!
//! 所有 SQL 逻辑委托 TemplateEngine，体现“小接口、大行为在背后”。

use crate::{
    loader::RegistryLoader,
    template::{
        TemplateEngine, TriggerMetadata, TriggerOperationDef, TriggerTemplate, TriggerTimingDef,
    },
    utils::*,
    Trigger, TriggerContext, TriggerError, TriggerResult,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

// ============================================
// Dimension Auto-Code Symbol Array
// ============================================
const SYMA: &[char] = &[
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'L', 'M', 'N', 'P', 'Q', 'R', 'S', 'T', 'U',
    'V', 'W', 'X', 'Y', 'Z', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'Γ', 'Δ', 'Λ', 'Ξ', 'Π',
    'Σ', 'Φ', 'Ψ', 'Ω',
];
// ============================================

// ============================================
// Dimension Auto-Code Template
// ============================================

/// 基于 TriggerTemplate 接口的维度自动编码实现。
pub struct DimensionAutoCodeTemplate {
    table: &'static str,
    /// ck_category 指向的类目表（scene→cons-industry-cate 等）。
    /// code/c_sort_ 从该类目表读取（父表 zc_id_category 无 c_sort_ 列，
    /// 且父表加列会被子表本地同名列阻断——不能依赖父表视角查询）。
    cate_table: &'static str,
    prefix: &'static str,
}

impl DimensionAutoCodeTemplate {
    pub fn scene() -> Self {
        Self {
            table: "zc_id_scene",
            cate_table: "zc_id_cons-industry-cate",
            prefix: "SC",
        }
    }
    pub fn factor() -> Self {
        Self {
            table: "zc_id_factor",
            cate_table: "zc_id_cons-factor-cate",
            prefix: "FC",
        }
    }
    pub fn function() -> Self {
        Self {
            table: "zc_id_function",
            cate_table: "zc_id_cons-function-cate",
            prefix: "FN",
        }
    }
}

#[async_trait]
impl TriggerTemplate for DimensionAutoCodeTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: format!("tf_bf_ups_on_{}", self.table),
            applies_to: vec![self.table.to_string()],
            operations: vec![TriggerOperationDef::Insert, TriggerOperationDef::Update],
            timing: TriggerTimingDef::Before,
        }
    }

    async fn execute(
        &self,
        ctx: &TriggerContext,
        _old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;
        // 已有 code 则跳过
        if get_field::<String>(new, "code").is_some() {
            return Ok(TriggerResult::new());
        }

        let ck_category: Option<i64> = get_field(new, "ck_category");
        if ck_category.is_none() {
            // 无 ck_category：从 notice 生成简码
            let notice: Option<String> = get_field(new, "notice");
            let source = notice.as_deref().unwrap_or("");
            if source.is_empty() {
                return Ok(TriggerResult::new());
            }
            let clean: String = source
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .take(8)
                .collect();
            let hash = crate::utils::crc32_hex(source);
            let code = format!("{}-{}", clean.to_uppercase(), &hash[..4]);
            return Ok(TriggerResult::new().with_modified_field("code", Value::String(code)));
        }
        let ck = ck_category.unwrap();

        let engine = TemplateEngine::new(ctx.pool.clone());

        // 无 pool fallback
        if ctx.pool.is_none() {
            return Ok(TriggerResult::new()
                .with_modified_field("code", Value::String(format!("{}{}", self.prefix, ck))));
        }

        // 1. 查询已用 slots
        let used_slots_sql = format!(
            r#"SELECT c_sort_ & 63 FROM isahl."{}" WHERE ck_category = $1 AND c_sort_ IS NOT NULL"#,
            self.table
        );
        let slots: Vec<i64> = engine
            .query_scalar_all(&used_slots_sql, vec![Value::Number(ck.into())])
            .await?;

        let mut sidx: usize = 1;
        for i in 1..=SYMA.len() {
            if !slots.contains(&(i as i64)) {
                sidx = i;
                break;
            }
        }
        if sidx > SYMA.len() {
            return Err(TriggerError::ExecutionFailed("编码符容量不足!".to_string()));
        }

        // 2. 查询 ck 类目表的 code 和 c_sort_（各维度类目表固定：
        //    scene→cons-industry-cate、factor→cons-factor-cate、function→cons-function-cate）
        let cat_code: Option<String> = engine
            .query_scalar(
                &format!(
                    "SELECT code FROM isahl.\"{}\" WHERE id = $1",
                    self.cate_table
                ),
                vec![Value::Number(ck.into())],
            )
            .await?;
        let cat_sort: Option<i64> = engine
            .query_scalar(
                &format!(
                    "SELECT c_sort_ FROM isahl.\"{}\" WHERE id = $1",
                    self.cate_table
                ),
                vec![Value::Number(ck.into())],
            )
            .await?;

        let code = format!("{}{}", cat_code.unwrap_or_default(), SYMA[sidx - 1]);
        let c_sort = (cat_sort.unwrap_or(0) << 6) | (sidx as i64);

        Ok(TriggerResult::new()
            .with_modified_field("code", Value::String(code))
            .with_modified_field("c_sort_", Value::Number(c_sort.into())))
    }
}

// ============================================
// Project / Task Templates
// ============================================

pub struct ProjectParticipantsTemplate;

#[async_trait]
impl TriggerTemplate for ProjectParticipantsTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_72_on_zc_id_project".to_string(),
            applies_to: vec!["zc_id_project".to_string()],
            operations: vec![TriggerOperationDef::Insert, TriggerOperationDef::Update],
            timing: TriggerTimingDef::Before,
        }
    }

    async fn execute(
        &self,
        ctx: &TriggerContext,
        _old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;

        let created_by_id: Option<i64> = get_field(new, "created_by_id");
        let fk_launcher: Option<i64> = get_field(new, "fk_launcher");

        if created_by_id.is_none() || fk_launcher.is_some() {
            return Ok(TriggerResult::new());
        }

        let engine = TemplateEngine::new(ctx.pool.clone());
        let launcher_id: Option<i64> = engine
            .resolve_subject_by_user(created_by_id.unwrap())
            .await?;

        if let Some(lid) = launcher_id {
            Ok(TriggerResult::new().with_modified_field("fk_launcher", Value::Number(lid.into())))
        } else {
            Ok(TriggerResult::new())
        }
    }
}

pub struct TaskInitiatorTemplate;

#[async_trait]
impl TriggerTemplate for TaskInitiatorTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_72_on_zc_id_task".to_string(),
            applies_to: vec!["zc_id_task".to_string()],
            operations: vec![TriggerOperationDef::Insert, TriggerOperationDef::Update],
            timing: TriggerTimingDef::Before,
        }
    }

    async fn execute(
        &self,
        ctx: &TriggerContext,
        _old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;

        let created_by_id: Option<i64> = get_field(new, "created_by_id");
        if created_by_id.is_none() {
            return Ok(TriggerResult::new());
        }

        let engine = TemplateEngine::new(ctx.pool.clone());
        let subject_id: Option<i64> = engine
            .resolve_subject_by_user(created_by_id.unwrap())
            .await?;

        if let Some(sid) = subject_id {
            Ok(TriggerResult::new().with_modified_field("fk_subject", Value::Number(sid.into())))
        } else {
            Ok(TriggerResult::new())
        }
    }
}

// ============================================
// Entity User Template
// ============================================

pub struct EntityUserTemplate;

#[async_trait]
impl TriggerTemplate for EntityUserTemplate {
    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata {
            name: "tf_bf_ups_71_on_zc_id_entity".to_string(),
            applies_to: vec!["zc_id_entity".to_string()],
            operations: vec![TriggerOperationDef::Insert, TriggerOperationDef::Update],
            timing: TriggerTimingDef::Before,
        }
    }

    async fn execute(
        &self,
        ctx: &TriggerContext,
        _old_record: Option<&HashMap<String, Value>>,
        new_record: Option<&HashMap<String, Value>>,
    ) -> Result<TriggerResult, TriggerError> {
        let new = new_record
            .ok_or_else(|| TriggerError::ExecutionFailed("New record required".to_string()))?;

        let fk_user: Option<i64> = get_field(new, "fk_user");
        if fk_user.is_some() {
            return Ok(TriggerResult::new());
        }

        let notice_val: Option<String> = get_field(new, "notice");
        let valid_types = [
            "雇员-智体",
            "雇员-自然人",
            "组织-公司制企业",
            "组织-非公司制企业",
            "组织-非盈利机构",
            "公司-船运公司",
            "公司-轨道陆运",
            "公司-保险公司",
            "公司-公路车队",
            "公司-航空公司",
        ];
        match notice_val {
            Some(ref name) if valid_types.contains(&name.as_str()) => {}
            _ => return Ok(TriggerResult::new()),
        }

        let engine = TemplateEngine::new(ctx.pool.clone());
        let notice: Option<String> = get_field(new, "notice");
        let created_by_id: Option<i64> = get_field(new, "created_by_id");
        let updated_by_id: Option<i64> = get_field(new, "updated_by_id");
        let created_at: Option<chrono::DateTime<chrono::Utc>> = get_field(new, "created_at");
        let updated_at: Option<chrono::DateTime<chrono::Utc>> = get_field(new, "updated_at");

        let username = notice.clone().unwrap_or_default();
        let nickname = notice.unwrap_or_else(|| username.clone());

        let user_table = match ctx.app_container {
            crate::AppContainer::Meta => "isahl_meta.meta_user",
            crate::AppContainer::Gateway => "isahl_auth.auth_users",
        };

        let sql = format!(
            r#"
            INSERT INTO {} (
                created_by_id, updated_by_id, created_at, updated_at,
                username, nickname, app_lang, system_settings
            )
            VALUES ($1, $2, $3, $4, $5, $6, 'zh_CN', '{{}}')
            ON CONFLICT (username) DO UPDATE SET
                updated_by_id = EXCLUDED.updated_by_id,
                updated_at = EXCLUDED.updated_at
            RETURNING id
            "#,
            user_table
        );

        let user_id: i64 = engine
            .query_scalar(
                &sql,
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
                    Value::String(username),
                    Value::String(nickname),
                ],
            )
            .await?
            .ok_or_else(|| {
                TriggerError::ExecutionFailed("User creation returned no id".to_string())
            })?;

        Ok(TriggerResult::new().with_modified_field("fk_user", Value::Number(user_id.into())))
    }
}

// ============================================
// BusinessRegistryLoader
// ============================================

/// 业务触发器注册表加载器
///
/// 封装 `RegistryLoader`，提供业务域内置模板的批量注册，
/// 并支持从外部 config / DB 动态加载。
pub struct BusinessRegistryLoader {
    inner: RegistryLoader,
}

impl Default for BusinessRegistryLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl BusinessRegistryLoader {
    pub fn new() -> Self {
        Self {
            inner: RegistryLoader::new(),
        }
    }

    /// 注册所有**内置业务模板**（TriggerTemplate 接口实现）
    pub fn register_builtin_templates(&mut self) {
        // Dimension auto-code templates
        self.inner.register(
            "builtin:dimension",
            Arc::new(DimensionAutoCodeTemplate::scene()),
        );
        self.inner.register(
            "builtin:dimension",
            Arc::new(DimensionAutoCodeTemplate::factor()),
        );
        self.inner.register(
            "builtin:dimension",
            Arc::new(DimensionAutoCodeTemplate::function()),
        );

        // Project / Task templates
        self.inner
            .register("builtin:project", Arc::new(ProjectParticipantsTemplate));
        self.inner
            .register("builtin:task", Arc::new(TaskInitiatorTemplate));

        // Entity templates
        self.inner
            .register("builtin:entity", Arc::new(EntityUserTemplate));
        self.inner.register(
            "builtin:entity",
            Arc::new(crate::entity::EntityDefaultTemplate),
        );

        // Lifecycle _f_ / _t_ auto-derivation template
        self.inner.register(
            "builtin:lifecycle",
            Arc::new(crate::lifecycle::LifecycleBizTemplate),
        );

        // Product p_number templates
        self.inner.register(
            "builtin:product",
            Arc::new(crate::product::ProdPNumberTemplate::sales()),
        );
        self.inner.register(
            "builtin:product",
            Arc::new(crate::product::ProdPNumberTemplate::purchase()),
        );
        self.inner.register(
            "builtin:product",
            Arc::new(crate::product::ProdPNumberTemplate::request()),
        );
        self.inner.register(
            "builtin:product",
            Arc::new(crate::product::ProdPNumberTemplate::made()),
        );

        // BOM b_number template
        self.inner
            .register("builtin:bom", Arc::new(crate::bom::BomBNumberTemplate));

        // Operation / Process templates
        self.inner.register(
            "builtin:operation",
            Arc::new(crate::operation::OperationOpNumberTemplate),
        );
        self.inner.register(
            "builtin:process",
            Arc::new(crate::operation::ProcessPNumberTemplate),
        );

        // Version template
        self.inner.register(
            "builtin:version",
            Arc::new(crate::version::VersionHeadFlagTemplate),
        );
    }

    /// 小接口：注册任意业务模板
    pub fn register(
        &mut self,
        source: impl Into<String>,
        template: Arc<dyn TriggerTemplate>,
    ) -> Arc<dyn Trigger> {
        self.inner.register(source, template)
    }

    pub fn load_from_yaml(
        &mut self,
        path: &str,
    ) -> Result<Vec<Arc<dyn Trigger>>, crate::loader::LoaderError> {
        self.inner.load_from_yaml(path)
    }

    pub fn load_from_json(
        &mut self,
        path: &str,
    ) -> Result<Vec<Arc<dyn Trigger>>, crate::loader::LoaderError> {
        self.inner.load_from_json(path)
    }

    pub fn load_from_dir(
        &mut self,
        dir: &str,
    ) -> Result<Vec<Arc<dyn Trigger>>, crate::loader::LoaderError> {
        self.inner.load_from_dir(dir)
    }

    pub async fn load_from_db(
        &mut self,
        pool: &sqlx::PgPool,
    ) -> Result<Vec<Arc<dyn Trigger>>, crate::loader::LoaderError> {
        self.inner.load_from_db(pool, None).await
    }

    pub fn into_registry(self) -> crate::TriggerRegistry {
        self.inner.into_registry()
    }

    pub fn registry(&self) -> &crate::TriggerRegistry {
        self.inner.registry()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// 获取所有已注册的触发器句柄（用于 SmartTriggerRegistry 注册）
    pub fn handles(&self) -> &[Arc<dyn crate::Trigger>] {
        self.inner.handles()
    }
}

// ============================================
// Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dimension_auto_code_metadata() {
        let tpl = DimensionAutoCodeTemplate::scene();
        let meta = tpl.metadata();
        assert_eq!(meta.name, "tf_bf_ups_on_zc_id_scene");
        assert_eq!(meta.applies_to, vec!["zc_id_scene"]);
    }

    #[tokio::test]
    async fn test_dimension_auto_code_no_pool() {
        let tpl = DimensionAutoCodeTemplate::scene();
        let mut new = HashMap::new();
        new.insert("ck_category".to_string(), Value::Number(42i64.into()));

        let ctx = TriggerContext::new("zc_id_scene", crate::TriggerOperation::Insert);
        let result = tpl.execute(&ctx, None, Some(&new)).await.unwrap();

        assert_eq!(
            result.modified_fields.get("code"),
            Some(&Value::String("SC42".to_string()))
        );
    }
}
