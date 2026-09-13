//! cron 表达式解析（标准 5 字段，UTC）
//!
//! 字段顺序：`分 时 日 月 周`。每个字段支持 `*`、`N`、`a-b`、`*/N`、`a-b/N`、`a/N` 与逗号列表；
//! 月字段兼容 `JAN`-`DEC`、周字段兼容 `SUN`-`SAT`（三字母、大小写不敏感，周日 = `0` 或 `7`）。
//!
//! 语义（对齐 Vixie cron）：
//! - 按 UTC 分解时间戳（业务本地时间窗口由各 handler 自行判断，调度器不解析时区）；
//! - 日（DOM）与周（DOW）字段同时受限（原文非 `*`）时命中判定取「或」，仅一侧受限时取该侧，
//!   两侧皆 `*` 时恒真。
//!
//! 计划行 cron 来自 `isahl.zc_id_plan.cron`（用户可写，`ALIOTH_ONTOLOGY_SPEC §8.4`）；
//! 解析失败由调度循环告警跳过，不影响其它计划。

use chrono::{DateTime, Datelike, Timelike};

const MONTH_NAMES: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const DOW_NAMES: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// cron 解析错误
#[derive(Debug, thiserror::Error)]
pub enum CronError {
    #[error("cron 表达式为空")]
    Empty,
    #[error("cron 字段数必须为 5（分 时 日 月 周）: {0}")]
    InvalidFieldCount(String),
    #[error("cron {field} 字段无效: {value}")]
    InvalidField { field: &'static str, value: String },
}

/// 解析后的 cron 调度规则（位掩码 = 命中集合，命中判定即位测试）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    /// 分 0-59
    minutes: u64,
    /// 时 0-23
    hours: u32,
    /// 日 1-31
    days_of_month: u32,
    /// 月 1-12
    months: u16,
    /// 周 0-6（0 = 周日）
    days_of_week: u8,
    /// 日字段原文非 `*`（Vixie 语义：与周字段同时受限时取「或」）
    dom_restricted: bool,
    /// 周字段原文非 `*`
    dow_restricted: bool,
}

impl CronSchedule {
    /// 解析 5 字段 cron 表达式（字段见模块文档）
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        let expr = expr.trim();
        if expr.is_empty() {
            return Err(CronError::Empty);
        }
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(CronError::InvalidFieldCount(expr.to_string()));
        }
        Ok(Self {
            minutes: parse_field(fields[0], 0, 59, None, "分")?,
            hours: parse_field(fields[1], 0, 23, None, "时")? as u32,
            days_of_month: parse_field(fields[2], 1, 31, None, "日")? as u32,
            months: parse_field(fields[3], 1, 12, Some((&MONTH_NAMES, 1)), "月")? as u16,
            days_of_week: normalize_dow(parse_field(fields[4], 0, 7, Some((&DOW_NAMES, 0)), "周")?),
            dom_restricted: fields[2] != "*",
            dow_restricted: fields[4] != "*",
        })
    }

    /// 给定时刻（epoch 秒，UTC）是否命中
    pub fn matches(&self, ts_epoch_secs: i64) -> bool {
        let Some(dt) = DateTime::from_timestamp(ts_epoch_secs, 0) else {
            return false;
        };
        if self.minutes & (1u64 << dt.minute()) == 0 {
            return false;
        }
        if self.hours & (1u32 << dt.hour()) == 0 {
            return false;
        }
        if self.months & (1u16 << dt.month()) == 0 {
            return false;
        }
        let dom = self.days_of_month & (1u32 << dt.day()) != 0;
        let dow = (u32::from(self.days_of_week) >> dt.weekday().num_days_from_sunday()) & 1 == 1;
        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => dom || dow,
            (true, false) => dom,
            (false, true) => dow,
            (false, false) => true,
        }
    }
}

/// 解析单个字段为位掩码（`值 = 位号`）
///
/// `names` 为 `(三字母名表, 名字起始值)`；`label` 仅用于错误信息。
fn parse_field(
    field: &str,
    min: u32,
    max: u32,
    names: Option<(&[&str], u32)>,
    label: &'static str,
) -> Result<u64, CronError> {
    let bad = |value: &str| CronError::InvalidField {
        field: label,
        value: value.to_string(),
    };
    if field.is_empty() {
        return Err(bad(field));
    }
    let mut mask: u64 = 0;
    for part in field.split(',') {
        let part = part.trim();
        let (base, step) = match part.split_once('/') {
            Some((base, step)) => {
                let step: u32 = step.trim().parse().map_err(|_| bad(part))?;
                if step == 0 || step > max {
                    return Err(bad(part));
                }
                (base.trim(), step)
            }
            None => (part, 1),
        };
        let (start, end) = if base == "*" {
            (min, max)
        } else if let Some((a, b)) = base.split_once('-') {
            let a = value_of(a, min, max, names).ok_or_else(|| bad(part))?;
            let b = value_of(b, min, max, names).ok_or_else(|| bad(part))?;
            (a, b)
        } else {
            let v = value_of(base, min, max, names).ok_or_else(|| bad(part))?;
            // `a/N` = 从 a 起每 N（至字段上界）；`a` = 单值
            (v, if step > 1 { max } else { v })
        };
        if start > end {
            return Err(bad(part));
        }
        let mut v = start;
        while v <= end {
            mask |= 1u64 << v;
            v += step;
        }
    }
    Ok(mask)
}

/// 单值解析：数字或三字母名（大小写不敏感）
fn value_of(token: &str, min: u32, max: u32, names: Option<(&[&str], u32)>) -> Option<u32> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if let Ok(v) = token.parse::<u32>() {
        return (min..=max).contains(&v).then_some(v);
    }
    let (names, base) = names?;
    let upper = token.to_ascii_uppercase();
    names
        .iter()
        .position(|n| upper.starts_with(n))
        .map(|i| base + i as u32)
}

/// 周字段归一化：`7`（周日）折到 `0`
fn normalize_dow(mask: u64) -> u8 {
    ((mask >> 7) & 1 | mask & 0x7f) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2024-01-01T00:00:00Z（周一）
    const MON_2024_01_01: i64 = 1_704_067_200;
    const DAY: i64 = 86_400;

    fn parse(expr: &str) -> CronSchedule {
        CronSchedule::parse(expr).unwrap_or_else(|e| panic!("parse {expr}: {e}"))
    }

    // ── 既有形态回归 ──────────────────────────────────────────────

    #[test]
    fn every_n_minutes() {
        let s = parse("*/5 * * * *");
        assert!(s.matches(0));
        assert!(s.matches(300));
        assert!(!s.matches(60));
        assert!(s.matches(600));
        assert!(parse("*/1 * * * *").matches(1_700_000_001));
    }

    #[test]
    fn fixed_minute() {
        let s = parse("30 * * * *");
        // 00:30 UTC = 1800s；01:30 = 5400s；00:00 不命中
        assert!(s.matches(1800));
        assert!(s.matches(5400));
        assert!(!s.matches(0));
        assert!(!s.matches(60));
        assert!(parse("0 * * * *").matches(3600));
    }

    #[test]
    fn star_means_every_minute() {
        let s = parse("* * * * *");
        assert!(s.matches(0));
        assert!(s.matches(2_000_000_000)); // 2033-05-18，双 `*` 恒真
    }

    // ── 每日计划（本次事故表达式）───────────────────────────────

    #[test]
    fn daily_plan_matches_at_scheduled_hour() {
        let s = parse("0 1 * * *");
        assert!(s.matches(3600), "1970-01-01T01:00Z MUST match");
        assert!(!s.matches(3599), "00:59:59 MUST NOT match");
        assert!(!s.matches(3660), "01:01 MUST NOT match");
        assert!(!s.matches(0), "00:00 MUST NOT match");
    }

    #[test]
    fn daily_plan_matches_across_day_and_month() {
        let s = parse("0 1 * * *");
        assert!(s.matches(3600 + DAY), "翌日 01:00 MUST match");
        assert!(s.matches(3600 + 31 * DAY), "跨月同日 01:00 MUST match");
        assert!(s.matches(3600 + 365 * DAY), "跨年同日 01:00 MUST match");
    }

    #[test]
    fn daily_plan_is_utc_anchored() {
        // 同一时刻不因「本地」解释漂移：2024-01-05T01:00Z
        let ts = MON_2024_01_01 + 4 * DAY + 3600;
        assert!(parse("0 1 * * *").matches(ts));
        assert!(!parse("0 9 * * *").matches(ts));
    }

    // ── 字段语法 ─────────────────────────────────────────────────

    #[test]
    fn minute_step_over_hour_range_and_weekday_range() {
        let s = parse("*/5 8-20 * * 1-5");
        let fri_0805 = MON_2024_01_01 + 4 * DAY + 8 * 3600 + 300; // 2024-01-05（周五）08:05Z
        assert!(s.matches(fri_0805));
        assert!(s.matches(fri_0805 + 12 * 3600 + 55 * 60 - 300)); // 20:55Z
        assert!(!s.matches(fri_0805 - 3600), "07:05 不在时字段范围");
        assert!(!s.matches(fri_0805 - 60), "分钟非 5 的倍数");
        assert!(!s.matches(fri_0805 + 13 * 3600), "21:00 不在时字段范围");
        assert!(!s.matches(fri_0805 + DAY), "周六不在周字段范围");
    }

    #[test]
    fn lists_ranges_steps_and_names() {
        let s = parse("30 9 * JAN,MAR MON");
        assert!(s.matches(MON_2024_01_01 + 7 * DAY + 9 * 3600 + 1800)); // 2024-01-08（周一）
        assert!(
            !s.matches(MON_2024_01_01 + 8 * DAY + 9 * 3600 + 1800),
            "周二不命中"
        );
        assert!(
            !s.matches(MON_2024_01_01 + 7 * DAY + 9 * 3600 + 1740),
            "09:29 不命中"
        );
        assert!(
            !s.matches(MON_2024_01_01 + 7 * DAY + 31 * DAY + 9 * 3600),
            "二月不命中"
        );
        assert!(parse("30 9 * jan mon").matches(MON_2024_01_01 + 7 * DAY + 9 * 3600 + 1800));

        let s = parse("0 9,18 * * *");
        assert!(s.matches(9 * 3600));
        assert!(s.matches(18 * 3600));
        assert!(!s.matches(13 * 3600));

        let s = parse("0-30/10 1 * * *");
        for m in [0, 10, 20, 30] {
            assert!(s.matches(3600 + m * 60), "01:{m:02} MUST match");
        }
        assert!(!s.matches(3600 + 31 * 60));
        assert!(!s.matches(2 * 3600));

        let s = parse("5/15 * * * *");
        for m in [5, 20, 35, 50] {
            assert!(s.matches(m * 60), "第 {m} 分 MUST match");
        }
        assert!(!s.matches(10 * 60));
    }

    #[test]
    fn sunday_accepts_zero_and_seven() {
        let sunday = MON_2024_01_01 + 6 * DAY + 3600; // 2024-01-07（周日）01:00Z
        assert!(parse("0 1 * * 0").matches(sunday));
        assert!(parse("0 1 * * 7").matches(sunday));
        assert!(parse("0 1 * * SUN").matches(sunday));
    }

    // ── 日/周「或」语义 ──────────────────────────────────────────

    #[test]
    fn dom_and_dow_restricted_takes_or() {
        let s = parse("0 0 1 * 1");
        assert!(s.matches(MON_2024_01_01), "1 日且周一");
        assert!(s.matches(MON_2024_01_01 + 7 * DAY), "仅周一（8 日）");
        assert!(s.matches(MON_2024_01_01 + 31 * DAY), "仅 1 日（周四）");
        assert!(!s.matches(MON_2024_01_01 + 36 * DAY), "6 日周二皆不满足");
    }

    #[test]
    fn single_restricted_field_governs() {
        let day_one_only = parse("0 0 1 * *");
        assert!(day_one_only.matches(MON_2024_01_01 + 31 * DAY));
        assert!(!day_one_only.matches(MON_2024_01_01 + 32 * DAY));

        let monday_only = parse("0 0 * * 1");
        assert!(monday_only.matches(MON_2024_01_01 + 7 * DAY));
        assert!(!monday_only.matches(MON_2024_01_01 + 8 * DAY));
    }

    // ── 非法表达式 ───────────────────────────────────────────────

    #[test]
    fn rejects_invalid_expressions() {
        assert!(matches!(CronSchedule::parse("   "), Err(CronError::Empty)));
        for expr in [
            "0 1 * *",       // 字段数不足
            "a b c",         // 字段数不足
            "0 1 * * * *",   // 字段数过多（秒级不在支持范围）
            "*/0 * * * *",   // 步长 0
            "*/60 * * * *",  // 步长超上界
            "61 * * * *",    // 分越界
            "0 24 * * *",    // 时越界
            "0 1 0 * *",     // 日越界（0）
            "0 1 * 13 *",    // 月越界
            "0 1 * * 8",     // 周越界
            "10-5 * * * *",  // 区间倒置
            "*/5 * * FOO *", // 未知名字
            "0 1 * * MON-",  // 缺项
        ] {
            assert!(
                CronSchedule::parse(expr).is_err(),
                "{expr} MUST be rejected"
            );
        }
    }

    #[test]
    fn error_carries_field_name_and_text() {
        match CronSchedule::parse("0 24 * * *") {
            Err(CronError::InvalidField { field, value }) => {
                assert_eq!(field, "时");
                assert_eq!(value, "24");
            }
            other => panic!("expected InvalidField, got {other:?}"),
        }
    }
}
