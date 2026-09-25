//! 认证标识候选解析（allow-duplicate-email-accounts 单一实现）
//!
//! 背景：email/phone 不再是唯一身份基点（同一 email MAY 属于多个账号，
//! phone 从无唯一约束），故「标识 → 账号」从「至多一行」变为「候选集合」。
//! 全仓唯一解析实现集中于此——登录 / zchat / 密码重置 / OAuth 绑定 / SCIM
//! MUST 经本模块取候选，MUST NOT 各自再写一份 `WHERE email = $1`（REUSE_FIRST）。
//!
//! 唯一身份基点仍是 `username`（唯一约束），其候选集合大小恒 ≤ 1。

use sqlx::PgPool;

/// 候选解析使用的标识列（闭集；查询文本编译期固化，零运行期拼装）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierColumn {
    /// 登录/重置标识推断出的 email（经 `auth_user_emails` ∪ `auth_users.email`）
    Email,
    /// 手机号（`auth_users.phone`；该列无唯一约束，天然可多匹配）
    Phone,
    /// 账号（唯一身份基点，大小恒 ≤ 1）
    Username,
}

impl IdentifierColumn {
    /// 按标识文本自动判别（与既有登录判别口径一致：含 `@` → email；
    /// 以 `+` 开头或纯数字/分隔符且长度 ≥ 8 → phone；其余 → username）。
    pub fn detect(identifier: &str) -> Self {
        if identifier.contains('@') {
            IdentifierColumn::Email
        } else if identifier.starts_with('+')
            || (identifier.len() >= 8
                && identifier
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '-' || c == ' ' || c == '(' || c == ')'))
        {
            IdentifierColumn::Phone
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
                    OR email = $1 \
                 ORDER BY id"
            }
            IdentifierColumn::Phone => {
                "SELECT id FROM isahl_auth.auth_users WHERE phone = $1 ORDER BY id"
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
