//! System Config Models
//!
//! 系统配置数据模型（由应用层 Repository 实现绑定物理表，Gateway 绑定活库 isahl.zc_id_prot-*_config 族表）。
//! 提供通用配置管理能力，支持 LLM、Email、IM 等外部服务接入配置。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ============================================
// Core Entity
// ============================================

/// 系统配置实体
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SystemConfig {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 配置显示名称（物理列 notice）
    pub notice: Option<String>,
    /// 配置编码 / 唯一标识（物理列 code）
    pub code: Option<String>,
    /// 配置大类（物理列 _f_）: llm, email, im, webhook, storage, sms
    #[sqlx(rename = "_f_")]
    pub _f_: Option<String>,
    /// 提供商类型（物理列 _t_）: openai, anthropic, smtp, wecom, dingtalk, feishu
    #[sqlx(rename = "_t_")]
    pub _t_: Option<String>,
    /// 配置描述（物理列 comments）
    pub comments: Option<String>,
    /// 敏感凭证（加密存储的 JSONB）
    pub credentials: Option<serde_json::Value>,
    /// 非敏感设置（明文 JSONB）
    pub settings: Option<serde_json::Value>,
    /// 是否启用
    pub enabled: Option<bool>,
    /// 是否为默认配置
    #[sqlx(rename = "is_default")]
    pub is_default: Option<bool>,
    /// 作用域
    pub domain_: Option<String>,
    /// 是否公开
    pub public: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(with = "common::serde_zuid::opt")]
    pub created_by_id: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub updated_by_id: Option<i64>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl crud::Identifiable for SystemConfig {
    fn id(&self) -> i64 {
        self.id
    }
}

// ============================================
// Request Types
// ============================================

/// 创建系统配置请求
#[derive(Debug, Clone, Deserialize)]
pub struct CreateSystemConfigRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    #[serde(rename = "_f_")]
    pub _f_: Option<String>,
    #[serde(rename = "_t_")]
    pub _t_: Option<String>,
    pub comments: Option<String>,
    /// 敏感凭证（服务层会自动加密）
    pub credentials: Option<serde_json::Value>,
    /// 非敏感设置
    pub settings: Option<serde_json::Value>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub is_default: bool,
    pub domain_: Option<String>,
    #[serde(default)]
    pub public: bool,
}

/// 更新系统配置请求
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSystemConfigRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    #[serde(rename = "_f_")]
    pub _f_: Option<String>,
    #[serde(rename = "_t_")]
    pub _t_: Option<String>,
    pub comments: Option<String>,
    /// 敏感凭证（服务层会自动加密）
    pub credentials: Option<serde_json::Value>,
    /// 非敏感设置
    pub settings: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub is_default: Option<bool>,
    pub domain_: Option<String>,
    pub public: Option<bool>,
}

fn default_true() -> bool {
    true
}

// ============================================
// Schema Types (for frontend dynamic form generation)
// ============================================

/// 配置分类元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigCategory {
    pub code: String,
    pub notice: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub providers: Vec<ConfigProvider>,
}

/// 配置提供商元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigProvider {
    pub code: String,
    pub notice: String,
    pub description: Option<String>,
    /// 该提供商对应的字段 Schema
    pub schema: Vec<ConfigFieldSchema>,
    /// 供应商预设默认值（如 LLM 的 base_url/model/flash_model），前端按供应商自动填充；
    /// None 表示无预设（用户全手工）。单一真相源为对应后端 crate 的 DEFAULT_* 常量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<serde_json::Value>,
}

/// 配置字段 Schema（驱动前端动态表单）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFieldSchema {
    pub key: String,
    pub notice: String,
    /// 字段类型: text, password, number, url, textarea, select, boolean
    pub field_type: String,
    pub required: bool,
    pub placeholder: Option<String>,
    pub default_value: Option<serde_json::Value>,
    /// select 类型时的选项
    pub options: Option<Vec<SelectOption>>,
    /// 是否属于敏感字段（需要加密存储）
    pub sensitive: bool,
    /// 字段说明
    pub help_text: Option<String>,
}

/// 下拉选项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub notice: String,
}

/// 配置验证结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

/// 配置分类列表响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigCategoryListResponse {
    pub categories: Vec<ConfigCategory>,
}

/// 带解敏凭证的配置响应（返回给前端时隐藏敏感字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfigSafeResponse {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub code: Option<String>,
    #[serde(rename = "_f_")]
    pub _f_: Option<String>,
    #[serde(rename = "_t_")]
    pub _t_: Option<String>,
    pub comments: Option<String>,
    /// 敏感字段标记为已设置（不返回真实值）
    pub credentials_set: bool,
    /// 非敏感设置（明文返回）
    pub settings: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub is_default: Option<bool>,
    pub domain_: Option<String>,
    pub public: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<SystemConfig> for SystemConfigSafeResponse {
    fn from(config: SystemConfig) -> Self {
        let credentials_set = config
            .credentials
            .as_ref()
            .map(|v| !v.is_null() && v.as_object().map(|o| !o.is_empty()).unwrap_or(false))
            .unwrap_or(false);
        Self {
            id: config.id,
            notice: config.notice,
            code: config.code,
            _f_: config._f_,
            _t_: config._t_,
            comments: config.comments,
            credentials_set,
            settings: config.settings,
            enabled: config.enabled,
            is_default: config.is_default,
            domain_: config.domain_,
            public: config.public,
            created_at: config.created_at,
            updated_at: config.updated_at,
        }
    }
}
