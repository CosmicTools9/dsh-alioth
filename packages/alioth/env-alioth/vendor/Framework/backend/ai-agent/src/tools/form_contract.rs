//! 表单契约（change `add-form-contract-seam`，ADR D-053）
//!
//! 契约 = 页面表单派生出的机器可读描述（字段名/类型/必填/枚举/范围/格式 + `dto` 元数据）。
//! 本模块是 **Rust 侧唯一解释器**：助手校验 MUST 以契约为准，MUST NOT 使用模型自备 schema
//! （后者为「可校验任意 schema 的第二实现」，已随本 change 退役）。
//!
//! 线协议（与前端 `contractFromZodSchema` 同形，camelCase）：
//! `{ fields: [{ name, type, required, enum?, minimum?, maximum?, minLength?, maxLength?,
//!   pattern?, dto?, nonPortable? }] }`
//!
//! fail-closed 语义（design D7）：
//! - **契约不可用**（未登记/不可解析/超限截断）⇒ 调用方 MUST 以 `CONTRACT_UNAVAILABLE` 拒绝，
//!   MUST NOT 放行、MUST NOT 回落模型自备 schema；
//! - **`.refine()` 类规则不可派生**（`nonPortable`）⇒ 该字段**未受校验覆盖**，MUST NOT
//!   被表述为「已通过」；其结果由 `ValidationOutcome::uncovered` 显式上报。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `dto` 元数据（`DESIGN_FORM_SPEC §1.5.1` / `DTO_DESIGN_SPEC §8.6` 既有形状；解析口径与前端
/// `parseDtoMeta` 一致）。本模块只承载与展示，不做绑定决策。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DtoMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_options: Option<Vec<DtoUnitOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_left_unique: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DtoUnitOption {
    pub value: String,
    pub label: String,
}

/// 契约字段。`type` 为 JSON Schema 派生词表；未知值按 `string` 处理（诚实退化，不猜规则）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormContractField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub required: bool,
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dto: Option<DtoMeta>,
    /// 规则派生不可表达（`.refine()`/`.superRefine()`）⇒ 未受契约校验覆盖
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_portable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormContract {
    pub fields: Vec<FormContractField>,
}

/// 单字段错误（`{field, error, message}` 口径与既有表单校验一致）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldError {
    pub field: String,
    pub error: String,
    pub message: String,
}

/// 校验结果：`errors` 为空且 `uncovered` 为空才是「可应用」。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ValidationOutcome {
    pub errors: Vec<FieldError>,
    /// 未受契约校验覆盖的字段（`nonPortable`）——MUST NOT 表述为已通过
    pub uncovered: Vec<String>,
    /// 是否可应用（= 无错误且无未覆盖字段）；前端「应用到当前表单」据此门控
    pub appliable: bool,
}

fn push(errs: &mut Vec<FieldError>, field: &str, error: &str, message: String) {
    errs.push(FieldError {
        field: field.to_string(),
        error: error.to_string(),
        message,
    });
}

/// 值是否满足契约声明的 type（`date` 只与字符串相容——前端表单一律以字符串承载日期）。
fn value_matches_type(value: &Value, declared: &str) -> bool {
    match declared {
        "string" | "date" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        // 未知声明：不误报（与既有实现同口径）
        _ => true,
    }
}

/// 按契约校验字段值（纯函数，无 I/O）。
///
/// 语义：
/// - `values` 非 object ⇒ 根级错误；
/// - `required` 缺值（缺失或 `null`）⇒ `required` 错误；
/// - type 不符 ⇒ 该字段跳过其余规则（避免级联噪音）；
/// - 数值规则只作用于数值、长度/pattern 只作用于字符串；
/// - `pattern` 非法正则 = 契约作者错误 ⇒ 跳过该规则（不阻断其余校验，也不误报）；
/// - `nonPortable` 字段**不产生错误**，但记入 `uncovered`。
pub fn validate_against_contract(contract: &FormContract, values: &Value) -> ValidationOutcome {
    let mut out = ValidationOutcome::default();

    let Some(map) = values.as_object() else {
        push(
            &mut out.errors,
            "",
            "type",
            "字段值必须是对象（field_values）".to_string(),
        );
        out.appliable = false;
        return out;
    };

    for field in &contract.fields {
        if field.non_portable == Some(true) {
            out.uncovered.push(field.name.clone());
        }

        let value = map.get(&field.name);

        if field.required && value.is_none_or(Value::is_null) {
            push(
                &mut out.errors,
                &field.name,
                "required",
                format!("{} 为必填", field.name),
            );
            continue;
        }

        let Some(value) = value else { continue };
        if value.is_null() {
            continue;
        }

        if !value_matches_type(value, &field.field_type) {
            push(
                &mut out.errors,
                &field.name,
                "type",
                format!("{} 类型应为 {}", field.name, field.field_type),
            );
            continue;
        }

        if let Some(allowed) = &field.enum_values {
            if !allowed.iter().any(|a| a == value) {
                push(
                    &mut out.errors,
                    &field.name,
                    "enum",
                    format!("{} 取值不在允许集合内", field.name),
                );
                continue;
            }
        }

        if let Some(n) = value.as_f64() {
            if let Some(min) = field.minimum {
                if n < min {
                    push(
                        &mut out.errors,
                        &field.name,
                        "minimum",
                        format!("{} 不得小于 {}", field.name, min),
                    );
                }
            }
            if let Some(max) = field.maximum {
                if n > max {
                    push(
                        &mut out.errors,
                        &field.name,
                        "maximum",
                        format!("{} 不得大于 {}", field.name, max),
                    );
                }
            }
        }

        if let Some(s) = value.as_str() {
            let len = s.chars().count() as u64;
            if let Some(min) = field.min_length {
                if len < min {
                    push(
                        &mut out.errors,
                        &field.name,
                        "minLength",
                        format!("{} 长度不得少于 {}", field.name, min),
                    );
                }
            }
            if let Some(max) = field.max_length {
                if len > max {
                    push(
                        &mut out.errors,
                        &field.name,
                        "maxLength",
                        format!("{} 长度不得超过 {}", field.name, max),
                    );
                }
            }
            if let Some(pattern) = &field.pattern {
                // 非法正则 = 契约作者错误 ⇒ 跳过（不阻断其余校验，也不误报）
                if let Ok(re) = regex::Regex::new(pattern) {
                    if !re.is_match(s) {
                        push(
                            &mut out.errors,
                            &field.name,
                            "pattern",
                            format!("{} 不符合格式要求", field.name),
                        );
                    }
                }
            }
        }
    }

    out.appliable = out.errors.is_empty() && out.uncovered.is_empty();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn contract(fields: Vec<FormContractField>) -> FormContract {
        FormContract { fields }
    }

    fn field(name: &str, ty: &str, required: bool) -> FormContractField {
        FormContractField {
            name: name.to_string(),
            field_type: ty.to_string(),
            required,
            enum_values: None,
            minimum: None,
            maximum: None,
            min_length: None,
            max_length: None,
            pattern: None,
            dto: None,
            non_portable: None,
        }
    }

    #[test]
    fn required_missing_and_null_both_error() {
        let c = contract(vec![field("transNo", "string", true)]);
        let out = validate_against_contract(&c, &json!({}));
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].error, "required");
        let out = validate_against_contract(&c, &json!({ "transNo": null }));
        assert_eq!(out.errors[0].error, "required");
    }

    #[test]
    fn optional_field_absent_is_ok() {
        let c = contract(vec![field("summary", "string", false)]);
        let out = validate_against_contract(&c, &json!({}));
        assert!(out.errors.is_empty());
        assert!(out.appliable);
    }

    #[test]
    fn type_mismatch_skips_other_rules() {
        let mut f = field("qty", "integer", true);
        f.minimum = Some(0.0);
        let c = contract(vec![f]);
        let out = validate_against_contract(&c, &json!({ "qty": "abc" }));
        assert_eq!(out.errors.len(), 1, "type 不符后不应级联出 minimum 错误");
        assert_eq!(out.errors[0].error, "type");
    }

    #[test]
    fn enum_membership() {
        let mut f = field("status", "string", true);
        f.enum_values = Some(vec![json!("draft"), json!("approved")]);
        let c = contract(vec![f]);
        assert!(validate_against_contract(&c, &json!({ "status": "draft" }))
            .errors
            .is_empty());
        let out = validate_against_contract(&c, &json!({ "status": "other" }));
        assert_eq!(out.errors[0].error, "enum");
    }

    #[test]
    fn numeric_and_length_and_pattern_rules() {
        let mut qty = field("qty", "number", true);
        qty.minimum = Some(0.0);
        qty.maximum = Some(100.0);
        let mut code = field("code", "string", true);
        code.min_length = Some(2);
        code.max_length = Some(4);
        code.pattern = Some("^[A-Z]+$".to_string());
        let c = contract(vec![qty, code]);

        assert!(
            validate_against_contract(&c, &json!({ "qty": 5.0, "code": "AB" }))
                .errors
                .is_empty()
        );

        let out = validate_against_contract(&c, &json!({ "qty": 101.0, "code": "a" }));
        let kinds: Vec<&str> = out.errors.iter().map(|e| e.error.as_str()).collect();
        assert!(kinds.contains(&"maximum"));
        assert!(kinds.contains(&"minLength"));
        assert!(kinds.contains(&"pattern"));
    }

    #[test]
    fn length_counts_chars_not_bytes() {
        let mut f = field("name", "string", true);
        f.max_length = Some(2);
        let c = contract(vec![f]);
        assert!(validate_against_contract(&c, &json!({ "name": "张三" }))
            .errors
            .is_empty());
    }

    #[test]
    fn invalid_pattern_is_skipped_not_reported() {
        let mut f = field("code", "string", true);
        f.pattern = Some("[unclosed".to_string());
        let c = contract(vec![f]);
        assert!(validate_against_contract(&c, &json!({ "code": "x" }))
            .errors
            .is_empty());
    }

    #[test]
    fn array_type_for_m2n() {
        let c = contract(vec![field("tags", "array", false)]);
        assert!(validate_against_contract(&c, &json!({ "tags": [] }))
            .errors
            .is_empty());
        let out = validate_against_contract(&c, &json!({ "tags": "x" }));
        assert_eq!(out.errors[0].error, "type");
    }

    #[test]
    fn non_portable_is_uncovered_and_blocks_appliable() {
        let mut f = field("amount", "string", true);
        f.non_portable = Some(true);
        let c = contract(vec![f]);
        let out = validate_against_contract(&c, &json!({ "amount": "1" }));
        assert!(out.errors.is_empty(), "nonPortable 不产生值错误");
        assert_eq!(out.uncovered, vec!["amount".to_string()]);
        assert!(!out.appliable, "未受覆盖 ⇒ 不可应用（MUST NOT 假装通过）");
    }

    #[test]
    fn root_must_be_object() {
        let c = contract(vec![field("a", "string", false)]);
        let out = validate_against_contract(&c, &json!([1, 2]));
        assert!(!out.appliable);
        assert_eq!(out.errors[0].error, "type");
    }

    #[test]
    fn wire_shape_is_camel_case() {
        // 契约 wire 形状 MUST 与前端 `contractFromZodSchema` 一致（camelCase）
        let raw = json!({
            "fields": [{
                "name": "counterpartyId",
                "type": "string",
                "required": true,
                "minLength": 1,
                "dto": { "kind": "m2o", "endpoint": "/service/isahl-db/subjects", "labelField": "notice" },
                "nonPortable": true
            }]
        });
        let c: FormContract = serde_json::from_value(raw).expect("camelCase 契约可反序列化");
        assert_eq!(c.fields[0].field_type, "string");
        assert_eq!(c.fields[0].min_length, Some(1));
        assert_eq!(c.fields[0].non_portable, Some(true));
        assert_eq!(
            c.fields[0].dto.as_ref().unwrap().label_field.as_deref(),
            Some("notice")
        );
        // enum 键按 wire 名 `enum` 读取
        let c2: FormContract = serde_json::from_value(
            json!({"fields":[{"name":"s","type":"string","required":true,"enum":["a"]}]}),
        )
        .unwrap();
        assert_eq!(c2.fields[0].enum_values.as_ref().unwrap().len(), 1);
    }
}
