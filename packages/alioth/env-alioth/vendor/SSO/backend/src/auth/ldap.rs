//! LDAP/Active Directory 认证模块
//!
//! 提供 LDAP 连接、用户搜索、认证和属性同步功能
//! 支持标准 LDAP 和 LDAPS (SSL/TLS) 协议

use ldap3::{LdapConn, LdapConnSettings, Scope, SearchEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;

/// LDAP 配置
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LdapConfig {
    /// LDAP 服务器 URL (ldap://host:port 或 ldaps://host:port)
    pub url: String,
    /// 绑定 DN (管理员账号)
    pub bind_dn: String,
    /// 绑定密码
    pub bind_password: String,
    /// 搜索基础 DN
    pub base_dn: String,
    /// 用户搜索过滤器模板 (如 "(sAMAccountName={})" 或 "(uid={})")
    pub user_filter: String,
    /// 属性映射配置
    pub attributes: LdapAttributes,
    /// 是否使用 LDAPS
    pub use_ldaps: bool,
    /// 连接超时(秒)
    pub timeout_secs: u64,
    /// 是否启用
    pub enabled: bool,
    /// 是否同步组信息
    pub sync_groups: bool,
    /// LDAP 组 DN 到 NGAC UA o_name 映射
    pub group_mapping: HashMap<String, String>,
}

impl Default for LdapConfig {
    fn default() -> Self {
        Self {
            url: "ldap://localhost:389".to_string(),
            bind_dn: String::new(),
            bind_password: String::new(),
            base_dn: String::new(),
            user_filter: "(sAMAccountName={})".to_string(),
            attributes: LdapAttributes::default(),
            use_ldaps: false,
            timeout_secs: 30,
            enabled: true,
            sync_groups: false,
            group_mapping: HashMap::new(),
        }
    }
}

/// LDAP 属性映射配置
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LdapAttributes {
    /// 用户名属性 (sAMAccountName, uid, cn 等)
    pub username: String,
    /// 邮箱属性
    pub email: String,
    /// 显示名称属性
    pub display_name: String,
    /// 组属性 (memberOf)
    pub groups: String,
}

impl Default for LdapAttributes {
    fn default() -> Self {
        Self {
            username: "sAMAccountName".to_string(),
            email: "mail".to_string(),
            display_name: "displayName".to_string(),
            groups: "memberOf".to_string(),
        }
    }
}

/// LDAP 用户信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapUser {
    /// 用户 DN
    pub dn: String,
    /// 用户名
    pub username: String,
    /// 邮箱
    pub email: Option<String>,
    /// 显示名称
    pub display_name: Option<String>,
    /// 所属组列表
    pub groups: Vec<String>,
    /// 原始属性 (用于调试和扩展)
    pub raw_attributes: std::collections::HashMap<String, Vec<String>>,
}

/// LDAP 错误类型
#[derive(Debug, Error)]
pub enum LdapError {
    #[error("连接失败: {0}")]
    ConnectionFailed(String),
    #[error("绑定失败: {0}")]
    BindFailed(String),
    #[error("用户未找到")]
    UserNotFound,
    #[error("凭据无效")]
    InvalidCredentials,
    #[error("搜索失败: {0}")]
    SearchFailed(String),
    #[error("超时")]
    Timeout,
    #[error("配置错误: {0}")]
    ConfigError(String),
    #[error("属性解析错误: {0}")]
    ParseError(String),
}

/// LDAP 客户端
pub struct LdapClient {
    config: LdapConfig,
    conn: LdapConn,
}

impl LdapClient {
    /// 创建新的 LDAP 客户端并建立连接
    pub fn new(config: LdapConfig) -> Result<Self, LdapError> {
        // 验证配置
        if config.url.is_empty() {
            return Err(LdapError::ConfigError("LDAP URL 不能为空".to_string()));
        }
        if config.bind_dn.is_empty() {
            return Err(LdapError::ConfigError("绑定 DN 不能为空".to_string()));
        }

        // 创建连接设置
        let settings = LdapConnSettings::new()
            .set_no_tls_verify(!config.use_ldaps) // 生产环境应验证证书
            .set_conn_timeout(Duration::from_secs(config.timeout_secs));

        // 建立连接。ldap3 sync 模式连接失败时在 crate 内部 panic（实测
        // ldap3-0.12.1 sync.rs:62 expect）而非返回 Err——catch_unwind 归一为
        // ConnectionFailed，保证「认证服务暂时不可用」500 兜底路径可达，
        // LDAP server 宕机不打崩 actix worker（fix-sso-id-default-heal 实测发现）。
        let conn = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            LdapConn::with_settings(settings, &config.url)
        })) {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => return Err(LdapError::ConnectionFailed(e.to_string())),
            Err(_) => {
                return Err(LdapError::ConnectionFailed(
                    "ldap3 panic during connect (server unreachable?)".to_string(),
                ))
            }
        };

        let mut client = Self { config, conn };

        // 使用绑定 DN 进行初始认证
        client.bind_admin()?;

        Ok(client)
    }

    /// 使用管理员账号绑定
    fn bind_admin(&mut self) -> Result<(), LdapError> {
        self.conn
            .simple_bind(&self.config.bind_dn, &self.config.bind_password)
            .map_err(|e| LdapError::BindFailed(e.to_string()))?
            .success()
            .map_err(|e| LdapError::BindFailed(format!("{:?}", e)))?;
        Ok(())
    }

    /// 搜索用户
    pub fn search_user(&mut self, username: &str) -> Result<Option<LdapUser>, LdapError> {
        // 构建搜索过滤器
        let filter = self
            .config
            .user_filter
            .replace("{}", username)
            .replace("{username}", username);

        // 构建属性列表
        let attrs = vec![
            &self.config.attributes.username as &str,
            &self.config.attributes.email,
            &self.config.attributes.display_name,
            &self.config.attributes.groups,
            "objectClass",
        ];

        // 执行搜索
        let (results, _) = self
            .conn
            .search(&self.config.base_dn, Scope::Subtree, &filter, attrs.clone())
            .map_err(|e| LdapError::SearchFailed(e.to_string()))?
            .success()
            .map_err(|e| LdapError::SearchFailed(format!("{:?}", e)))?;

        // 解析结果
        if results.is_empty() {
            return Ok(None);
        }

        let entry = SearchEntry::construct(results.into_iter().next().unwrap());
        let user = self.parse_entry(entry)?;

        Ok(Some(user))
    }

    /// 使用用户凭据进行认证
    pub fn authenticate(&mut self, username: &str, password: &str) -> Result<LdapUser, LdapError> {
        // 首先搜索用户获取 DN
        let user = self.search_user(username)?.ok_or(LdapError::UserNotFound)?;

        // 尝试使用用户 DN 和密码进行绑定
        let result = self
            .conn
            .simple_bind(&user.dn, password)
            .map_err(|e| LdapError::ConnectionFailed(e.to_string()))?;

        match result.success() {
            Ok(_) => Ok(user),
            Err(_) => Err(LdapError::InvalidCredentials),
        }
    }

    /// 解析 LDAP 条目为 LdapUser
    fn parse_entry(&self, entry: SearchEntry) -> Result<LdapUser, LdapError> {
        let attrs = &entry.attrs;

        // 提取用户名
        let username = attrs
            .get(&self.config.attributes.username)
            .and_then(|v| v.first())
            .cloned()
            .or_else(|| {
                // 尝试从 DN 中提取
                extract_cn_from_dn(&entry.dn)
            })
            .unwrap_or_else(|| entry.dn.clone());

        // 提取邮箱
        let email = attrs
            .get(&self.config.attributes.email)
            .and_then(|v| v.first())
            .cloned();

        // 提取显示名称
        let display_name = attrs
            .get(&self.config.attributes.display_name)
            .and_then(|v| v.first())
            .cloned();

        // 提取组信息
        let groups = if self.config.sync_groups {
            attrs
                .get(&self.config.attributes.groups)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|dn| extract_cn_from_dn(&dn))
                .collect()
        } else {
            Vec::new()
        };

        // 构建原始属性映射
        let raw_attributes: std::collections::HashMap<String, Vec<String>> = attrs
            .iter()
            .map(|(k, v): (&String, &Vec<String>)| (k.clone(), v.clone()))
            .collect();

        Ok(LdapUser {
            dn: entry.dn,
            username,
            email,
            display_name,
            groups,
            raw_attributes,
        })
    }

    /// 测试 LDAP 连接
    pub fn test_connection(&mut self) -> Result<(), LdapError> {
        self.bind_admin()
    }

    /// 获取配置
    pub fn config(&self) -> &LdapConfig {
        &self.config
    }
}

/// 从 DN 中提取 CN 值
fn extract_cn_from_dn(dn: &str) -> Option<String> {
    dn.split(',')
        .find(|part| part.trim().starts_with("CN=") || part.trim().starts_with("cn="))
        .map(|cn_part| {
            cn_part
                .trim()
                .trim_start_matches("CN=")
                .trim_start_matches("cn=")
                .to_string()
        })
}

/// LDAP 认证器 - 管理多个 LDAP 配置
pub struct LdapAuthenticator {
    configs: Vec<LdapConfig>,
}

impl LdapAuthenticator {
    /// 创建新的认证器
    pub fn new(configs: Vec<LdapConfig>) -> Self {
        Self { configs }
    }

    /// 添加配置
    pub fn add_config(&mut self, config: LdapConfig) {
        self.configs.push(config);
    }

    /// 使用所有配置尝试认证
    pub fn authenticate(&self, username: &str, password: &str) -> Result<LdapUser, LdapError> {
        let mut last_error = None;

        for config in &self.configs {
            if !config.enabled {
                continue;
            }

            match LdapClient::new(config.clone()) {
                Ok(mut client) => match client.authenticate(username, password) {
                    Ok(user) => return Ok(user),
                    Err(e) => {
                        // 如果是用户未找到，继续尝试下一个配置
                        if !matches!(e, LdapError::UserNotFound) {
                            last_error = Some(e);
                        }
                    }
                },
                Err(e) => {
                    log::warn!("LDAP 连接失败: {}", e);
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or(LdapError::UserNotFound))
    }

    /// 测试所有配置
    pub fn test_all_configs(&self) -> Vec<(String, Result<(), LdapError>)> {
        self.configs
            .iter()
            .filter(|c| c.enabled)
            .map(|config| {
                let name = config.url.clone();
                let result =
                    LdapClient::new(config.clone()).and_then(|mut client| client.test_connection());
                (name, result)
            })
            .collect()
    }
}

// HTTP API 请求/响应类型
use super::{
    jwt::{encode_access_token, encode_refresh_token, set_refresh_cookie, Claims},
    session::{CreateSessionRequest, SessionManager},
    AuthState,
};
use actix_web::{web, HttpRequest, HttpResponse};
use sqlx::PgPool;

/// LDAP 登录请求
#[derive(Debug, Deserialize)]
pub struct LdapLoginRequest {
    pub username: String,
    pub password: String,
    pub domain: Option<String>, // 可选的域选择
}

/// LDAP 登录响应
#[derive(Debug, Serialize)]
pub struct LdapLoginResponse {
    pub access_token: String,
    pub user: LdapUserInfo,
}

/// LDAP 用户信息
#[derive(Debug, Serialize)]
pub struct LdapUserInfo {
    pub id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub groups: Vec<String>,
    pub is_ldap_user: bool,
}

/// LDAP 配置响应 (隐藏敏感信息)
#[derive(Debug, Serialize)]
pub struct LdapConfigResponse {
    pub id: String,
    pub name: String,
    pub url: String,
    pub base_dn: String,
    pub user_filter: String,
    pub use_ldaps: bool,
    pub enabled: bool,
    pub sync_groups: bool,
}

/// LDAP 测试请求
#[derive(Debug, Deserialize)]
pub struct LdapTestRequest {
    pub config: LdapConfig,
}

/// LDAP 测试结果
#[derive(Debug, Serialize)]
pub struct LdapTestResponse {
    pub success: bool,
    pub message: String,
}

/// 错误响应
#[derive(Debug, Serialize)]
pub struct LdapErrorResponse {
    pub error: String,
}

/// LDAP 登录处理
pub async fn ldap_login(
    pool: web::Data<PgPool>,
    state: web::Data<AuthState>,
    req: HttpRequest,
    body: web::Json<LdapLoginRequest>,
) -> HttpResponse {
    // 从数据库获取 LDAP 配置
    let configs = match load_ldap_configs(pool.get_ref()).await {
        Ok(configs) => configs,
        Err(e) => {
            log::error!("Failed to load LDAP configs: {}", e);
            return HttpResponse::InternalServerError().json(LdapErrorResponse {
                error: "LDAP 配置加载失败".to_string(),
            });
        }
    };

    if configs.is_empty() {
        return HttpResponse::ServiceUnavailable().json(LdapErrorResponse {
            error: "LDAP 认证未配置".to_string(),
        });
    }

    // 提取 NGAC 映射配置（configs 即将被 move）
    let ngac_config = configs
        .iter()
        .find(|c| !c.group_mapping.is_empty())
        .cloned();

    // 创建认证器
    let authenticator = LdapAuthenticator::new(configs);

    // 尝试认证
    let ldap_user = match authenticator.authenticate(&body.username, &body.password) {
        Ok(user) => user,
        Err(LdapError::UserNotFound) => {
            return HttpResponse::Unauthorized().json(LdapErrorResponse {
                error: "用户不存在".to_string(),
            });
        }
        Err(LdapError::InvalidCredentials) => {
            return HttpResponse::Unauthorized().json(LdapErrorResponse {
                error: "用户名或密码错误".to_string(),
            });
        }
        Err(e) => {
            log::error!("LDAP authentication error: {}", e);
            return HttpResponse::InternalServerError().json(LdapErrorResponse {
                error: "认证服务暂时不可用".to_string(),
            });
        }
    };

    // 同步用户到本地数据库
    let user_id = match sync_ldap_user(pool.get_ref(), &ldap_user).await {
        Ok(id) => id,
        Err(e) => {
            log::error!("Failed to sync LDAP user: {}", e);
            return HttpResponse::InternalServerError().json(LdapErrorResponse {
                error: "用户同步失败".to_string(),
            });
        }
    };

    // 同步 LDAP 组到 NGAC 用户属性
    if let Some(ldap_config) = &ngac_config {
        if let Err(e) =
            sync_ldap_ngac_attributes(pool.get_ref(), &ldap_user, user_id, ldap_config).await
        {
            log::warn!("Failed to sync LDAP group to NGAC UA: {}", e);
        }
    }

    // 创建会话
    let session_manager = SessionManager::new(pool.get_ref().clone());
    let _session = match session_manager
        .create_session(CreateSessionRequest {
            user_id,
            ip_address: get_client_ip(&req),
            user_agent: get_user_agent(&req),
            ..Default::default()
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to create session: {}", e);
            return HttpResponse::InternalServerError().json(LdapErrorResponse {
                error: "会话创建失败".to_string(),
            });
        }
    };

    // 生成 JWT token
    let claims = Claims::with_expiry_seconds(
        &user_id.to_string(),
        "",
        false,
        state.jwt_access_expiry_secs,
    );

    let access_token = match encode_access_token(&claims, &state.jwt_private_key) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::InternalServerError().json(LdapErrorResponse {
                error: "Token 生成失败".to_string(),
            })
        }
    };

    let refresh_token = match encode_refresh_token(
        &claims,
        &state.jwt_private_key,
        state.jwt_refresh_expiry_secs,
    ) {
        Ok(t) => t,
        Err(_) => {
            return HttpResponse::InternalServerError().json(LdapErrorResponse {
                error: "Refresh token 生成失败".to_string(),
            })
        }
    };

    let response = HttpResponse::Ok().json(LdapLoginResponse {
        access_token,
        user: LdapUserInfo {
            id: user_id.to_string(),
            username: ldap_user.username.clone(),
            email: ldap_user.email.clone(),
            display_name: ldap_user.display_name.clone(),
            groups: ldap_user.groups.clone(),
            is_ldap_user: true,
        },
    });

    set_refresh_cookie(response, &refresh_token, state.jwt_refresh_expiry_secs)
}

/// 获取 LDAP 配置列表 (Admin only)
pub async fn list_ldap_configs(pool: web::Data<PgPool>) -> HttpResponse {
    match load_ldap_configs_safe(pool.get_ref()).await {
        Ok(configs) => HttpResponse::Ok().json(configs),
        Err(e) => {
            log::error!("Failed to load LDAP configs: {}", e);
            HttpResponse::InternalServerError().json(LdapErrorResponse {
                error: "配置加载失败".to_string(),
            })
        }
    }
}

/// 测试 LDAP 连接 (Admin only)
pub async fn test_ldap_connection(body: web::Json<LdapTestRequest>) -> HttpResponse {
    let mut client = match LdapClient::new(body.config.clone()) {
        Ok(client) => client,
        Err(e) => {
            return HttpResponse::Ok().json(LdapTestResponse {
                success: false,
                message: format!("连接失败: {}", e),
            });
        }
    };

    match client.test_connection() {
        Ok(_) => HttpResponse::Ok().json(LdapTestResponse {
            success: true,
            message: "连接成功".to_string(),
        }),
        Err(e) => HttpResponse::Ok().json(LdapTestResponse {
            success: false,
            message: format!("连接测试失败: {}", e),
        }),
    }
}

/// 从数据库加载 LDAP 配置
async fn load_ldap_configs(pool: &PgPool) -> Result<Vec<LdapConfig>, sqlx::Error> {
    use sqlx::Row;

    let rows = sqlx::query(
        r#"
        SELECT 
            url, bind_dn, bind_password, base_dn, user_filter,
            username_attr, email_attr, display_name_attr, groups_attr,
            use_ldaps, timeout_secs, enabled, sync_groups, group_mapping
        FROM isahl_auth.ldap_configs
        WHERE enabled = true
        ORDER BY created_at
        "#,
    )
    .fetch_all(pool)
    .await?;

    let configs: Vec<LdapConfig> = rows
        .into_iter()
        .map(|row| LdapConfig {
            url: row.try_get("url").unwrap_or_default(),
            bind_dn: row.try_get("bind_dn").unwrap_or_default(),
            bind_password: row.try_get("bind_password").unwrap_or_default(),
            base_dn: row.try_get("base_dn").unwrap_or_default(),
            user_filter: row.try_get("user_filter").unwrap_or_default(),
            attributes: LdapAttributes {
                username: row.try_get("username_attr").unwrap_or_default(),
                email: row.try_get("email_attr").unwrap_or_default(),
                display_name: row.try_get("display_name_attr").unwrap_or_default(),
                groups: row.try_get("groups_attr").unwrap_or_default(),
            },
            use_ldaps: row.try_get("use_ldaps").unwrap_or(false),
            timeout_secs: row.try_get::<i32, _>("timeout_secs").unwrap_or(30) as u64,
            enabled: row.try_get("enabled").unwrap_or(true),
            sync_groups: row.try_get("sync_groups").unwrap_or(false),
            group_mapping: row
                .try_get::<serde_json::Value, _>("group_mapping")
                .ok()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default(),
        })
        .collect();

    Ok(configs)
}

/// 加载 LDAP 配置 (安全版本，隐藏密码)
async fn load_ldap_configs_safe(pool: &PgPool) -> Result<Vec<LdapConfigResponse>, sqlx::Error> {
    use sqlx::Row;

    let rows = sqlx::query(
        r#"
        SELECT 
            id, name, url, base_dn, user_filter,
            use_ldaps, enabled, sync_groups
        FROM isahl_auth.ldap_configs
        ORDER BY created_at
        "#,
    )
    .fetch_all(pool)
    .await?;

    let configs: Vec<LdapConfigResponse> = rows
        .into_iter()
        .map(|row| {
            let id: i64 = row.try_get("id").unwrap_or_default();
            LdapConfigResponse {
                id: id.to_string(),
                name: row.try_get("name").unwrap_or_default(),
                url: row.try_get("url").unwrap_or_default(),
                base_dn: row.try_get("base_dn").unwrap_or_default(),
                user_filter: row.try_get("user_filter").unwrap_or_default(),
                use_ldaps: row.try_get("use_ldaps").unwrap_or(false),
                enabled: row.try_get("enabled").unwrap_or(true),
                sync_groups: row.try_get("sync_groups").unwrap_or(false),
            }
        })
        .collect();

    Ok(configs)
}

/// 同步 LDAP 用户到本地数据库
async fn sync_ldap_user(pool: &PgPool, ldap_user: &LdapUser) -> Result<i64, sqlx::Error> {
    use sqlx::Row;

    // 首先检查用户是否已存在
    let existing_id: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT id FROM isahl_auth.auth_users 
        WHERE username = $1 AND is_ldap_user = true
        "#,
    )
    .bind(&ldap_user.username)
    .fetch_optional(pool)
    .await?;

    if let Some(id) = existing_id {
        // 更新现有用户
        sqlx::query(
            r#"
            UPDATE isahl_auth.auth_users 
            SET email = $1, display_name = $2, ldap_dn = $3, updated_at = NOW()
            WHERE id = $4
            "#,
        )
        .bind(&ldap_user.email)
        .bind(&ldap_user.display_name)
        .bind(&ldap_user.dn)
        .bind(id)
        .execute(pool)
        .await?;

        Ok(id)
    } else {
        // 创建新用户
        let row = sqlx::query(
            r#"
            INSERT INTO isahl_auth.auth_users (
                id, name, username, email, display_name, 
                is_ldap_user, ldap_dn, created_at, updated_at
            )
            VALUES (
                isahl.gen_next_zuid(), $1, $1, $2, $3, true, $4, NOW(), NOW()
            )
            RETURNING id
            "#,
        )
        .bind(&ldap_user.username)
        .bind(&ldap_user.email)
        .bind(&ldap_user.display_name)
        .bind(&ldap_user.dn)
        .fetch_one(pool)
        .await?;

        let id: i64 = row.try_get(0)?;
        Ok(id)
    }
}

/// 同步 LDAP 组到 NGAC 用户属性
async fn sync_ldap_ngac_attributes(
    pool: &PgPool,
    ldap_user: &LdapUser,
    user_id: i64,
    config: &LdapConfig,
) -> Result<(), sqlx::Error> {
    // 从用户的原始 LDAP 属性中获取组 DN 列表
    let raw_groups = match ldap_user.raw_attributes.get(&config.attributes.groups) {
        Some(groups) => groups,
        None => return Ok(()), // 没有组属性
    };

    if raw_groups.is_empty() || config.group_mapping.is_empty() {
        return Ok(());
    }

    // 获取或创建默认 policy class
    let default_pc_id: i64 = match sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM isahl_auth.ngac_policy_class ORDER BY id LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await?
    {
        Some(id) => id,
        None => {
            sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl_auth.ngac_policy_class (id, o_name) VALUES (isahl.gen_next_zuid(), 'default') RETURNING id"#,
            )
            .fetch_one(pool)
            .await?
        }
    };

    for group_dn in raw_groups {
        // 查找组 DN 是否在映射表中
        let ua_o_name = match config.group_mapping.get(group_dn) {
            Some(name) => name,
            None => continue,
        };

        // 查找或创建 ngac_user_attribute
        let ua_id: i64 = match sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id FROM isahl_auth.ngac_user_attribute
            WHERE o_name = $1 AND deleted_at IS NULL
            LIMIT 1
            "#,
        )
        .bind(ua_o_name)
        .fetch_optional(pool)
        .await?
        {
            Some(id) => id,
            None => {
                // 属性图写一致性：统一校验入口（空层级恒过；未来写真实层级时防环/跨域/悬空父）
                crate::ngac::integrity::validate_ancestors(
                    pool,
                    "user_attribute",
                    None,
                    &[],
                    Some(default_pc_id),
                )
                .await
                .map_err(sqlx::Error::Protocol)?;
                sqlx::query_scalar::<_, i64>(
                    r#"
                    INSERT INTO isahl_auth.ngac_user_attribute (id, o_name, fk_policy_class, ancestor_ids, children_ids)
                    VALUES (isahl.gen_next_zuid(), $1, $2, '{}'::bigint[], '{}'::bigint[])
                    RETURNING id
                    "#,
                )
                .bind(ua_o_name)
                .bind(default_pc_id)
                .fetch_one(pool)
                .await?
            }
        };

        // 关联用户与属性 (ON CONFLICT DO UPDATE 确保幂等)
        sqlx::query(
            r#"
            INSERT INTO isahl_auth.ngac_user_rr_attribute
                (id, fk_user, fk_user_attribute, o_name)
            VALUES
                (isahl.gen_next_zuid(), $1, $2, $3)
            ON CONFLICT (fk_user, fk_user_attribute)
            DO UPDATE SET updated_at = NOW(), deleted_at = NULL
            "#,
        )
        .bind(user_id)
        .bind(ua_id)
        .bind(ua_o_name)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// 获取客户端 IP
fn get_client_ip(req: &HttpRequest) -> Option<String> {
    req.connection_info()
        .realip_remote_addr()
        .map(|s| s.to_string())
}

/// 获取 User Agent
fn get_user_agent(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_cn_from_dn() {
        assert_eq!(
            extract_cn_from_dn("CN=John Doe,OU=Users,DC=example,DC=com"),
            Some("John Doe".to_string())
        );
        assert_eq!(
            extract_cn_from_dn("cn=admin,dc=example,dc=com"),
            Some("admin".to_string())
        );
        assert_eq!(
            extract_cn_from_dn("uid=john,ou=users,dc=example,dc=com"),
            None
        );
    }

    #[test]
    fn test_ldap_config_default() {
        let config = LdapConfig::default();
        assert_eq!(config.url, "ldap://localhost:389");
        assert_eq!(config.user_filter, "(sAMAccountName={})");
        assert_eq!(config.attributes.username, "sAMAccountName");
        assert!(config.enabled);
    }

    #[test]
    fn test_ldap_attributes_default() {
        let attrs = LdapAttributes::default();
        assert_eq!(attrs.username, "sAMAccountName");
        assert_eq!(attrs.email, "mail");
        assert_eq!(attrs.display_name, "displayName");
        assert_eq!(attrs.groups, "memberOf");
    }
}
