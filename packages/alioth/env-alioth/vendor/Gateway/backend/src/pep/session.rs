//! PEP 内部 seam — SSO 会话态校验（评审候选 5 分缝之一：session）。
//!
//! 自 `pep/middleware.rs` 内部抽出；接受 SSO 会话令牌（`sid`）与 DB 池，
//! 判定会话当前是否可用（fail-closed）。

use sqlx::PgPool;

/// 校验 SSO 会话是否仍处于活跃状态（**含账号态复查**，E2 / 评审候选 2 的 PEP 半）。
///
/// 仅当 JWT 携带 `sid`（SSO 会话令牌）时执行；旧版未绑定会话的 token
/// 跳过检查（向后兼容）。会话状态从 `isahl_auth.sso_sessions` 读取，
/// 已吊销/过期/不存在的会话视为失效，返回 false。
/// `expires_at` 为 not null 列：本地直接校验过期，不依赖 SSO 定时任务
/// 翻转 status（M69 审计 P3：过期窗口内旧 token 仍被接受）。
/// **账号态 join（2026-10-10）**：并查 `auth_users.is_active`——账号被禁用后，
/// 其仍处有效期的会话不得再通过 PEP（否则「禁用 ⇒ 停供」在会话 TTL 内失效；
/// 与 `DELEGATED_CTE` 的授权方 `is_active` 门同族，fail-closed：无行 ⇒ false）。
pub(super) async fn is_session_active(pool: &PgPool, sid: &str) -> bool {
    let result = sqlx::query_as::<_, (String,)>(
        "SELECT s.status FROM isahl_auth.sso_sessions s \
         JOIN isahl_auth.auth_users u ON u.id = s.user_id \
         WHERE s.session_token = $1 AND s.expires_at > NOW() AND u.is_active = TRUE LIMIT 1",
    )
    .bind(sid)
    .fetch_optional(pool)
    .await;

    match result {
        Ok(Some((status,))) => status == "active",
        // 会话不存在 / 账号禁用 / 查询失败：保守地视为失效，拒绝请求。
        Ok(None) | Err(_) => false,
    }
}
