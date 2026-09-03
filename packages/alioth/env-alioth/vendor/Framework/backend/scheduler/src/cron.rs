//! cron 表达式解析（简化实现）
//!
//! 覆盖收编任务所需形态：
//! - `*/N * * * *` — 每 N 分钟（N ≥ 1）
//! - `M * * * *`   — 每小时第 M 分钟（0 ≤ M ≤ 59）
//!
//! 与 quality scheduler 的简化实现同级别（calculate_next_run 简化），
//! 后续复杂表达式（时/日/月/周维度）按需扩展，解析器保持独立模块。

use std::fmt;

/// cron 解析错误
#[derive(Debug, thiserror::Error)]
pub enum CronError {
    #[error("cron 表达式为空")]
    Empty,
    #[error("cron 表达式格式无效: {0}（支持 */N * * * * 与 M * * * *）")]
    InvalidFormat(String),
    #[error("cron 分钟字段无效: {0}")]
    InvalidMinute(String),
}

/// 解析后的 cron 调度规则
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronSchedule {
    /// 每 N 分钟（1..=59）
    EveryMinutes(u32),
    /// 每小时第 M 分钟（0..=59）
    FixedMinute(u32),
}

impl fmt::Display for CronSchedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CronSchedule::EveryMinutes(n) => write!(f, "*/{n} * * * *"),
            CronSchedule::FixedMinute(m) => write!(f, "{m} * * * *"),
        }
    }
}

impl CronSchedule {
    /// 解析 cron 表达式（仅分钟维度，覆盖 */N 与固定分钟）
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let expr = expr.trim();
        if expr.is_empty() {
            return Err(CronError::Empty);
        }
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(CronError::InvalidFormat(expr.to_string()));
        }
        let minute = fields[0];
        // 其余字段必须为 *（简化实现仅分钟维度）
        for f in &fields[1..] {
            if *f != "*" {
                return Err(CronError::InvalidFormat(expr.to_string()));
            }
        }
        if let Some(n) = minute.strip_prefix("*/") {
            let n: u32 = n
                .parse()
                .map_err(|_| CronError::InvalidMinute(minute.to_string()))?;
            if !(1..=59).contains(&n) {
                return Err(CronError::InvalidMinute(minute.to_string()));
            }
            Ok(CronSchedule::EveryMinutes(n))
        } else if minute == "*" {
            // 每分钟
            Ok(CronSchedule::EveryMinutes(1))
        } else {
            let m: u32 = minute
                .parse()
                .map_err(|_| CronError::InvalidMinute(minute.to_string()))?;
            if m > 59 {
                return Err(CronError::InvalidMinute(minute.to_string()));
            }
            Ok(CronSchedule::FixedMinute(m))
        }
    }

    /// 给定分钟时间戳（epoch 秒），判断当前分钟是否命中
    pub fn matches(&self, ts_epoch_secs: i64) -> bool {
        let minutes = ts_epoch_secs / 60;
        match self {
            CronSchedule::EveryMinutes(n) => {
                let n = i64::from(*n);
                minutes % n == 0
            }
            CronSchedule::FixedMinute(m) => {
                let m = i64::from(*m);
                // UTC 分钟对齐（调度器统一 UTC；业务窗口由 handler 自行判断本地时间）
                minutes.rem_euclid(60) == m
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_every_n_minutes() {
        assert_eq!(
            CronSchedule::parse("*/5 * * * *").unwrap(),
            CronSchedule::EveryMinutes(5)
        );
        assert_eq!(
            CronSchedule::parse("*/1 * * * *").unwrap(),
            CronSchedule::EveryMinutes(1)
        );
    }

    #[test]
    fn parse_fixed_minute() {
        assert_eq!(
            CronSchedule::parse("30 * * * *").unwrap(),
            CronSchedule::FixedMinute(30)
        );
        assert_eq!(
            CronSchedule::parse("0 * * * *").unwrap(),
            CronSchedule::FixedMinute(0)
        );
    }

    #[test]
    fn parse_star_means_every_minute() {
        assert_eq!(
            CronSchedule::parse("* * * * *").unwrap(),
            CronSchedule::EveryMinutes(1)
        );
    }

    #[test]
    fn parse_rejects_invalid() {
        assert!(CronSchedule::parse("").is_err());
        assert!(CronSchedule::parse("*/0 * * * *").is_err());
        assert!(CronSchedule::parse("*/60 * * * *").is_err());
        assert!(CronSchedule::parse("61 * * * *").is_err());
        assert!(CronSchedule::parse("*/5 * * 1 *").is_err()); // 非 * 的日字段
        assert!(CronSchedule::parse("a b c").is_err()); // 字段数不足
    }

    #[test]
    fn every_five_minutes_matches() {
        let s = CronSchedule::EveryMinutes(5);
        // epoch 秒 0（1970-01-01T00:00:00Z）命中；+5min 命中；+1min 不命中
        assert!(s.matches(0));
        assert!(s.matches(300));
        assert!(!s.matches(60));
        assert!(s.matches(600));
    }

    #[test]
    fn fixed_minute_matches() {
        let s = CronSchedule::FixedMinute(30);
        // 00:30 UTC = 1800s；01:30 UTC = 5400s；00:00 = 0 不命中
        assert!(s.matches(1800));
        assert!(s.matches(5400));
        assert!(!s.matches(0));
        assert!(!s.matches(60));
    }
}
