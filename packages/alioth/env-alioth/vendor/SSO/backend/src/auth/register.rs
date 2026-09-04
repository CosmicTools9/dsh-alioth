//! User registration handler
//!
//! Provides HTTP handlers for user registration

use actix_web::{web, HttpRequest, HttpResponse};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::{
    jwt::{self, set_refresh_cookie, Claims},
    password::{self, hash_password_async},
    session::{CreateSessionRequest, SessionManager},
    AuthState,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// Snowflake ID 生成器
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn generate_snowflake_id() -> i64 {
    // 时间戳部分 (41 bits) - 毫秒级 Unix 时间戳
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    // 机器 ID (10 bits) - 这里使用 0
    let machine_id: u64 = 0;

    // 序列号 (12 bits)
    let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst) & 0xFFF;

    // 组合成 64 bit ID
    // | 1 bit unused | 41 bits timestamp | 10 bits machine | 12 bits sequence |
    let id = ((timestamp & 0x1FFFFFFFFFF) << 22) | (machine_id << 12) | sequence;

    id as i64
}

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

/// 注册通道描述符（add-dual-register-channels）：通道即身份语义——身份类型、
/// 审批事件/实例 code、绑定审批流程、文案语义均由通道决定。
pub struct RegisterChannel {
    /// 身份类型（auth_users.user_type）
    pub kind: &'static str,
    /// 审批事件与实例 code（Gateway 激活链匹配键）
    pub event_code: &'static str,
    /// 绑定审批流程（zc_id_process.code）
    pub flow_code: &'static str,
    /// 事件 notice 文案（{} 为账号占位）
    pub notice_tpl: &'static str,
    /// 事件 comments 文案
    pub comments_tpl: &'static str,
}

/// 内部通道（Gateway 注册页）：访问授权审批，FLOW-AUTHORIZATION（现状语义）
pub const INTERNAL_CHANNEL: RegisterChannel = RegisterChannel {
    kind: "standard",
    event_code: "user-register-approval",
    flow_code: "FLOW-AUTHORIZATION",
    notice_tpl: "用户 {} 访问授权审批",
    comments_tpl: "访问授权审批：申请人 {}",
};

/// 外部主体通道（OpenActivity 门户）：外部主体入驻审批，独立流程
pub const EXTERNAL_CHANNEL: RegisterChannel = RegisterChannel {
    kind: "external",
    event_code: "external-subject-register-approval",
    flow_code: "FLOW-EXTERNAL-SUBJECT",
    notice_tpl: "外部主体 {} 入驻审批",
    comments_tpl: "外部主体入驻审批：申请人 {}",
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

    // email 若提供：格式 + 全局唯一性校验（可选通道，非唯一基点）
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
        let existing_email: i64 = sqlx::query_scalar(
            "SELECT COALESCE(\
                (SELECT 1 FROM isahl_auth.auth_users WHERE email = $1), \
                (SELECT 1 FROM isahl_auth.auth_user_emails WHERE email = $1 AND deleted_at IS NULL), \
                0)::bigint",
        )
        .bind(e)
        .fetch_one(pool.get_ref())
        .await
        .map_err(|err| {
            log::error!("Database error checking email: {}", err);
            actix_web::error::ErrorInternalServerError("Database error")
        })?;
        if existing_email != 0 {
            return Ok(HttpResponse::Conflict().json(AuthError {
                error: "User with this email already exists".to_string(),
            }));
        }
    }

    // username 唯一性（INSERT 时 name=username，name/username 均 UNIQUE）。
    // add-register-approval-closure：被驳回/禁用用户（status IN disabled/rejected）
    // 重新注册 → 复用原用户行（重置密码 + 重建审批），不再返回 409；
    // 其余冲突保持 409。
    let existing_username = sqlx::query_as::<_, (i64, String)>(
        r#"SELECT id, status FROM isahl_auth.auth_users WHERE username = $1 OR name = $1"#,
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
    let now = Utc::now();

    // Generate snowflake ID（新用户）；复用场景沿用旧 id
    let user_id = match reuse_user_id {
        Some(id) => id,
        None => generate_snowflake_id(),
    };

    let mut tx = pool.begin().await.map_err(|e| {
        log::error!("Database error starting registration tx: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to create user")
    })?;

    // 新用户 INSERT 或复用用户重置（add-register-approval-closure）：
    // 复用 = 被驳回/禁用用户重新注册——重置密码/邮箱/状态为 pending_approval，
    // 旧审批实例保留审计；其余字段（entity 绑定等）不动。
    let result = if let Some(old_id) = reuse_user_id {
        sqlx::query_as::<_, (i64,)>(
            r#"
            UPDATE isahl_auth.auth_users
            SET password_hash = $1, email = $2, status = 'pending_approval',
                updated_at = NOW()
            WHERE id = $3
            RETURNING id
            "#,
        )
        .bind(&password_hash)
        .bind(email.as_deref())
        .bind(old_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            log::error!("Database error resetting reused user: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create user")
        })?
    } else {
        sqlx::query_as::<_, (i64,)>(
            r#"
            INSERT INTO isahl_auth.auth_users (id, name, username, email, password_hash, status, created_at, updated_at, user_type)
            VALUES ($1, $2, $2, $3, $4, 'pending_approval', $5, $6, $7)
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(&username)
        .bind(email.as_deref())
        .bind(&password_hash)
        .bind(now)
        .bind(now)
        .bind(channel.kind)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            log::error!("Database error creating user: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create user")
        })?
    };

    // 审批事件（zc_id_even-approve/authorization 叶表）：comments 仅人类可读文本摘要
    // （comments-text-semantics 规约：写侧 MUST NOT JSON）；申请人归属经
    // oper-approve.fk_subject 模型列承载（fix-register-approval-activation-chain D1）。
    // 「实现·实例」绑 FLOW-AUTHORIZATION 模板（与 approvals/apply 一致，fix-approval-event-adaptive-write）——
    // fk_process=流程 id、tpl_id=approve 节点事件模板 id。`_f_/_t_` 类列仅对带
    // dk_function 的行由 lifecycle 触发器派生（ALIOTH_ONTOLOGY_SPEC §4.3）——
    // 审批事件/操作行无 dk_function：操作行由 seed/组件显式赋值（'实现'/'范例'），
    // 载体行无消费方（非业务数据）保持 NULL，两者均不触发派生。
    // 模板缺失降级 NULL 不阻断注册（Gateway seed 组件启动自愈 + 一致性回填）。
    let approval_notice = channel.notice_tpl.replace("{}", &username);
    let approval_comments = channel.comments_tpl.replace("{}", &username);
    // user UA 注册默认指派（fix-approval-endpoint-gates）：对齐 SSO 005 seed
    // 「普通用户 (注册默认)」语义——注册即有基础身份（零业务资源；PEP 对
    // approvals:0 create 的自助申请放行依赖 user UA 关联）。
    let user_ua_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl_auth.ngac_user_attribute
           WHERE o_name = 'user' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(&mut *tx)
    .await
    .ok()
    .flatten();
    if let Some(ua_id) = user_ua_id {
        let _ = sqlx::query(
            r#"INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, o_name)
               VALUES ($1, $2, 'user')
               ON CONFLICT (fk_user, fk_user_attribute)
               DO UPDATE SET deleted_at = NULL, updated_at = NOW()"#,
        )
        .bind(user_id)
        .bind(ua_id)
        .execute(&mut *tx)
        .await;
    }

    let flow_binding: Option<(i64, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT p.id,
               (SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
                WHERE rro.ref_left = p.id AND rro.code = 'approve'
                  AND rro.deleted_at IS NULL LIMIT 1)
        FROM isahl.zc_id_process p
        WHERE p.code = $1 AND p.deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(channel.flow_code)
    .fetch_optional(&mut *tx)
    .await
    .ok()
    .flatten();
    // SLA 时长绑定（add-register-approval-closure 缺口 2）：72h 时长维度行，
    // 缺省 NULL（seed 组件启动预置 + 一致性回填）；超时自动驳回由 SLA 监控处理。
    let sla_duration_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_scal-duration"
           WHERE o_number = '72h' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(&mut *tx)
    .await
    .ok()
    .flatten();
    // 审批事件（绑 FLOW-AUTHORIZATION）。
    // 写入目标表自适应（fix-approval-event-adaptive-write）：`zc_id_appr-authorization`
    // 叶表并非所有 namespace 存在——存在写叶表（继承 even-approve），否则写
    // even-approve 主表，保证无叶表的 namespace 注册主链路不因 SQL 报错回滚。
    // 叶表存在性检测：失败（schema/权限错误）不静默当无叶表——写主表 even-approve
    // （总是存在，注册主链路不阻断）并告警（fix-approval-event-adaptive-write 契约）。
    let leaf_table_exists: bool = match sqlx::query_scalar(
        "SELECT to_regclass('isahl.\"zc_id_appr-authorization\"') IS NOT NULL",
    )
    .fetch_one(&mut *tx)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::warn!("register: 检测 authorization 叶表失败（降级写 even-approve 主表）: {e}");
            false
        }
    };
    let event_id: i64 = if leaf_table_exists {
        sqlx::query_scalar(
            r#"
            INSERT INTO isahl."zc_id_appr-authorization" (
                created_by_id, updated_by_id, notice, code, comments,
                tpl_id, qk_sla, created_at, updated_at
            ) VALUES ($1, $1, $2, $3, $4, $5, $6, NOW(), NOW())
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(&approval_notice)
        .bind(channel.event_code)
        .bind(&approval_comments)
        .bind(flow_binding.as_ref().and_then(|(_, tpl)| *tpl))
        .bind(sla_duration_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            log::error!("Database error creating approval event: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create approval")
        })?
    } else {
        sqlx::query_scalar(
            r#"
            INSERT INTO isahl."zc_id_even-approve" (
                created_by_id, updated_by_id, notice, code, comments,
                tpl_id, qk_sla, created_at, updated_at
            ) VALUES ($1, $1, $2, $3, $4, $5, $6, NOW(), NOW())
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(&approval_notice)
        .bind(channel.event_code)
        .bind(&approval_comments)
        .bind(flow_binding.as_ref().and_then(|(_, tpl)| *tpl))
        .bind(sla_duration_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            log::error!("Database error creating approval event: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create approval")
        })?
    };

    // fk_process 列已物理移除（2026-08-30 remove-event-fk-process-pollution D3 写侧）：
    // 事件↔FLOW-AUTHORIZATION 归属经桥链——'register-context' 上下文 oper 行
    // （每流程复用一行）+ process_rr_operation 归属桥 + rr_event 模板桥
    if let Some((flow_id, _)) = flow_binding {
        let ctx_oper: Option<i64> = sqlx::query_scalar(
            r#"SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
               JOIN isahl."zc_id_oper-approve" oa ON oa.id = rro.ref_right
                 AND oa.deleted_at IS NULL AND oa.notice = 'register-context'
               WHERE rro.ref_left = $1 AND rro.deleted_at IS NULL LIMIT 1"#,
        )
        .bind(flow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            log::error!("Database error resolving register context: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create approval")
        })?;
        let ctx_oper: i64 = match ctx_oper {
            Some(v) => v,
            None => {
                let new_id: i64 = sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_oper-approve" (notice, created_by_id)
                       VALUES ('register-context', $1) RETURNING id"#,
                )
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| {
                    log::error!("Database error creating register context: {}", e);
                    actix_web::error::ErrorInternalServerError("Failed to create approval")
                })?;
                sqlx::query(
                    "INSERT INTO isahl.zc_id_process_rr_operation (id, ref_left, ref_right, created_by_id)
                     VALUES (isahl.gen_next_zuid(), $1, $2, $3)",
                )
                .bind(flow_id)
                .bind(new_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| {
                    log::error!("Database error linking register context: {}", e);
                    actix_web::error::ErrorInternalServerError("Failed to create approval")
                })?;
                new_id
            }
        };
        sqlx::query(
            "INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
             VALUES (isahl.gen_next_zuid(), $1, $2, $3)",
        )
        .bind(ctx_oper)
        .bind(event_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            log::error!("Database error bridging approval event: {}", e);
            actix_web::error::ErrorInternalServerError("Failed to create approval")
        })?;
    }

    // 审批实例（zc_id_oper-approve）：fk_operator = 首个 admin（审批工作区按 operator 可见）；
    // 无 admin（bootstrap 阶段）→ NULL，由管理面后补。
    let admin_id: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT ur.fk_user FROM isahl_auth.ngac_user_rr_attribute ur
        JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
        WHERE ua.o_name = 'admin' AND ur.deleted_at IS NULL AND ua.deleted_at IS NULL
          AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
        ORDER BY ur.id LIMIT 1
        "#,
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        log::error!("Database error resolving approver: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to create approval")
    })?
    .flatten();

    let instance_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl."zc_id_oper-approve" (
            notice, code, fk_subject, fk_operator, created_by_id, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $3, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(&approval_notice)
    .bind(channel.event_code)
    .bind(user_id)
    .bind(admin_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        log::error!("Database error creating approval instance: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to create approval")
    })?;

    // 实例↔事件关联（fk_approve 列已物理移除）→ operation_rr_event 桥
    sqlx::query(
        "INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
         VALUES (isahl.gen_next_zuid(), $1, $2, $3)",
    )
    .bind(instance_id)
    .bind(event_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        log::error!("Database error linking approval instance: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to create approval")
    })?;

    tx.commit().await.map_err(|e| {
        log::error!("Database error committing registration: {}", e);
        actix_web::error::ErrorInternalServerError("Failed to create user")
    })?;

    // Generate temporary JWT for identity-only access (24h)
    let user_id_str = result.0.to_string();
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
    let ip_address = req
        .connection_info()
        .realip_remote_addr()
        .map(|s| s.to_string());
    let user_agent = req
        .headers()
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let _session = session_manager
        .create_session(CreateSessionRequest {
            user_id: result.0,
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

    // 邮箱写入 auth_user_emails（1:N；首个为主邮箱并镜像 auth_users.email）
    if let Some(e) = &email {
        let _ = sqlx::query(
            "INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary, verified, created_at, updated_at) \
             VALUES ($1, $2, TRUE, FALSE, NOW(), NOW()) ON CONFLICT DO NOTHING",
        )
        .bind(result.0)
        .bind(e)
        .execute(pool.get_ref())
        .await;
    }

    // Clean up used email verification record（若有；未验证门禁已移除，此处仅清理残留）
    if let Some(e) = &email {
        let _ = sqlx::query(
            "DELETE FROM isahl_auth.auth_email_verifications WHERE email = $1 AND purpose = 'register'",
        )
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

    let response = set_refresh_cookie(response, &refresh_token, auth_state.jwt_refresh_expiry_secs);
    Ok(jwt::set_access_cookie(
        response,
        &temp_token,
        // temp token 固定 24h（identity 流程），cookie 跟随 token 实际寿命
        24 * 3600,
    ))
}
