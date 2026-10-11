//! 计费货量口径链写件（合同 → 标准 → 计算公式）。
//!
//! 动因（openspec change `add-contract-billing-weight-basis`，用户裁决 2026-10-10）：合同创建时把
//! 「计费货量口径」（出库货量 / 入库货量 / 较大值 / 较小值）落为**可执行口径**——表达式一律
//! **Rhai**（平台唯一表达式引擎，`ALIOTH_ONTOLOGY_SPEC` §表达式唯一引擎）。**零 DDL**：只用既有
//! `isahl` 表写数据行。
//!
//! 链路（模型既有元素；权威先例 = `Framework/backend/approval/src/handlers/publish.rs` 的
//! 「loop 公式 + 标准范例/实例 + 双桥」四段写入）：
//!
//! ```text
//! 合同 zc_id_contract ──zc_id_contract_rr_standard(关联-合约↔约定标准)──▶ 标准
//! 标准 = zc_id_stan-operation（实现·范例 / 实现·实例 成对，实例 tpl_id → 范例）
//! 标准 ──zc_id_standard_r_formula(关系-标准→公式)──▶ 计算公式
//! 计算公式 = zc_id_form-billing（**数值公式-计费**，公式-数值族叶表；`expression` = Rhai，
//! `context = {"engine":"rhai","params":["out_qty","in_qty"]}`）——叶表铁律 §8.5：INSERT 必须落叶，
//! `zc_id_form-calculation` 是中间父表不可直写；该叶即平台「计价脚本」承载面（`ALIOTH_ONTOLOGY_SPEC`
//! 表达式表：计价脚本 → `zc_id_form-billing.expression`，参数落 `context.params`）。
//! ```
//!
//! 坐标：标准族 = **(JE, FBA, ↓_AB)** —— 声明值见 approval `DkEntity::JeFbaAb`
//! （「标准/规章模板 `zc_id_standard`」），且 `wz` 库 17 条标准实测同值（code 取自声明面，非硬编码 ZUID）。
//! 公式叶按框架先例**不落 dk 列**；桥行只写 `ref_left/ref_right/created_by_id`（框架同款）。
//!
//! 复用面：**公式**与**标准范例** = 口径级共享实体（按 `code` find-or-create，跨合同复用）；
//! **标准实例** = 合同级（每合同一行，`tpl_id` → 范例）。
//!
//! 边界：本写件不写 `zc_id_agreement`（协定）/ `zc_id_form-term`（条款）——两跳在全局库无行、
//! 平台无坐标声明（`ontology_binding` 不预置实体映射）⇒ 按其「无映射来源的实体不写 coordinates」
//! 与 `check-leaf-insert` 规约 2（`_f_`/`_t_` MUST 由 `dk_function` 派生）**MUST 报缺不写**；
//! 模型中心登记坐标后由 change 后续补全（见 design.md §4）。

use sqlx::PgConnection;

use common::AliothError;

/// 计费货量口径 → Rhai 表达式（**唯一实现**；值域 = 前端下拉四项）。
///
/// 上下文变量 `out_qty` / `in_qty` 由结算求值方注入（取数面：承运凭证 `zc_id_stat-tsp-voucher`
/// `.qk_qty` / `.qk_balance` → `zc_id_scal-weight.mark`；仓储侧 `zc_id_stat-whs-voucher`
/// `.qk_outgo` / `.qk_income`）。「较小值」与 `transport-operations` 既有裁决同口径
/// （实际运输货量 = min(开始货量, 入库货量)，2026-10-08）。
pub fn billing_expression(basis: &str) -> Option<&'static str> {
    match basis {
        "out_qty" => Some("out_qty"),
        "in_qty" => Some("in_qty"),
        "max" => Some("max(out_qty, in_qty)"),
        "min" => Some("min(out_qty, in_qty)"),
        _ => None,
    }
}

/// 口径中文名（标准/公式 notice 用；与前端 `BILLING_BASIS_OPTIONS` 同口径）。
fn billing_label(basis: &str) -> &'static str {
    match basis {
        "out_qty" => "出库货量",
        "in_qty" => "入库货量",
        "max" => "较大值",
        "min" => "较小值",
        _ => "未知口径",
    }
}

/// 标准族坐标 code（声明值；`ontology_binding::resolve_conn` 同源语义，本 crate 内联解析）。
const STD_SCENE: &str = "JE";
const STD_FACTOR: &str = "FBA";
const STD_FN: &str = "↓_AB";

/// 链路写入入参。
pub struct BillingBasisInput<'a> {
    pub contract_id: i64,
    /// 口径（out_qty / in_qty / max / min）
    pub basis: &'a str,
    pub user_id: i64,
}

/// 写「合同 → 标准 → 计算公式」链路（调用方管理事务）。
///
/// 幂等：公式/标准范例按 `code` find-or-create（命中即同步表达式/复用）；标准实例与两桥按合同写。
pub async fn write_billing_basis_chain_tx(
    conn: &mut PgConnection,
    input: &BillingBasisInput<'_>,
) -> Result<(), AliothError> {
    let expr = billing_expression(input.basis).ok_or_else(|| AliothError::Validation {
        field: "transportProduct.billingBasis".into(),
        message: format!("未知计费货量口径：{}", input.basis),
    })?;
    // 写侧 fail-closed 校验（`ALIOTH_ONTOLOGY_SPEC` 表达式唯一引擎：写入点 MUST 调统一校验；
    // 门禁 `check-expression-dialect` R3-write-site-unvalidated）。已知键 = 公式 `context.params`。
    runtime_engine::expression::RhaiExpressionEngine::new()
        .validate(expr, &["out_qty".to_string(), "in_qty".to_string()])
        .map_err(|e| AliothError::Validation {
            field: "transportProduct.billingBasis".into(),
            message: format!("计费货量公式校验失败：{e}"),
        })?;
    let label = billing_label(input.basis);

    // ── 1. 计算公式（口径级共享，公式-数值叶；context MUST = {"engine":"rhai"}）──
    let formula_code = format!("BWB-FMLA-{}", input.basis);
    let formula_notice = format!("计费货量口径公式 · {label}");
    let formula_id: i64 = match sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM isahl."zc_id_form-billing"
           WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(&formula_code)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?
    {
        Some(id) => {
            // 复用行同步表达式（口径语义唯一 ⇒ 写入值与既有值一致，幂等）
            sqlx::query(
                r#"UPDATE isahl."zc_id_form-billing"
                   SET expression = $2, context = '{"engine":"rhai","params":["out_qty","in_qty"]}'::jsonb,
                       updated_at = NOW(), updated_by_id = $3
                   WHERE id = $1 AND deleted_at IS NULL"#,
            )
            .bind(id)
            .bind(expr)
            .bind(input.user_id)
            .execute(&mut *conn)
            .await
            .map_err(AliothError::from_sqlx)?;
            id
        }
        None => sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO isahl."zc_id_form-billing"
               (notice, code, expression, active, context, created_by_id)
               VALUES ($1, $2, $3, true, '{"engine":"rhai","params":["out_qty","in_qty"]}'::jsonb, $4)
               RETURNING id"#,
        )
        .bind(&formula_notice)
        .bind(&formula_code)
        .bind(expr)
        .bind(input.user_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?,
    };

    // ── 2. 标准：范例（口径级共享）+ 实例（合同级）──
    let std_code = format!("BWB-STD-{}", input.basis);
    let std_notice = format!("计费货量口径标准 · {label}");
    let std_tpl: i64 = match sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM isahl."zc_id_stan-operation"
           WHERE code = $1 AND deleted_at IS NULL AND tpl_id IS NULL LIMIT 1"#,
    )
    .bind(&std_code)
    .fetch_optional(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?
    {
        Some(id) => id,
        None => sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO isahl."zc_id_stan-operation"
               (notice, code, _f_, _t_, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, '实现', '范例', $3,
                       (SELECT id FROM isahl."zc_id_scene" WHERE code = $4 LIMIT 1),
                       (SELECT id FROM isahl."zc_id_factor" WHERE code = $5 LIMIT 1),
                       (SELECT id FROM isahl."zc_id_function" WHERE code = $6 LIMIT 1))
               RETURNING id"#,
        )
        .bind(&std_notice)
        .bind(&std_code)
        .bind(input.user_id)
        .bind(STD_SCENE)
        .bind(STD_FACTOR)
        .bind(STD_FN)
        .fetch_one(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?,
    };
    let std_inst: i64 = sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO isahl."zc_id_stan-operation"
           (notice, code, tpl_id, _f_, _t_, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, '实现', '实例', $4,
                   (SELECT id FROM isahl."zc_id_scene" WHERE code = $5 LIMIT 1),
                   (SELECT id FROM isahl."zc_id_factor" WHERE code = $6 LIMIT 1),
                   (SELECT id FROM isahl."zc_id_function" WHERE code = $7 LIMIT 1))
           RETURNING id"#,
    )
    .bind(&std_notice)
    .bind(&std_code)
    .bind(std_tpl)
    .bind(input.user_id)
    .bind(STD_SCENE)
    .bind(STD_FACTOR)
    .bind(STD_FN)
    .fetch_one(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;

    // ── 3. 双桥：合同 → 标准实例；标准实例 → 公式 ──
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contract_rr_standard" (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3)"#,
    )
    .bind(input.contract_id)
    .bind(std_inst)
    .bind(input.user_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_standard_r_formula" (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, $3)"#,
    )
    .bind(std_inst)
    .bind(formula_id)
    .bind(input.user_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{billing_expression, billing_label};

    #[test]
    fn expression_covers_four_bases() {
        assert_eq!(billing_expression("out_qty"), Some("out_qty"));
        assert_eq!(billing_expression("in_qty"), Some("in_qty"));
        assert_eq!(billing_expression("max"), Some("max(out_qty, in_qty)"));
        assert_eq!(billing_expression("min"), Some("min(out_qty, in_qty)"));
        assert_eq!(billing_expression("nope"), None);
        assert_eq!(billing_label("min"), "较小值");
    }
}
