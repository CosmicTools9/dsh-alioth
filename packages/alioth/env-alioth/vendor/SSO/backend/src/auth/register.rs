//! User registration handler
//!
//! Provides HTTP handlers for user registration

use actix_web::{web, HttpRequest, HttpResponse};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::{
    jwt::{self, set_refresh_cookie, Claims},
    password::{self, hash_password_async},
    session::{CreateSessionRequest, SessionManager},
    AuthState,
};
/// Registration request body
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    /// 账号（身份唯一基点）；与 email 至少其一必填
    pub username: Option<String>,
    pub password: String,
    /// 可选认证/联系方式（可多个，非唯一基点）
    pub email: Option<String>,
    /// 外部主体标记（OpenActivity 外部协同门户注册传入）；仅允许 "external"，其余显式值 400
    #[serde(default)]
    pub user_type: Option<String>,
}

/// Registration response
#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user_id: String,
    pub email: String,
    pub status: String,
    /// 注册通道（standard / external——通道即身份语义）
    pub channel: String,
    /// 自动触发的审批实例 id（oper-approve）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_instance_id: Option<String>,
}

/// 注册通道描述符（add-dual-register-channels）：通道即身份语义——身份类型由通道决定；
/// 审批事件/实例 code、绑定流程、事件叶表与文案模板四要素 MUST 取自
/// `common::registration_approval` 映射表（wire-registration-approval-closure D3；
/// MUST NOT 在本文件复制字面值——Gateway 启动自检按同一表决定断链重建目标）。
pub struct RegisterChannel {
    /// 身份类型（auth_users.user_type）
    pub kind: &'static str,
    /// 审批通道规格（event_code / flow_code / leaf_table / notice、comments 模板）
    pub spec: &'static common::registration_approval::ApprovalChannelSpec,
}

/// 内部通道（Gateway 注册页）：访问授权审批，FLOW-AUTHORIZATION（现状语义）
pub const INTERNAL_CHANNEL: RegisterChannel = RegisterChannel {
    kind: "standard",
    spec: &common::registration_approval::INTERNAL_REGISTRATION_CHANNEL,
};

/// 外部主体通道（OpenActivity 门户）：外部主体入驻审批，独立流程
pub const EXTERNAL_CHANNEL: RegisterChannel = RegisterChannel {
    kind: "external",
    spec: &common::registration_approval::EXTERNAL_SUBJECT_CHANNEL,
};

/// Error response
#[derive(Debug, Serialize)]
pub struct AuthError {
    pub error: String,
}
/// 内部注册通道（Gateway 注册页）：仅允许缺省/standard 身份；
/// 显式 external 一律 400 引导外部通道——内部端点不产外部账号。
///
/// POST /auth/register
pub async fn register(
    req: HttpRequest,
    body: web::Json<RegisterRequest>,
    pool: web::Data<PgPool>,
    auth_state: web::Data<AuthState>,
) -> Result<HttpResponse, actix_web::Error> {
    let channel = match body.user_type.as_deref().map(str::trim) {
        None | Some("") => &INTERNAL_CHANNEL,
        Some("external") => {
            return Ok(HttpResponse::BadRequest().json(AuthError {
                error: "外部主体注册必须走 /auth/register/external".to_string(),
            }));
        }
        Some(other) => {
            return Ok(HttpResponse::BadRequest().json(AuthError {
                error: format!("user_type '{other}' not allowed"),
            }));
        }
    };
    register_core(req, &body, &pool, &auth_state, channel).await
}

/// 外部主体注册通道（OpenActivity 外部协同门户专用）：服务端强制
/// user_type='external'（请求体身份键不参与判定）；绑定外部入驻审批流
/// FLOW-EXTERNAL-SUBJECT 与独立审批事件 code；独立更严限流档。
///
/// POST /auth/register/external
pub async fn register_external(
    req: HttpRequest,
    body: web::Json<RegisterRequest>,
    pool: web::Data<PgPool>,
    auth_state: web::Data<AuthState>,
) -> Result<HttpResponse, actix_web::Error> {
    register_core(req, &body, &pool, &auth_state, &EXTERNAL_CHANNEL).await
}

/// 注册核心（通道参数化）。
async fn register_core(
    req: HttpRequest,
    body: &web::Json<RegisterRequest>,
    pool: &web::Data<PgPool>,
    auth_state: &web::Data<AuthState>,
    channel: &RegisterChannel,
) -> Result<HttpResponse, actix_web::Error> {
    let email: Option<String> = body.email.as_ref().map(|e| e.trim().to_lowercase());
    let email_str = email.clone().unwrap_or_default();
    let password = &body.password;

    // 账号（唯一基点）与 email（可选认证链路）至少其一必填；username 缺省由 email local part 派生
    let username = body
        .username
        .clone()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .or_else(|| {
            email
                .as_ref()
                .and_then(|e| e.split('@').next().map(|s| s.to_string()))
        });
    let Some(username) = username.filter(|u| !u.is_empty()) else {
        return Ok(HttpResponse::BadRequest().json(AuthError {
            error: "Username or email is required".to_string(),
        }));
    };

    // email 若提供：仅校验格式（可选联系方式，非身份门禁）。
    // allow-duplicate-email-accounts：MUST NOT 校验全局唯一性——同一 email MAY 属于多个账号，
    // 邮箱冲突 MUST NOT 阻断注册（注册的唯一身份门禁 = username 唯一性，见下）。
    if let Some(e) = &email {
        let at_pos = e.find('@');
        let dot_pos = e.rfind('.');
        let valid = at_pos
            .is_some_and(|at| at > 0 && dot_pos.is_some_and(|dot| dot > at && dot < e.len() - 1));
        if !valid {
            return Ok(HttpResponse::BadRequest().json(AuthError {
                error: "Invalid email format".to_string(),
            }));
        }
    }

    // username 唯一性（唯一身份基点；username 唯一索引保证幂等锚点）。
    // add-register-approval-closure：被驳回/禁用用户（status IN disabled/rejected）
    // 重新注册 → 复用原用户行（重置密码 + 重建审批），不再返回 409；
    // 其余冲突保持 409。
    let existing_username = sqlx::query_as::<_, (i64, String)>(
        r#"SELECT id, status::text FROM isahl_auth.auth_users WHERE username = $1"#,
    )
    .bind(&username)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(|e| {
        log::error!("Database error checking username: {}", e);
        actix_web::error::ErrorInternalServerError("Database error")
    })?;
    // 可复用标志：旧用户存在且处于禁用/驳回态
    let reuse_user_id: Option<i64> = match &existing_username {
        Some((id, status)) if status == "disabled" || status == "rejected" => Some(*id),
        Some(_) => None,
        None => None,
    };
    if existing_username.is_some() && reuse_user_id.is_none() {
        return Ok(HttpResponse::Conflict().json(AuthError {
            error: "Username already exists".to_string(),
        }));
    }

    // Validate password strength via centralized policy (SECURITY_SPEC §5 基线)
    if let Err(e) = password::validate_password_policy(password) {
        return Ok(HttpResponse::BadRequest().json(AuthError {
            error: e.to_string(),
        }));
    }

    // Hash password (offload CPU-intensive Argon2 to blocking pool)
    let password_hash = hash_password_async(password.to_string())
        .await
        .map_err(|e| {
            log::error!("Failed to hash password: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to process password")
        })?;

    // Create user + 访问授权审批（同一事务，add-register-auto-approval）：
    // 注册成功即自动触发审批（even-approve 事件 + oper-approve 实例，fk_operator=首个 admin），
    // 状态置 pending_approval——登录门禁拒绝，PDP 无 UA 拒绝所有资源；审批通过/驳回见
    // Gateway approvals.rs 激活/禁用（经 even-approve.comments.applicant_id 关联）。
    let mut tx = pool.begin().await.map_err(|e| {
        log::error!("Database error starting registration tx: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to create user")
    })?;

    // 新用户 INSERT 或复用用户重置（add-register-approval-closure）：
    // 复用 = 被驳回/禁用用户重新注册——重置密码/状态为 pending_approval（邮箱经
    // auth_user_emails 重写，见末尾 upsert），旧审批实例保留审计；
    // 其余字段（entity 绑定等）不动。
    // 新用户 id 不再由进程内 snowflake 显式生成（G5，fix-sso-auth-gaps）——
    // 由表默认 isahl.gen_next_zuid() 生成（ensure 自愈保证 default 在位），
    // 消除多实例同毫秒碰撞与时钟回拨风险。
    let user_id: i64 = if let Some(old_id) = reuse_user_id {
        sqlx::query_as::<_, (i64,)>(
            r#"
            UPDATE isahl_auth.auth_users
            SET password_hash = $1, status = 'pending_approval',
                updated_at = NOW()
            WHERE id = $2
            RETURNING id
            "#,
        )
        .bind(&password_hash)
        .bind(old_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            log::error!("Database error resetting reused user: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create user")
        })?
        .0
    } else {
        sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO isahl_auth.auth_users (username, password_hash, status, created_at, updated_at, user_type)
            VALUES ($1, $2, 'pending_approval', NOW(), NOW(), $3::isahl_auth.auth_user_type)
            RETURNING id
            "#,
        )
        .bind(&username)
        .bind(&password_hash)
        .bind(channel.kind)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            log::error!("Database error creating user: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create user")
        })?
    };

    // user UA 注册默认指派（approval-endpoint-gates::user-ua-on-registration）——唯一实现
    // = common::registration_approval::ensure_user_ua（对齐 SSO 005 seed「普通用户 (注册默认)」
    // 语义：注册即有基础身份，零业务资源；PEP 对 approvals:0 create 的自助申请放行依赖
    // user UA 关联）。best-effort：失败不阻断注册（原语义）。
    let _ = common::registration_approval::ensure_user_ua(&mut tx, user_id).await;

    // 受理序列（审批事件 + register-context 上下文操作行 + 归属桥 + 模板桥 +
    // oper-approve 实例）单一实现 = `common::registration_approval::intake`
    // （wire-registration-approval-closure D2）：事件落 `channel.spec.leaf_table`、
    // 实例 fk_operator = 首个 admin / fk_subject = 申请人、坐标与桥链形状逐字保持；
    // 本端点只保留身份侧私有写入（auth_users upsert/复用、user UA、会话与邮箱）。
    let instance_id =
        common::registration_approval::intake(&mut tx, channel.spec, user_id, &username)
            .await
            .map_err(|e| {
                log::error!("Database error creating approval intake: {}", e);
                actix_web::error::ErrorInternalServerError("Failed to create approval")
            })?
            .instance_id;

    tx.commit().await.map_err(|e| {
        log::error!("Database error committing registration: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to create user")
    })?;

    // Generate temporary JWT for identity-only access (24h)
    let user_id_str = user_id.to_string();
    let temp_claims = Claims::temp(&user_id_str, &email_str);

    let temp_token =
        jwt::encode_temp_token(&temp_claims, &auth_state.jwt_private_key).map_err(|e| {
            log::error!("Failed to generate temp token: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to generate token")
        })?;

    let refresh_token = jwt::encode_refresh_token(
        &temp_claims,
        &auth_state.jwt_private_key,
        auth_state.jwt_refresh_expiry_secs,
    )
    .map_err(|e| {
        log::error!("Failed to generate refresh token: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to generate token")
    })?;

    // Create session using BIGINT user_id
    let session_manager = SessionManager::new(pool.get_ref().clone());
    let refresh_token_hash = Some(format!("{:x}", md5::compute(refresh_token.as_bytes())));
    // 客户端 IP（可信跳才采信转发头；见 `auth::session::client_ip` 文档）
    let ip_address = crate::auth::session::client_ip(&req);
    let user_agent = req
        .headers()
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let _session = session_manager
        .create_session(CreateSessionRequest {
            user_id,
            idp_provider_id: None,
            idp_session_id: None,
            ip_address,
            user_agent,
            front_channel_logout_uri: None,
            back_channel_logout_uri: None,
            refresh_token_hash,
        })
        .await
        .map_err(|e| {
            log::error!("Failed to create session: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create session")
        })?;

    // 邮箱写入 auth_user_emails（1:N 唯一存储；注册邮箱回主位——重注册换邮箱时
    // 旧主邮箱降级保留，读径按 is_primary DESC, id 取主邮箱）
    if let Some(e) = &email {
        let _ = sqlx::query(
            "UPDATE isahl_auth.auth_user_emails SET is_primary = FALSE, updated_at = NOW() \
             WHERE fk_user = $1 AND deleted_at IS NULL AND is_primary AND email <> $2",
        )
        .bind(user_id)
        .bind(e)
        .execute(pool.get_ref())
        .await;
        let _ = sqlx::query(
            "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, verified, created_at, updated_at) \
             VALUES ($1, $2, TRUE, FALSE, NOW(), NOW()) \
             ON CONFLICT (fk_user, email) WHERE deleted_at IS NULL \
             DO UPDATE SET is_primary = TRUE, updated_at = NOW()",
        )
        .bind(user_id)
        .bind(e)
        .execute(pool.get_ref())
        .await;
    }

    // Set refresh token cookie
    let response = HttpResponse::Created().json(RegisterResponse {
        user_id: user_id_str,
        email: email_str,
        status: "pending_approval".to_string(),
        channel: channel.kind.to_string(),
        approval_instance_id: Some(instance_id.to_string()),
    });

    let response = set_refresh_cookie(
        response,
        &refresh_token,
        auth_state.jwt_refresh_expiry_secs,
        None,
    );
    let response = jwt::set_access_cookie(
        response,
        &temp_token,
        // temp token 固定 24h（identity 流程），cookie 跟随 token 实际寿命
        24 * 3600,
        None,
    );
    // 注册即建立临时会话（identity 流程）：与令牌同批签发 CSRF
    Ok(jwt::attach_csrf_cookie(
        response,
        common::csrf::cookie_scope(&req).as_deref(),
    ))
}
