//! System Config Service
//!
//! 业务逻辑层：负责配置验证、敏感字段加解密、Schema 查询等。
//! 泛型实现，不绑定具体 Repository 类型。

use crate::crypto;
use crate::models::{
    ConfigCategoryListResponse, CreateSystemConfigRequest, SystemConfig, SystemConfigSafeResponse,
    UpdateSystemConfigRequest,
};
use crate::repository::SystemConfigRepository;
use crate::schema;

pub struct SystemConfigService<R> {
    repo: R,
}

impl<R> SystemConfigService<R>
where
    R: SystemConfigRepository,
{
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    // ============================================
    // Schema queries (no DB required)
    // ============================================

    pub fn get_categories() -> ConfigCategoryListResponse {
        ConfigCategoryListResponse {
            categories: schema::get_all_categories(),
        }
    }

    pub fn get_category(code: &str) -> Option<crate::models::ConfigCategory> {
        schema::get_category(code)
    }

    // ============================================
    // CRUD Operations (with encryption)
    // ============================================

    pub async fn create(
        &self,
        mut req: CreateSystemConfigRequest,
    ) -> Result<SystemConfigSafeResponse, SystemConfigError> {
        self.validate_request(&req).await?;

        // validate_request 已确保 _f_ / _t_ 非空
        let _f_ = req._f_.as_deref().unwrap();
        let _t_ = req._t_.as_deref().unwrap();

        // 加密敏感字段
        if let Some(ref mut creds) = req.credentials {
            let keys = schema::get_sensitive_keys(_f_, _t_);
            let keys_ref: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
            if !keys_ref.is_empty() {
                crypto::encrypt_json_fields(creds, &keys_ref).map_err(SystemConfigError::Crypto)?;
            }
        }

        let config = self
            .repo
            .insert(&req)
            .await
            .map_err(SystemConfigError::Database)?;
        Ok(SystemConfigSafeResponse::from(config))
    }

    pub async fn update(
        &self,
        id: i64,
        mut req: UpdateSystemConfigRequest,
    ) -> Result<Option<SystemConfigSafeResponse>, SystemConfigError> {
        let existing = self
            .repo
            .find_by_id(id)
            .await
            .map_err(SystemConfigError::Database)?
            .ok_or(SystemConfigError::NotFound(id))?;

        let _f_ = req
            ._f_
            .as_deref()
            .or(existing._f_.as_deref())
            .unwrap_or("")
            .to_string();
        let _t_ = req
            ._t_
            .as_deref()
            .or(existing._t_.as_deref())
            .unwrap_or("")
            .to_string();

        req._f_ = Some(_f_.clone());
        req._t_ = Some(_t_.clone());

        if let Some(ref mut creds) = req.credentials {
            let keys = schema::get_sensitive_keys(&_f_, &_t_);
            let keys_ref: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
            if !keys_ref.is_empty() {
                crypto::encrypt_json_fields(creds, &keys_ref).map_err(SystemConfigError::Crypto)?;
            }
        }

        let config = self
            .repo
            .update(id, &req)
            .await
            .map_err(SystemConfigError::Database)?;
        Ok(config.map(SystemConfigSafeResponse::from))
    }

    pub async fn delete(&self, id: i64) -> Result<u64, SystemConfigError> {
        self.repo
            .soft_delete(id)
            .await
            .map_err(SystemConfigError::Database)
    }

    pub async fn find_by_id(
        &self,
        id: i64,
    ) -> Result<Option<SystemConfigSafeResponse>, SystemConfigError> {
        let config = self
            .repo
            .find_by_id(id)
            .await
            .map_err(SystemConfigError::Database)?;
        Ok(config.map(SystemConfigSafeResponse::from))
    }

    pub async fn list(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SystemConfigSafeResponse>, SystemConfigError> {
        let configs = self
            .repo
            .list(limit, offset)
            .await
            .map_err(SystemConfigError::Database)?;
        Ok(configs
            .into_iter()
            .map(SystemConfigSafeResponse::from)
            .collect())
    }

    // ============================================
    // Decryption API (for internal service use)
    // ============================================

    pub async fn get_full_config(
        &self,
        id: i64,
    ) -> Result<Option<SystemConfig>, SystemConfigError> {
        let mut config = self
            .repo
            .find_by_id(id)
            .await
            .map_err(SystemConfigError::Database)?;
        if let Some(ref mut c) = config {
            self.decrypt_credentials(c)?;
        }
        Ok(config)
    }

    pub async fn get_full_config_by_code(
        &self,
        code: &str,
    ) -> Result<Option<SystemConfig>, SystemConfigError> {
        let mut config = self
            .repo
            .find_by_code(code)
            .await
            .map_err(SystemConfigError::Database)?;
        if let Some(ref mut c) = config {
            self.decrypt_credentials(c)?;
        }
        Ok(config)
    }

    // ============================================
    // Helpers
    // ============================================

    fn decrypt_credentials(&self, config: &mut SystemConfig) -> Result<(), SystemConfigError> {
        if let Some(ref mut creds) = config.credentials {
            let keys = schema::get_sensitive_keys(
                config._f_.as_deref().unwrap_or(""),
                config._t_.as_deref().unwrap_or(""),
            );
            let keys_ref: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
            if !keys_ref.is_empty() {
                crypto::decrypt_json_fields(creds, &keys_ref).map_err(SystemConfigError::Crypto)?;
            }
        }
        Ok(())
    }

    async fn validate_request(
        &self,
        req: &CreateSystemConfigRequest,
    ) -> Result<(), SystemConfigError> {
        let (_f_, _t_) = match (req._f_.as_deref(), req._t_.as_deref()) {
            (Some(f), Some(t)) if !f.is_empty() && !t.is_empty() => (f.to_string(), t.to_string()),
            _ => {
                return Err(SystemConfigError::Validation(
                    "_f_ 和 _t_ 不能为空".to_string(),
                ))
            }
        };

        let category = schema::get_category(&_f_);
        if category.is_none() {
            return Err(SystemConfigError::Validation(format!(
                "不支持的配置分类: {}",
                _f_
            )));
        }
        let provider_exists = category.unwrap().providers.iter().any(|p| p.code == _t_);
        if !provider_exists {
            return Err(SystemConfigError::Validation(format!(
                "分类 {} 下不支持的提供商: {}",
                _f_, _t_
            )));
        }

        if let Some(ref code) = req.code {
            let existing = self
                .repo
                .find_by_code(code)
                .await
                .map_err(SystemConfigError::Database)?;
            if existing.is_some() {
                return Err(SystemConfigError::Validation(format!(
                    "配置编码 '{}' 已存在",
                    code
                )));
            }
        }

        Ok(())
    }
}

// ============================================
// Error Type
// ============================================

#[derive(Debug, thiserror::Error)]
pub enum SystemConfigError {
    #[error("数据库错误: {0}")]
    Database(#[from] sqlx::Error),

    #[error("加密错误: {0}")]
    Crypto(#[from] anyhow::Error),

    #[error("配置不存在: id={0}")]
    NotFound(i64),

    #[error("验证失败: {0}")]
    Validation(String),
}
