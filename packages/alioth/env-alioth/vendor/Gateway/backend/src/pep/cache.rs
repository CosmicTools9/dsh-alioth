//! NGAC 列级授权缓存 + 策略版本探针。
//!
//! 决策缓存已移除（remove-ngac-pep-decision-cache）：权限立即生效为硬性要求，
//! 每个 PEP 决策现调 SSO PDP（认知派生等无版本信号输入不存在陈旧面）。
//! 本模块仅保留列级授权缓存（ColumnCache，association 派生、版本探针失效）
//! 与版本探针（VersionProbe）。

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// 策略版本探针周期（2s）——版本化变更（策略边/权限定义/指派/委托）跨 worker
///  staleness 上界；非版本化来源（认知派生：岗位/任职，isahl 冻结不可加触发器）
/// 仍由条目 TTL 兜底。
pub const DEFAULT_PROBE_TTL: Duration = Duration::from_secs(2);

/// 策略版本探针（fix-ngac-decision-consistency D4）——per-worker 决策/列缓存的失效信号源。
///
/// 每周期单航班查询 SSO `GET /api/ngac/policy-version`；版本变化即清空本 worker
/// 决策缓存与列缓存。探针失败返回 `Unavailable`——调用方 MUST 绕过缓存直调 PDP
///（不服务陈旧条目、不毒化缓存；PDP 失败仍 fail-closed 403）。
/// standalone / NGAC_FAIL_OPEN（无 PDP 客户端）不探针。
pub struct VersionProbe {
    state: RwLock<ProbeState>,
    probe_ttl: Duration,
    /// 单航班锁：并发请求只允许一个实际发起探针，其余按「新鲜」处理
    /// （staleness 上界仍为一个周期）。
    flight: tokio::sync::Mutex<()>,
}

struct ProbeState {
    last_version: Option<i64>,
    probed_at: Instant,
}

/// 探针结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// 版本已确认新鲜（周期内或刚探针）。
    Fresh,
    /// 探针失败——调用方绕过缓存。
    Unavailable,
}

impl VersionProbe {
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_PROBE_TTL)
    }

    pub fn with_ttl(probe_ttl: Duration) -> Self {
        Self {
            state: RwLock::new(ProbeState {
                last_version: None,
                // 首次请求即探针（probed_at 回退一个周期）
                probed_at: Instant::now()
                    .checked_sub(probe_ttl + Duration::from_millis(1))
                    .unwrap_or_else(Instant::now),
            }),
            probe_ttl,
            flight: tokio::sync::Mutex::new(()),
        }
    }

    fn is_fresh(&self) -> bool {
        self.state
            .read()
            .map(|s| s.probed_at.elapsed() < self.probe_ttl)
            .unwrap_or(false)
    }

    /// 应用探针版本：变化 → 失效列缓存并记录；返回是否发生变化。
    /// 首次记录（None → Some）不失效（缓存可能为空，无需清）。
    fn apply_version(&self, version: i64, column_cache: &ColumnCache) -> bool {
        let changed = match self.state.write() {
            Ok(mut st) => {
                let changed = matches!(st.last_version, Some(v) if v != version);
                st.last_version = Some(version);
                st.probed_at = Instant::now();
                changed
            }
            Err(_) => false,
        };
        if changed {
            column_cache.invalidate_all();
            common::telemetry::info!(
                "NGAC policy version changed to {} — worker column cache invalidated",
                version
            );
        }
        changed
    }

    /// 确保版本新鲜：周期内直接 Fresh；过期则单航班探针。
    pub async fn ensure_fresh(
        &self,
        client: &ngac_contract::HttpNgacClient,
        column_cache: &ColumnCache,
    ) -> ProbeOutcome {
        if self.is_fresh() {
            return ProbeOutcome::Fresh;
        }
        let Ok(_guard) = self.flight.try_lock() else {
            // 另一请求正在探针——staleness 上界仍为一个周期
            return ProbeOutcome::Fresh;
        };
        // double-check：等锁期间可能已被其他请求探针
        if self.is_fresh() {
            return ProbeOutcome::Fresh;
        }
        match client.policy_version().await {
            Ok(resp) => {
                self.apply_version(resp.version, column_cache);
                ProbeOutcome::Fresh
            }
            Err(e) => {
                common::telemetry::warn!(
                    "NGAC policy version probe failed: {} — column cache bypassed for this request",
                    e
                );
                ProbeOutcome::Unavailable
            }
        }
    }

    /// 当前已知版本（测试观测用）。
    #[cfg(test)]
    pub fn last_version(&self) -> Option<i64> {
        self.state.read().ok().and_then(|s| s.last_version)
    }
}

impl Default for VersionProbe {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_cache_basic_and_invalidate() {
        let cc = ColumnCache::with_defaults();
        let key = ColumnCache::make_key(42, "engineers", false);
        assert_eq!(cc.get(&key), None);
        cc.set(key.clone(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(cc.get(&key), Some(vec!["a".to_string(), "b".to_string()]));
        cc.invalidate_all();
        assert_eq!(cc.get(&key), None);
    }

    #[test]
    fn version_probe_first_record_no_invalidation() {
        let probe = VersionProbe::with_ttl(Duration::from_secs(2));
        let cc = ColumnCache::with_defaults();
        cc.set("k".to_string(), vec!["a".to_string()]);
        let changed = probe.apply_version(7, &cc);
        assert!(!changed, "首次记录（None→Some）不失效");
        assert_eq!(cc.get("k"), Some(vec!["a".to_string()]));
        assert_eq!(probe.last_version(), Some(7));
    }

    #[test]
    fn version_probe_change_invalidates_column_cache() {
        let probe = VersionProbe::with_ttl(Duration::from_secs(2));
        let cc = ColumnCache::with_defaults();
        probe.apply_version(7, &cc);
        cc.set("k".to_string(), vec!["a".to_string()]);
        let changed = probe.apply_version(8, &cc);
        assert!(changed, "版本变化必须报告");
        assert_eq!(cc.get("k"), None, "列缓存必须清空");
        assert_eq!(probe.last_version(), Some(8));
    }

    #[test]
    fn version_probe_same_version_no_invalidation() {
        let probe = VersionProbe::with_ttl(Duration::from_secs(2));
        let cc = ColumnCache::with_defaults();
        probe.apply_version(7, &cc);
        cc.set("k".to_string(), vec!["a".to_string()]);
        let changed = probe.apply_version(7, &cc);
        assert!(!changed);
        assert_eq!(cc.get("k"), Some(vec!["a".to_string()]));
    }
}

/// 列级授权缓存——user + resource_type → 授权列集合（TTL 60s，对齐 PDP 对象属性缓存）。
pub struct ColumnCache {
    entries: RwLock<HashMap<String, (Vec<String>, std::time::Instant)>>,
    ttl_seconds: u64,
    max_entries: usize,
}

impl ColumnCache {
    pub fn new(ttl_seconds: u64, max_entries: usize) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            ttl_seconds: ttl_seconds.max(1),
            max_entries: max_entries.max(100),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(60, 1024)
    }

    /// 列授权缓存 key：`{主体类}:{user_id}:{resource_type}`。
    /// 主体类前缀（n=自然人 / s=服务用户）隔离两套 id 空间——自然人 sub 解析的 i64
    /// 与服务用户 svc_user_id 同属 auth_users id 数值空间，无前缀会跨主体串线
    /// （服务用户 `["*"]` 缓存可被同 id 自然人命中 → 敏感列越权暴露）。
    pub fn make_key(user_id: i64, resource_type: &str, is_service_token: bool) -> String {
        format!(
            "{}:{}:{}",
            if is_service_token { "s" } else { "n" },
            user_id,
            resource_type
        )
    }

    pub fn get(&self, key: &str) -> Option<Vec<String>> {
        let entries = self.entries.read().ok()?;
        entries.get(key).and_then(|(cols, at)| {
            if at.elapsed().as_secs() > self.ttl_seconds {
                None
            } else {
                Some(cols.clone())
            }
        })
    }

    pub fn set(&self, key: String, cols: Vec<String>) {
        let mut entries = match self.entries.write() {
            Ok(e) => e,
            Err(e) => {
                common::telemetry::warn!("Failed to acquire write lock on column cache: {}", e);
                return;
            }
        };
        if entries.len() >= self.max_entries {
            if let Some(oldest_key) = entries
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&oldest_key);
            }
        }
        entries.insert(key, (cols, std::time::Instant::now()));
    }

    /// 清空全部条目（版本探针失效路径，fix-ngac-decision-consistency D4）。
    pub fn invalidate_all(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.clear();
        }
    }
}
