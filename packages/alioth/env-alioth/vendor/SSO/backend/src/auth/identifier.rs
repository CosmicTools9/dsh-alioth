//! 认证标识候选解析（allow-duplicate-email-accounts 单一实现）
//!
//! 背景：email 不再是唯一身份基点（同一 email MAY 属于多个账号），故
//! 「标识 → 账号」从「至多一行」变为「候选集合」。
//! 全仓唯一解析实现集中于此——登录 / zchat / 密码重置 / OAuth 绑定 / SCIM
//! MUST 经本模块取候选，MUST NOT 各自再写一份「按 email 找账号」的 SQL（REUSE_FIRST）。
//!
//! 唯一身份基点仍是 `username`（唯一约束），其候选集合大小恒 ≤ 1。
//! email 的唯一存储 = `isahl_auth.auth_user_emails`（`auth_users` 无 email 冗余列，
//! 见 `openspec/specs/auth-users-schema`）。

use sqlx::PgPool;

/// 候选解析使用的标识列（闭集；查询文本编译期固化，零运行期拼装）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierColumn {
    /// 登录/重置标识推断出的 email（唯一存储 = `auth_user_emails`，
    /// 取 `deleted_at IS NULL` 的全部行——账号级候选，非主邮箱投影）
    Email,
    /// 账号（唯一身份基点，大小恒 ≤ 1）
    Username,
}

impl IdentifierColumn {
    /// 按标识文本自动判别（与既有登录判别口径一致：含 `@` → email；其余 → username）。
    ///
    /// 手机号不在此闭集：`auth_users` 从无 phone 列（手机号载体 = 联系方式链，
    /// 另有 `/auth/phone/*` 验证码链路），故数值型标识按 username 解析。
    pub fn detect(identifier: &str) -> Self {
        if identifier.contains('@') {
            IdentifierColumn::Email
        } else {
            IdentifierColumn::Username
        }
    }

    fn sql(self) -> &'static str {
        match self {
            IdentifierColumn::Email => {
                "SELECT id FROM isahl_auth.auth_users \
                 WHERE id IN (SELECT fk_user FROM isahl_auth.auth_user_emails \
                              WHERE email = $1 AND deleted_at IS NULL) \
                 ORDER BY id"
            }
            IdentifierColumn::Username => {
                "SELECT id FROM isahl_auth.auth_users WHERE username = $1 ORDER BY id"
            }
        }
    }
}

/// 解析标识命中的**全部**候选账号 id（按 id 升序，确定性顺序）。
///
/// 返回空集 = 无匹配（调用方自行决定 401/其它）；返回多元素 = 标识歧义，
/// 调用方 MUST 按各面语义处置（交互面择账号挑战 / 机器面 fail-closed /
/// 重置逐账号投递 / OAuth 不自动绑定 / SCIM 返错）。
pub async fn resolve_candidate_ids(
    pool: &PgPool,
    column: IdentifierColumn,
    value: &str,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(column.sql())
        .bind(value)
        .fetch_all(pool)
        .await
}
