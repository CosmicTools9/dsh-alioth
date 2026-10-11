//! 生效配置解析（`isahl.zc_id_prot-*_config` 族）——**唯一实现**。
//!
//! 消费方（LLM 装配、邮件、短信、文件存储、连通性探测）MUST 经本模块取行，
//! MUST NOT 自行拼接取行 SQL 或自行解密凭证（规约：
//! `openspec/specs/system-config` :: `effective-config-resolution`）。
//!
//! 生效语义（单一判据）：
//! - 门：`deleted_at IS NULL` 且 `settings.enabled` 不为 false（缺键通过，兼容陈旧行）；
//! - 决胜：`settings.is_default=true` 优先（`DESC NULLS LAST`——缺键行不压过显式默认行），
//!   再按 `updated_at DESC, id DESC`；
//! - `settings.public` MUST NOT 参与判定（旗标已退役）。
//!
//! 凭证解密：由调用方注入解密器（统一传 `system_config::crypto::decrypt_prefixed`）；
//! 本模块对 `enc_fields` 的**全部字符串值**过解密器（非 `enc:` 值透传，无副作用）。
//! 注入而非内置的原因：`common` 处于依赖最底层，不得反向依赖 `system-config`。

use serde_json::{json, Map, Value};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

/// 配置族 → 物理表（白名单枚举；表名唯一来源，杜绝字符串拼接注入面）。
///
/// 不含 `Env` 族：`zc_id_prot-env_config` 的 `settings.enabled` 是**开关值本身**
/// （非「配置生效」门槛），按 code 直读归 `platform_config.rs`（fail-closed 布尔读取器），
/// 不适用 winner-pick。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigFamily {
    Llm,
    Email,
    Im,
    Webhook,
    Storage,
    Sms,
}

impl ConfigFamily {
    /// 分类 code（业务枚举 llm/email/…）→ 族。`Env`（approval 分类）不在 winner-pick 面 ⇒ `None`。
    pub fn from_category(code: &str) -> Option<ConfigFamily> {
        match code {
            "llm" => Some(ConfigFamily::Llm),
            "email" => Some(ConfigFamily::Email),
            "im" => Some(ConfigFamily::Im),
            "webhook" => Some(ConfigFamily::Webhook),
            "storage" => Some(ConfigFamily::Storage),
            "sms" => Some(ConfigFamily::Sms),
            _ => None,
        }
    }

    /// `isahl` schema 下的物理表名
    pub const fn table(self) -> &'static str {
        match self {
            ConfigFamily::Llm => "zc_id_prot-llm_config",
            ConfigFamily::Email => "zc_id_prot-email_config",
            ConfigFamily::Im => "zc_id_prot-im_config",
            ConfigFamily::Webhook => "zc_id_prot-webhook_config",
            ConfigFamily::Storage => "zc_id_prot-oss_config",
            ConfigFamily::Sms => "zc_id_prot-sms_config",
        }
    }
}

/// 解析出的生效配置（凭证已解密）。
///
/// 凭证解密器（注入面；调用方统一传 `decrypt_prefixed` 语义）以裸
/// `&(dyn Fn(&str) -> String + Send + Sync)` 出现在本模块函数签名——MUST NOT 提取为
/// 类型别名：`type X = dyn Fn…` 的 trait-object 默认界是 `'static`，会拒绝借用 `self` 的闭包。
#[derive(Clone, Debug)]
pub struct ActiveConfig {
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub provider: Option<String>,
    /// `settings` jsonb（缺省归一为 `{}`）
    pub settings: Value,
    /// `enc_fields` jsonb（字符串值已过解密器；缺省归一为 `{}`）
    pub credentials: Value,
}

/// 单族 SQL 三形态（编译期固化；表名 = 白名单字面量 ⇒ 满足 `SqlSafeStr` 静态面，
/// 与 Gateway `system_config_repo` 的 `concat!` 同款——MUST NOT 运行期拼 SQL）。
struct FamilySql {
    /// winner-pick：生效门 + 决胜序 + LIMIT 1
    winner: &'static str,
    /// 全量列举：同门同序（无 LIMIT）
    list: &'static str,
    /// 按 id 直取（**不带** enabled 门）
    by_id: &'static str,
}

macro_rules! family_sql {
    ($table:literal) => {
        FamilySql {
            winner: concat!(
                "SELECT id, notice, code, settings->>'provider' AS provider, settings, enc_fields \
                 FROM isahl.\"", $table, "\" \
                 WHERE deleted_at IS NULL AND (settings->>'enabled')::boolean IS NOT FALSE \
                 ORDER BY (settings->>'is_default')::boolean DESC NULLS LAST, updated_at DESC, id DESC \
                 LIMIT 1"
            ),
            list: concat!(
                "SELECT id, notice, code, settings->>'provider' AS provider, settings, enc_fields \
                 FROM isahl.\"", $table, "\" \
                 WHERE deleted_at IS NULL AND (settings->>'enabled')::boolean IS NOT FALSE \
                 ORDER BY (settings->>'is_default')::boolean DESC NULLS LAST, updated_at DESC, id DESC"
            ),
            by_id: concat!(
                "SELECT id, notice, code, settings->>'provider' AS provider, settings, enc_fields \
                 FROM isahl.\"", $table, "\" WHERE id = $1 AND deleted_at IS NULL"
            ),
        }
    };
}

fn sql_for(family: ConfigFamily) -> FamilySql {
    match family {
        ConfigFamily::Llm => family_sql!("zc_id_prot-llm_config"),
        ConfigFamily::Email => family_sql!("zc_id_prot-email_config"),
        ConfigFamily::Im => family_sql!("zc_id_prot-im_config"),
        ConfigFamily::Webhook => family_sql!("zc_id_prot-webhook_config"),
        ConfigFamily::Storage => family_sql!("zc_id_prot-oss_config"),
        ConfigFamily::Sms => family_sql!("zc_id_prot-sms_config"),
    }
}

/// 解析生效配置（winner-pick；单行族用）。
///
/// `Ok(None)` = 无生效行（调用方按各自语义兜底：env 兜底 / NotFound / 默认值）。
pub async fn resolve_active(
    pool: &PgPool,
    family: ConfigFamily,
    decryptor: &(dyn Fn(&str) -> String + Send + Sync),
) -> Result<Option<ActiveConfig>, sqlx::Error> {
    let row = sqlx::query(sql_for(family).winner)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| row_to_active(&r, decryptor)))
}

/// 列举全部生效配置（多后端族用，如 storage；顺序 = 决胜序：默认行在先）。
pub async fn list_active(
    pool: &PgPool,
    family: ConfigFamily,
    decryptor: &(dyn Fn(&str) -> String + Send + Sync),
) -> Result<Vec<ActiveConfig>, sqlx::Error> {
    let rows = sqlx::query(sql_for(family).list).fetch_all(pool).await?;
    Ok(rows.iter().map(|r| row_to_active(r, decryptor)).collect())
}

/// 按 id 取行（**不带** enabled 门——编辑/连通性探测路径按 id 直取）。
pub async fn load_by_id(
    pool: &PgPool,
    family: ConfigFamily,
    id: i64,
    decryptor: &(dyn Fn(&str) -> String + Send + Sync),
) -> Result<Option<ActiveConfig>, sqlx::Error> {
    let row = sqlx::query(sql_for(family).by_id)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| row_to_active(&r, decryptor)))
}

fn row_to_active(row: &PgRow, decryptor: &(dyn Fn(&str) -> String + Send + Sync)) -> ActiveConfig {
    ActiveConfig {
        id: row.get("id"),
        code: row.get("code"),
        notice: row.get("notice"),
        provider: row.get("provider"),
        settings: normalize_object(row.get::<Option<Value>, _>("settings")),
        credentials: decrypt_credentials(row.get::<Option<Value>, _>("enc_fields"), decryptor),
    }
}

/// settings 归一：NULL / 非对象 → `{}` 语义容器（消费者按 map 读键）。
fn normalize_object(raw: Option<Value>) -> Value {
    match raw {
        Some(v @ Value::Object(_)) => v,
        _ => json!({}),
    }
}

/// enc_fields 字符串值全量过解密器（保留非字符串值原样）。
fn decrypt_credentials(
    raw: Option<Value>,
    decryptor: &(dyn Fn(&str) -> String + Send + Sync),
) -> Value {
    match raw {
        Some(Value::Object(map)) => {
            let decrypted: Map<String, Value> = map
                .into_iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(s) => Value::String(decryptor(&s)),
                        other => other,
                    };
                    (k, v)
                })
                .collect();
            Value::Object(decrypted)
        }
        _ => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn family_table_is_whitelist() {
        assert_eq!(ConfigFamily::Llm.table(), "zc_id_prot-llm_config");
        assert_eq!(ConfigFamily::Storage.table(), "zc_id_prot-oss_config");
        assert_eq!(ConfigFamily::Email.table(), "zc_id_prot-email_config");
    }

    #[test]
    fn winner_order_pins_nulls_last() {
        assert!(sql_for(ConfigFamily::Llm)
            .winner
            .contains("DESC NULLS LAST"));
    }

    #[test]
    fn credentials_decrypt_all_string_fields() {
        let raw = json!({"api_key": "enc:x", "retries": 3, "plain": "p"});
        let out = decrypt_credentials(Some(raw), &identity);
        assert_eq!(out["api_key"], json!("enc:x"));
        assert_eq!(out["retries"], json!(3));
        assert_eq!(out["plain"], json!("p"));
    }

    #[test]
    fn credentials_missing_normalizes_to_empty_object() {
        assert_eq!(decrypt_credentials(None, &identity), json!({}));
        assert_eq!(normalize_object(None), json!({}));
    }
}
