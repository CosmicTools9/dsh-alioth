//! OpenAPI 计费聚合内核（跨面共享单源）。
//!
//! 唯一计价实现：产品行（`zc_id_prod-openapi-{sales,purchase,made,request}`）的 `projection`
//! 前缀匹配用量 route × `qk_price` → `isahl."zc_id_scal-price".mark`（**唯一价源**）。
//!
//! - 金额/单价以**字符串**承载（numeric 精度，MUST NOT f64）；
//! - 未定价（无产品行命中或标量价缺失）⇒ `unit_price: None` 且不计入总额；
//! - 同 route 命中多产品 ⇒ 逐产品分行（MUST NOT 合并，避免重复计价）。
//!
//! 消费方：SSO `admin::billing`（报表，`client_id = None`）与
//! Gateway `openapi::payment`（发起收款，`client_id = Some(..)` 的权威金额）。

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use sqlx::{PgPool, Row};

/// 产品面齐备判定（4 张产品表 + 标量价表）；缺任一 ⇒ 按「无定价产品」处理（MUST NOT 500）。
pub const PRODUCTS_FACE_SQL: &str = r#"
SELECT to_regclass('isahl."zc_id_prod-openapi-sales"') IS NOT NULL
   AND to_regclass('isahl."zc_id_prod-openapi-purchase"') IS NOT NULL
   AND to_regclass('isahl."zc_id_prod-openapi-made"') IS NOT NULL
   AND to_regclass('isahl."zc_id_prod-openapi-request"') IS NOT NULL
   AND to_regclass('isahl."zc_id_scal-price"') IS NOT NULL
"#;

/// 定价产品集（4 表 union；projection 非空的非删除行）——与 L4 投影文法**同源**。
///
/// 以 `macro_rules!` + `concat!` 组装**静态** SQL 字面量：`sqlx::query` MUST 收
/// `&'static str`（动态拼接串被仓库 lint 拒：可能注入面）。
macro_rules! products_cte {
    () => {
        r#"
WITH products AS (
    SELECT notice::text AS name, code::text AS code, btrim(projection) AS projection, qk_price
      FROM isahl."zc_id_prod-openapi-sales"
     WHERE deleted_at IS NULL AND btrim(projection) <> ''
    UNION ALL
    SELECT notice::text, code::text, btrim(projection), qk_price
      FROM isahl."zc_id_prod-openapi-purchase"
     WHERE deleted_at IS NULL AND btrim(projection) <> ''
    UNION ALL
    SELECT notice::text, code::text, btrim(projection), qk_price
      FROM isahl."zc_id_prod-openapi-made"
     WHERE deleted_at IS NULL AND btrim(projection) <> ''
    UNION ALL
    SELECT notice::text, code::text, btrim(projection), qk_price
      FROM isahl."zc_id_prod-openapi-request"
     WHERE deleted_at IS NULL AND btrim(projection) <> ''
)
"#
    };
}

/// 用量 × 订阅 × 调用方 × 产品 × 价标的共享 FROM/WHERE 段（**计价文法单一实现**）。
///
/// `$1` = 窗口下界、`$2` = 窗口上界（`[start, end)`）、`$3` = 可选 client 过滤
/// （`NULL` ⇒ 全量；明细/汇总两处共用，保证「客户端口径」与「全局口径」同一文法）。
macro_rules! usage_join_where {
    () => {
        r#"
  FROM isahl_auth.api_usage u
  JOIN isahl_auth.api_subscriptions s ON s.id = u.fk_subscription
  JOIN isahl_auth.api_clients c ON c.id = s.fk_client
  JOIN products p ON u.route LIKE '/api/service/' || p.projection || '%'
  LEFT JOIN isahl."zc_id_scal-price" sp ON sp.id = p.qk_price
 WHERE u.requested_at >= $1 AND u.requested_at < $2
   AND ($3::text IS NULL OR c.client_id = $3)
"#
    };
}

/// 明细（按 client × 产品行分组；同 route 命中多产品 ⇒ 分行，MUST NOT 合并以免重复计价）。
pub const DETAIL_SQL: &str = concat!(
    products_cte!(),
    r#"
SELECT c.client_id::text AS client_id,
       p.code AS product_code,
       p.name AS product_name,
       p.projection AS projection,
       COUNT(*)::bigint AS calls,
       trim_scale(sp.mark)::text AS unit_price,
       CASE WHEN sp.mark IS NULL THEN NULL
            ELSE trim_scale(sp.mark * COUNT(*))::text END AS amount,
       sp.sk_unit::text AS currency
"#,
    usage_join_where!(),
    r#"
 GROUP BY c.client_id, p.code, p.name, p.projection, sp.mark, sp.sk_unit
 ORDER BY c.client_id, p.code NULLS LAST
"#
);

/// 汇总（定价调用量 / 未定价命中 / 总金额）——与明细同一 FROM/WHERE 的聚合视图。
pub const TOTALS_SQL: &str = concat!(
    products_cte!(),
    r#"
SELECT COALESCE(SUM(t.calls) FILTER (WHERE t.unit_price IS NOT NULL), 0)::bigint AS priced_calls,
       COALESCE(SUM(t.calls) FILTER (WHERE t.unit_price IS NULL), 0)::bigint AS unpriced_hits,
       trim_scale(COALESCE(SUM(t.amount_num), 0))::text AS total_amount,
       MIN(t.currency)::text AS currency
  FROM (
    SELECT COUNT(*)::bigint AS calls,
           sp.mark::numeric AS unit_price,
           (sp.mark * COUNT(*))::numeric AS amount_num,
           sp.sk_unit::text AS currency
"#,
    usage_join_where!(),
    r#"
     GROUP BY p.code, p.name, p.projection, sp.mark, sp.sk_unit
  ) t
"#
);

/// 单调用方的未命中产品行调用量（收款口径：与全局 [`UNMATCHED_SQL`] 同一 `NOT EXISTS` 文法）。
pub const CLIENT_UNMATCHED_SQL: &str = concat!(
    products_cte!(),
    r#"
SELECT COUNT(*)::bigint AS unmatched_calls
  FROM isahl_auth.api_usage u
  JOIN isahl_auth.api_subscriptions s ON s.id = u.fk_subscription
  JOIN isahl_auth.api_clients c ON c.id = s.fk_client
 WHERE u.requested_at >= $1 AND u.requested_at < $2
   AND c.client_id = $3
   AND NOT EXISTS (
        SELECT 1 FROM products p
         WHERE u.route LIKE '/api/service/' || p.projection || '%'
   )
"#
);

/// 未命中任何产品行的调用量（未定价的另一半；无 client 过滤——未命中即无产品归属）。
pub const UNMATCHED_SQL: &str = concat!(
    products_cte!(),
    r#"
SELECT COUNT(*)::bigint AS unmatched_calls
  FROM isahl_auth.api_usage u
 WHERE u.requested_at >= $1 AND u.requested_at < $2
   AND NOT EXISTS (
        SELECT 1 FROM products p
         WHERE u.route LIKE '/api/service/' || p.projection || '%'
   )
"#
);

/// 无产品面时的总调用量（未启用产品管理的 ns ⇒ 全部视为未定价）。
pub const TOTAL_USAGE_SQL: &str = "SELECT COUNT(*)::bigint FROM isahl_auth.api_usage \
     WHERE requested_at >= $1 AND requested_at < $2";

/// 月窗口（`month` 缺省 = 当月）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonthWindow {
    pub month: String,
    pub start: NaiveDate,
    pub next: NaiveDate,
}

impl MonthWindow {
    /// 窗口下界（0 点）。
    pub fn start_dt(&self) -> NaiveDateTime {
        self.start.and_hms_opt(0, 0, 0).expect("0 点恒可构造")
    }

    /// 窗口上界（次月 0 点，开区间）。
    pub fn end_dt(&self) -> NaiveDateTime {
        self.next.and_hms_opt(0, 0, 0).expect("0 点恒可构造")
    }
}

/// `month` 非法（非 `YYYY-MM` 或缺省月不可构造）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidMonth;

impl std::fmt::Display for InvalidMonth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "month must be YYYY-MM")
    }
}

/// 解析 `YYYY-MM` 为窗口（缺省 = `now` 当月）。失败 ⇒ [`InvalidMonth`]。
pub fn month_window(
    month: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<MonthWindow, InvalidMonth> {
    // `Some("")` 属非法输入（MUST NOT 静默回退当月）；仅「未提供」才取当月。
    // `month` 原样透传（MUST NOT 归一化/裁剪）：与迁入前的 admin 面解析逐字等价。
    let month_str = match month {
        Some(m) => m.to_string(),
        None => format!("{:04}-{:02}", now.year(), now.month()),
    };
    let start = NaiveDate::parse_from_str(&format!("{}-01", month_str), "%Y-%m-%d")
        .map_err(|_| InvalidMonth)?;
    let next = if start.month() == 12 {
        NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
    }
    .ok_or(InvalidMonth)?;
    Ok(MonthWindow {
        month: month_str,
        start,
        next,
    })
}

/// 明细行（client × 产品行）。
#[derive(Debug, Clone)]
pub struct MonthLine {
    pub client_id: String,
    pub product_code: Option<String>,
    pub product_name: Option<String>,
    pub projection: Option<String>,
    pub calls: i64,
    pub unit_price: Option<String>,
    pub amount: Option<String>,
    pub currency: Option<String>,
}

/// 汇总（同一 FROM/WHERE 的聚合视图）。
#[derive(Debug, Clone)]
pub struct MonthTotals {
    pub priced_calls: i64,
    /// 命中产品行但标量价缺失的调用量（未定价的另一半见 [`month_unmatched_calls`]）。
    pub unpriced_hits: i64,
    pub total_amount: String,
    /// 币种（单调用方口径下应为单一值；无定价行 ⇒ `None`）。
    pub currency: Option<String>,
}

/// 产品面是否齐备（缺面 ⇒ 消费方按「无定价产品」处理，MUST NOT 500）。
pub async fn products_face_present(pool: &PgPool) -> bool {
    sqlx::query_scalar::<_, bool>(PRODUCTS_FACE_SQL)
        .fetch_one(pool)
        .await
        .unwrap_or(false)
}

/// 明细查询；`client_id = None` ⇒ 全量（报表），`Some(..)` ⇒ 单调用方（收款）。
pub async fn month_lines(
    pool: &PgPool,
    start: NaiveDateTime,
    end: NaiveDateTime,
    client_id: Option<&str>,
) -> Result<Vec<MonthLine>, sqlx::Error> {
    let rows = sqlx::query(DETAIL_SQL)
        .bind(start)
        .bind(end)
        .bind(client_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| MonthLine {
            client_id: row.try_get::<String, _>("client_id").unwrap_or_default(),
            product_code: row.try_get("product_code").unwrap_or(None),
            product_name: row.try_get("product_name").unwrap_or(None),
            projection: row.try_get("projection").unwrap_or(None),
            calls: row.try_get("calls").unwrap_or(0),
            unit_price: row.try_get("unit_price").unwrap_or(None),
            amount: row.try_get("amount").unwrap_or(None),
            currency: row.try_get("currency").unwrap_or(None),
        })
        .collect())
}

/// 汇总查询（同一计价文法；`client_id` 语义同 [`month_lines`]）。
pub async fn month_totals(
    pool: &PgPool,
    start: NaiveDateTime,
    end: NaiveDateTime,
    client_id: Option<&str>,
) -> Result<MonthTotals, sqlx::Error> {
    let row = sqlx::query(TOTALS_SQL)
        .bind(start)
        .bind(end)
        .bind(client_id)
        .fetch_one(pool)
        .await?;
    Ok(MonthTotals {
        priced_calls: row.try_get("priced_calls").unwrap_or(0),
        unpriced_hits: row.try_get("unpriced_hits").unwrap_or(0),
        total_amount: row
            .try_get::<String, _>("total_amount")
            .unwrap_or_else(|_| "0".into()),
        currency: row.try_get("currency").unwrap_or(None),
    })
}

/// 单调用方的未命中产品行调用量（收款口径的未定价另一半）。
pub async fn client_unmatched_calls(
    pool: &PgPool,
    start: NaiveDateTime,
    end: NaiveDateTime,
    client_id: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(CLIENT_UNMATCHED_SQL)
        .bind(start)
        .bind(end)
        .bind(client_id)
        .fetch_one(pool)
        .await
}

/// 未命中任何产品行的调用量。
pub async fn month_unmatched_calls(
    pool: &PgPool,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(UNMATCHED_SQL)
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await
}

/// 窗口内总调用量（无产品面分支）。
pub async fn month_total_usage_calls(
    pool: &PgPool,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(TOTAL_USAGE_SQL)
        .bind(start)
        .bind(end)
        .fetch_one(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn month_window_defaults_to_current_month() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 10, 8, 3, 0, 0).unwrap();
        let w = month_window(None, now).unwrap();
        assert_eq!(w.month, "2026-10");
        assert_eq!(w.start.to_string(), "2026-10-01");
        assert_eq!(w.next.to_string(), "2026-11-01");
    }

    #[test]
    fn month_window_december_rolls_over() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let w = month_window(Some("2026-12"), now).unwrap();
        assert_eq!(w.start.to_string(), "2026-12-01");
        assert_eq!(w.next.to_string(), "2027-01-01");
    }

    #[test]
    fn month_window_rejects_bad_format() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 10, 8, 3, 0, 0).unwrap();
        assert!(month_window(Some("2026/10"), now).is_err());
        assert!(month_window(Some("2026-13"), now).is_err());
    }

    #[test]
    fn sql_fragments_share_one_where_clause() {
        // 计价文法单源自证：明细与汇总 MUST 都含同一 client 过滤形（无第二份文法）。
        assert!(DETAIL_SQL.contains("AND ($3::text IS NULL OR c.client_id = $3)"));
        assert!(TOTALS_SQL.contains("AND ($3::text IS NULL OR c.client_id = $3)"));
        assert!(DETAIL_SQL.contains("zc_id_scal-price"));
        assert!(TOTALS_SQL.contains("zc_id_scal-price"));
    }
}
