use async_trait::async_trait;
use llm::LlmService;
use sqlx::PgPool;

use crate::api::chat_sessions::ports::LlmConfigPort;

// LLM 装配单实现（consolidate-ai-chat-chain 4.0）：canonical = `approval::llm_assist`
// （未配置哨兵 + `resolve_roles` 角色映射 + DB/env 装配）。本文件只保留既有调用路径的
// 再导出——`chat_sessions/mod.rs`、系统配置「测试连接」端点
// （`system_config_llm_test.rs`）与本模块测试均经 `db_llm_config::*` 消费；
// MUST NOT 在本文件复制装配/解析逻辑。
pub(crate) use approval::llm_assist::{build_llm_service, is_unconfigured, visible_message};

pub struct DbLlmConfigAdapter {
    pool: PgPool,
}

impl DbLlmConfigAdapter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LlmConfigPort for DbLlmConfigAdapter {
    /// 装配委托单实现（`approval::llm_assist::load_llm_service`：DB `zc_id_prot-llm_config`
    /// enabled 行优先 + enc_fields.api_key 解密，env 兜底）；未配置错误串带哨兵。
    async fn load_service(&self) -> Result<LlmService, String> {
        approval::llm_assist::load_llm_service(&self.pool).await
    }

    /// D2.11 历史窗口：settings->>'max_history_messages'（正整数校验），
    /// 缺省 50；无生效行/非法值 → 50。取行经 `common::config_resolution`（唯一解析）。
    async fn max_history_messages(&self) -> i64 {
        let config = common::config_resolution::resolve_active(
            &self.pool,
            common::config_resolution::ConfigFamily::Llm,
            &system_config::crypto::decrypt_prefixed,
        )
        .await
        .ok()
        .flatten();
        config
            .and_then(|c| {
                c.settings
                    .get("max_history_messages")
                    .and_then(positive_int)
            })
            .unwrap_or(50)
    }
}

/// `max_history_messages` 提取：正整数（JSON 数字或十进制字符串），其余 → None。
fn positive_int(v: &serde_json::Value) -> Option<i64> {
    match v {
        serde_json::Value::String(s) => s.parse::<i64>().ok().filter(|n| *n > 0),
        serde_json::Value::Number(n) => n.as_i64().filter(|n| *n > 0),
        _ => None,
    }
}
