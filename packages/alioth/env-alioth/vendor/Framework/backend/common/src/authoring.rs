//! 作者通道（authoring channel）——「自然语言 → 提案 → **强校验** → 确认」的统一结论与失败面。
//!
//! change `consolidate-authoring-channel`（候选 3）。LLM 那一档已单源（`turn.rs` 的
//! `TurnSpec` / `load_llm` / `run_turn`）；本模块补上**强校验档**的语义单源：
//! 「什么算阻断、什么算逐项标注、什么算警告」在此定义一次，各域校验器作**适配器**接入。
//!
//! 三类既有形态（实测，收敛前的五端点）：
//! 1. 整体 pass/fail + 错误串（`approval_formula` 的 Rhai 校验、`dmn_assist` 的 `{valid,errors}`、
//!    `doc_recognition` 的单据成形）⇒ [`AuthoringVerdict::from_messages`]
//! 2. 逐项标注、**仍留在提案里展示**（`admin_ngac_assist` 的 `invalid_reason`）⇒
//!    [`AuthoringVerdict::with_item`]（`valid` 仍为 true）
//! 3. 非阻断降级（`knowledge` 的词扩展）⇒ [`AuthoringVerdict::warn`]
//!
//! 纪律：结论是**纯数据**、判定是**纯函数**（无 LLM、无 I/O）⇒ 可脱离大模型单测。
//! 本模块**不**表达「写回目标」（那属动作目录面，见 change `add-action-catalogue`）。

use serde::Serialize;
use serde_json::{json, Value};

/// 强校验中的一条问题。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    /// 定位：字段路径 / 表达式标识符 / 操作序号（字符串形式，便于各域复用同一类型）。
    pub path: String,
    pub message: String,
    /// `Some(i)` = 第 `i` 项无效（该项仍留在提案中展示，**不整体阻断**）；
    /// `None` = 整体性问题（**阻断**）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_index: Option<usize>,
}

impl Violation {
    /// 整体性问题（阻断项）。
    pub fn overall(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            item_index: None,
        }
    }

    /// 逐项问题（该项无效但保留在提案里展示）。
    pub fn item(index: usize, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            item_index: Some(index),
        }
    }

    /// 是否属整体性（= 是否触发阻断）。**失败面的唯一判据**。
    pub fn is_blocking(&self) -> bool {
        self.item_index.is_none()
    }
}

/// 强校验结论（纯数据）。`valid` 与 `violations` 的关系由构造入口维护，
/// MUST NOT 由调用方各自推算（否则又多一处「什么算阻断」的判定）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthoringVerdict {
    /// 是否可进入确认/写回（存在**整体性** violation 即 false）
    pub valid: bool,
    pub violations: Vec<Violation>,
    /// 非阻断提示（降级路径也保留）
    pub warnings: Vec<String>,
}

impl Default for AuthoringVerdict {
    fn default() -> Self {
        Self {
            valid: true,
            violations: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

impl AuthoringVerdict {
    /// 通过（无问题）。
    pub fn ok() -> Self {
        Self::default()
    }

    /// 形态 1（有定位）：整体性错误 ⇒ `valid=false`。
    pub fn from_errors<I, P, M>(errors: I) -> Self
    where
        I: IntoIterator<Item = (P, M)>,
        P: Into<String>,
        M: Into<String>,
    {
        let violations: Vec<Violation> = errors
            .into_iter()
            .map(|(p, m)| Violation::overall(p, m))
            .collect();
        Self {
            valid: violations.is_empty(),
            violations,
            warnings: Vec::new(),
        }
    }

    /// 形态 1（仅错误串，无定位）：`path` 用调用方给的域标签（如 `"expression"` / `"dmn"`）。
    pub fn from_messages<S: Into<String>>(domain: &str, messages: Vec<S>) -> Self {
        Self::from_errors(
            messages
                .into_iter()
                .map(|m| (domain.to_string(), m.into()))
                .collect::<Vec<_>>(),
        )
    }

    /// 形态 2：逐项问题——**不整体阻断**（`valid` 保持 true）。
    pub fn with_item(
        mut self,
        index: usize,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        self.violations.push(Violation::item(index, path, message));
        // valid 不变：逐项标注不是整体失败（与 admin_ngac_assist 现状一致）
        self
    }

    /// 形态 3：非阻断警告——不影响 `valid`。
    pub fn warn(mut self, message: impl Into<String>) -> Self {
        self.warnings.push(message.into());
        self
    }

    /// 合并两份结论：violations/warnings 取并集，`valid` 取与。
    pub fn merge(mut self, other: Self) -> Self {
        self.valid = self.valid && other.valid;
        self.violations.extend(other.violations);
        self.warnings.extend(other.warnings);
        self
    }

    /// 整体性（阻断）问题。
    pub fn blocking(&self) -> impl Iterator<Item = &Violation> {
        self.violations.iter().filter(|v| v.is_blocking())
    }

    /// 是否存在阻断项。**端点判断自身是否该报错 MUST 用本方法**（不得按域规则重判）。
    pub fn is_blocking(&self) -> bool {
        self.blocking().next().is_some()
    }

    /// 统一失败面（默认路径）：有整体性问题 ⇒ 400 + 统一体；否则 `None`（不阻断）。
    ///
    /// 端点若其响应字段是既有对外契约，MAY 保留自有响应体——但**阻断判定 MUST 来自本类型**。
    pub fn into_http_error(&self) -> Option<(u16, Value)> {
        if !self.is_blocking() {
            return None;
        }
        Some((400, self.to_response_json()))
    }

    /// 结论的 JSON 形态（`{valid, errors, warnings}`）——与既有端点字段名对齐，
    /// 便于「对外行为不变」的接入。
    pub fn to_response_json(&self) -> Value {
        json!({
            "valid": self.valid,
            "errors": self
                .violations
                .iter()
                .map(|v| json!({
                    "path": v.path,
                    "message": v.message,
                    "item_index": v.item_index,
                }))
                .collect::<Vec<_>>(),
            "warnings": self.warnings,
        })
    }

    /// 错误消息清单（保形用）：既有端点以 `errors: Vec<String>` 为对外字段，
    /// 接入时以本方法取同一内容，**对外行为不变**。
    pub fn messages(&self) -> Vec<String> {
        self.violations.iter().map(|v| v.message.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_messages_blocks_when_non_empty() {
        let bad = AuthoringVerdict::from_messages("expression", vec!["语法错误"]);
        assert!(!bad.valid);
        assert!(bad.is_blocking());
        assert_eq!(bad.into_http_error().map(|(c, _)| c), Some(400));

        let good = AuthoringVerdict::from_messages("expression", Vec::<String>::new());
        assert!(good.valid);
        assert!(!good.is_blocking());
        assert!(good.into_http_error().is_none(), "无整体性问题不得阻断");
    }

    #[test]
    fn item_level_violation_does_not_block() {
        // 形态 2 红线：逐项标注不整体失败（admin_ngac_assist 语义）
        let v = AuthoringVerdict::ok().with_item(2, "operations[2]", "引用未命中快照");
        assert!(v.valid, "逐项问题 MUST NOT 使整体无效");
        assert!(!v.is_blocking(), "逐项问题 MUST NOT 触发失败面");
        assert!(v.into_http_error().is_none());
        assert_eq!(v.violations[0].item_index, Some(2));
    }

    #[test]
    fn warning_does_not_affect_validity() {
        // 形态 3：降级（knowledge 词扩展）不阻断
        let v = AuthoringVerdict::ok().warn("词扩展不可用，已用原词检索");
        assert!(v.valid);
        assert!(!v.is_blocking());
        assert_eq!(v.warnings.len(), 1);
    }

    #[test]
    fn merge_intersects_validity_and_unions_issues() {
        let a = AuthoringVerdict::from_messages("dmn", vec!["输入列名重复"]);
        let b = AuthoringVerdict::ok()
            .warn("降级：无 LLM 夹具")
            .with_item(0, "ops[0]", "未知操作");
        let m = a.merge(b);
        assert!(!m.valid, "有整体性错误 ⇒ 合并后仍无效");
        assert_eq!(m.violations.len(), 2);
        assert_eq!(m.warnings.len(), 1);
        assert!(m.is_blocking());
    }

    #[test]
    fn response_json_carries_three_shapes() {
        let v = AuthoringVerdict::from_messages("expression", vec!["非法标识符 x"])
            .with_item(1, "ops[1]", "重复")
            .warn("提示");
        let j = v.to_response_json();
        assert_eq!(j["valid"], false);
        assert_eq!(j["errors"].as_array().unwrap().len(), 2);
        assert_eq!(j["errors"][0]["item_index"], Value::Null); // 整体项无序号
        assert_eq!(j["errors"][1]["item_index"], 1);
        assert_eq!(j["warnings"][0], "提示");
    }

    #[test]
    fn no_io_no_llm_pure_data() {
        // 纯数据：可脱离 LLM/DB 断言（本 change 的可测性目标）
        let v = AuthoringVerdict::ok();
        assert_eq!(v, AuthoringVerdict::default());
    }
}
