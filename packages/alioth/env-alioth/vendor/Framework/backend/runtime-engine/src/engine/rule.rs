//! 业务规则引擎
//!
//! 执行业务规则：条件-动作模式。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// 业务规则配置（由 LLM-Agent 生成）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessRuleConfig {
    pub entity: String,
    pub rule_name: String,
    pub trigger: String,
    pub condition: String,
    pub action: String,
    pub priority: i32,
    pub error_message: String,
    /// 是否为阻塞规则（条件成立时阻止操作，返回 error_message）
    #[serde(default = "default_true")]
    pub blocking: bool,
}

fn default_true() -> bool {
    true
}

/// 单条规则执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExecution {
    pub rule_name: String,
    pub entity: String,
    pub triggered: bool,
    pub executed: bool,
    pub mutations: Vec<(String, Value)>,
    pub error: Option<String>,
}

/// 批量规则执行结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuleExecutionResult {
    pub executions: Vec<RuleExecution>,
    pub all_passed: bool,
    pub mutations: HashMap<String, Value>,
    pub errors: Vec<String>,
    /// 求解层：是否到达不动点（单轮 `execute` 恒 false——未做依赖传播）
    #[serde(default)]
    pub saturated: bool,
    /// 求解层：实际轮次（单轮入口恒 0）
    #[serde(default)]
    pub rounds: usize,
    /// 求解层：达轮数上限时仍待处理的规则名（MUST NOT 静默截断）
    #[serde(default)]
    pub remaining_triggers: Vec<String>,
    /// 求解层：冲突裁决留痕（确定性策略，可审计）
    #[serde(default)]
    pub conflicts: Vec<ConflictRecord>,
    /// 求解层：降级原因（如读集合不可 AST 提取 ⇒ 退回单轮顺序执行）
    #[serde(default)]
    pub degraded: Option<String>,
}

/// 同字段多写的裁决策略（确定性；默认先写者胜）
///
/// 真身 = `runtime_contract::extension::ConflictPolicy`（扩展配置面声明所用同一枚举；
/// engine 侧只 `re-export`，禁止另立第二份）。
pub use runtime_contract::extension::ConflictPolicy;

/// 冲突裁决留痕条目
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictRecord {
    /// 被争写的字段
    pub field: String,
    /// 获胜规则名
    pub winner: String,
    /// 被覆盖（或被拒）的规则名
    pub loser: String,
    /// 裁决依据
    pub policy: ConflictPolicy,
}

/// 迭代求解配置
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaturationConfig {
    /// 轮数上限（兜底终止；达上限 MUST 留痕）
    pub max_rounds: usize,
    /// 同字段多写裁决策略
    pub on_conflict: ConflictPolicy,
}

impl Default for SaturationConfig {
    fn default() -> Self {
        Self {
            max_rounds: 8,
            on_conflict: ConflictPolicy::FirstWins,
        }
    }
}

impl SaturationConfig {
    /// 由扩展配置视图构造（`extensions/rule_execution.yaml`）。
    ///
    /// `max_rounds` 缺省取引擎默认；显式 `0` 按 1 处理（`MUST NOT` 出现「零轮求解」
    /// 这种等价于静默不执行的上限）。
    pub fn from_extension(view: runtime_contract::extension::SaturationConfigView) -> Self {
        let defaults = Self::default();
        Self {
            max_rounds: std::cmp::max(
                1,
                view.max_rounds
                    .map(|m| m as usize)
                    .unwrap_or(defaults.max_rounds),
            ),
            on_conflict: view.on_conflict,
        }
    }
}

impl RuleExecutionResult {
    pub fn new() -> Self {
        Self {
            executions: Vec::new(),
            all_passed: true,
            mutations: HashMap::new(),
            errors: Vec::new(),
            saturated: false,
            rounds: 0,
            remaining_triggers: Vec::new(),
            conflicts: Vec::new(),
            degraded: None,
        }
    }

    pub fn add(&mut self, exec: RuleExecution) {
        if exec.error.is_some() {
            self.all_passed = false;
            self.errors.push(exec.error.clone().unwrap());
        }
        for (field, value) in &exec.mutations {
            self.mutations.insert(field.clone(), value.clone());
        }
        self.executions.push(exec);
    }
}

/// 业务规则引擎
pub struct RuleEngine;

impl RuleEngine {
    /// 执行一组业务规则（**单轮顺序**；语义与既有一致，不做依赖传播 ⇒ `saturated = false`）。
    /// 需要链式传播/不动点求解 ⇒ 用 [`Self::saturate`]。
    pub fn execute(
        rules: &[BusinessRuleConfig],
        variables: &mut HashMap<String, Value>,
    ) -> RuleExecutionResult {
        let mut result = RuleExecutionResult::new();

        let mut sorted_rules: Vec<_> = rules.iter().collect();
        sorted_rules.sort_by_key(|r| r.priority);

        for config in sorted_rules {
            let exec = Self::execute_single(config, variables);
            result.add(exec);
        }

        result
    }

    /// 迭代求解（依赖驱动 + 确定性冲突消解），迭代到**不动点**或轮数上限。
    ///
    /// - 读集合 = condition 的 AST 变量引用；写集合 = action 左值（MUST NOT 文本猜测）；
    ///   condition 不可 AST 提取 ⇒ `degraded` 留痕并退回单轮顺序执行（MUST NOT 静默）。
    /// - 不动点判据 = 整轮**无字段值变化**（幂等写入不计进展 ⇒ 收敛即停）；每轮每规则至多一次。
    /// - 同轮同字段多写 ⇒ 按 `cfg.on_conflict` 裁决并记入 `conflicts`（规则序 = priority 升序 → 规则名稳定序）。
    /// - 达 `max_rounds` ⇒ `saturated = false` + `remaining_triggers`（MUST NOT 静默截断）。
    /// - 屏蔽规则（`blocking`）语义不变：条件成立即记错误、不写入，且只参与首轮。
    pub fn saturate(
        rules: &[BusinessRuleConfig],
        variables: &mut HashMap<String, Value>,
        cfg: &SaturationConfig,
    ) -> RuleExecutionResult {
        let mut ordered: Vec<&BusinessRuleConfig> = rules.iter().collect();
        ordered.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.rule_name.cmp(&b.rule_name))
        });

        // 读集合（AST）——任一不可提取即降级（保守，不猜语义）
        let mut reads: HashMap<String, Vec<String>> = HashMap::new();
        for rule in &ordered {
            match Self::read_fields(&rule.condition) {
                Ok(fields) => {
                    reads.insert(rule.rule_name.clone(), fields);
                }
                Err(e) => {
                    let mut fallback = Self::execute(rules, variables);
                    fallback.saturated = false;
                    fallback.degraded = Some(format!(
                        "condition 不可 AST 提取（规则 {}）：{e}",
                        rule.rule_name
                    ));
                    return fallback;
                }
            }
            // 写侧契约可解析性（静态图完整性）：非空 action MUST 符合 `字段 = 表达式`（可多段 `;`）
            if !rule.action.trim().is_empty() {
                if let Err(e) = Self::action_assignments(&rule.action) {
                    let mut fallback = Self::execute(rules, variables);
                    fallback.saturated = false;
                    fallback.degraded =
                        Some(format!("action 不可解析（规则 {}）：{e}", rule.rule_name));
                    return fallback;
                }
            }
        }

        let mut result = RuleExecutionResult::new();
        let mut eligible: Vec<&BusinessRuleConfig> = ordered.clone();
        let mut rounds = 0usize;

        loop {
            if eligible.is_empty() {
                result.saturated = true;
                break;
            }
            if rounds >= cfg.max_rounds {
                result.saturated = false;
                result.remaining_triggers = eligible.iter().map(|r| r.rule_name.clone()).collect();
                break;
            }
            rounds += 1;

            let round_start = variables.clone();
            let mut staged = variables.clone();
            let mut written_by: HashMap<String, String> = HashMap::new();
            let mut first_value: HashMap<String, Value> = HashMap::new();

            for rule in &eligible {
                let name = rule.rule_name.clone();
                let condition_met = match Self::evaluate_condition(&rule.condition, &staged) {
                    Ok(v) => v,
                    Err(e) => {
                        let msg = format!("Condition evaluation failed: {e}");
                        result.add(RuleExecution {
                            rule_name: name,
                            entity: rule.entity.clone(),
                            triggered: false,
                            executed: false,
                            mutations: Vec::new(),
                            error: Some(msg),
                        });
                        continue;
                    }
                };
                if !condition_met {
                    result.add(RuleExecution {
                        rule_name: name,
                        entity: rule.entity.clone(),
                        triggered: false,
                        executed: false,
                        mutations: Vec::new(),
                        error: None,
                    });
                    continue;
                }
                if rule.blocking {
                    // 校验型规则（条件成立 = 判违规）：`error_message` 为空时 MUST 有可诊断文案，
                    // 否则阻断以空字符串上抛（响应文案为 ""，无法定位是哪条声明）
                    let msg = if rule.error_message.trim().is_empty() {
                        format!("规则 {} 校验未通过", name)
                    } else {
                        rule.error_message.clone()
                    };
                    result.add(RuleExecution {
                        rule_name: name,
                        entity: rule.entity.clone(),
                        triggered: true,
                        executed: false,
                        mutations: Vec::new(),
                        error: Some(msg),
                    });
                    continue;
                }

                match Self::execute_action(&rule.action, &mut staged) {
                    Ok(mutations) => {
                        let mut kept: Vec<(String, Value)> = Vec::new();
                        let mut error: Option<String> = None;
                        for (field, value) in mutations {
                            match written_by.get(&field) {
                                Some(owner) if owner != &name => match cfg.on_conflict {
                                    ConflictPolicy::LastWins => {
                                        result.conflicts.push(ConflictRecord {
                                            field: field.clone(),
                                            winner: name.clone(),
                                            loser: owner.clone(),
                                            policy: cfg.on_conflict,
                                        });
                                        staged.insert(field.clone(), value.clone());
                                        written_by.insert(field.clone(), name.clone());
                                        first_value.insert(field.clone(), value.clone());
                                        kept.push((field, value));
                                    }
                                    ConflictPolicy::FirstWins => {
                                        result.conflicts.push(ConflictRecord {
                                            field: field.clone(),
                                            winner: owner.clone(),
                                            loser: name.clone(),
                                            policy: cfg.on_conflict,
                                        });
                                        if let Some(prev) = first_value.get(&field) {
                                            staged.insert(field.clone(), prev.clone());
                                        }
                                    }
                                    ConflictPolicy::Error => {
                                        let msg = format!(
                                                "规则冲突：字段「{field}」被「{owner}」与「{name}」同时写入"
                                            );
                                        result.conflicts.push(ConflictRecord {
                                            field: field.clone(),
                                            winner: owner.clone(),
                                            loser: name.clone(),
                                            policy: cfg.on_conflict,
                                        });
                                        if let Some(prev) = first_value.get(&field) {
                                            staged.insert(field.clone(), prev.clone());
                                        }
                                        error = Some(msg);
                                    }
                                },
                                _ => {
                                    written_by.insert(field.clone(), name.clone());
                                    first_value.insert(field.clone(), value.clone());
                                    kept.push((field, value));
                                }
                            }
                        }
                        result.add(RuleExecution {
                            rule_name: name,
                            entity: rule.entity.clone(),
                            triggered: true,
                            executed: error.is_none(),
                            mutations: kept,
                            error,
                        });
                    }
                    Err(e) => {
                        result.add(RuleExecution {
                            rule_name: name,
                            entity: rule.entity.clone(),
                            triggered: true,
                            executed: false,
                            mutations: Vec::new(),
                            error: Some(e),
                        });
                    }
                }
            }

            let mut changed: Vec<String> = Vec::new();
            for field in written_by.keys() {
                let now = staged.get(field);
                if round_start.get(field) != now {
                    changed.push(field.clone());
                    if let Some(v) = now {
                        result.mutations.insert(field.clone(), v.clone());
                    }
                }
            }
            *variables = staged;

            if changed.is_empty() {
                result.saturated = true;
                break;
            }

            // 依赖驱动：下一轮候选 = 读集合与「本轮变化字段」相交的非屏蔽规则
            eligible = ordered
                .iter()
                .filter(|r| {
                    !r.blocking
                        && reads
                            .get(&r.rule_name)
                            .is_some_and(|rs| rs.iter().any(|f| changed.contains(f)))
                })
                .copied()
                .collect();
        }

        result.rounds = rounds;
        result
    }

    fn execute_single(
        config: &BusinessRuleConfig,
        variables: &mut HashMap<String, Value>,
    ) -> RuleExecution {
        let mut exec = RuleExecution {
            rule_name: config.rule_name.clone(),
            entity: config.entity.clone(),
            triggered: false,
            executed: false,
            mutations: Vec::new(),
            error: None,
        };

        // 1. 评估条件
        let condition_met = match Self::evaluate_condition(&config.condition, variables) {
            Ok(met) => met,
            Err(e) => {
                exec.error = Some(format!("Condition evaluation failed: {}", e));
                return exec;
            }
        };

        exec.triggered = condition_met;
        if !condition_met {
            return exec;
        }

        // 2. 阻塞规则：条件成立即阻止操作，不执行动作
        if config.blocking {
            exec.error = Some(config.error_message.clone());
            return exec;
        }

        // 3. 非阻塞规则：执行动作（字段赋值/副作用）
        match Self::execute_action(&config.action, variables) {
            Ok(mutations) => {
                exec.executed = true;
                exec.mutations = mutations;
            }
            Err(e) => {
                exec.error = Some(format!("Action execution failed: {}", e));
            }
        }

        exec
    }

    /// 条件求值（唯一引擎：Rhai 沙箱；真值语义见 `expression::is_truthy`）
    fn evaluate_condition(
        condition: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<bool, String> {
        let value = crate::expression::RhaiExpressionEngine::new()
            .evaluate(condition, variables)
            .map_err(|e| format!("Evaluation error: {e}"))?;
        Ok(crate::expression::is_truthy(&value))
    }

    /// action 顶层 `;` 切分（尊重引号与括号嵌套）——多重赋值契约的基础。
    fn split_assignments(action: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut buf = String::new();
        let mut depth = 0i32;
        let mut quote: Option<char> = None;
        let mut prev = '\0';
        for ch in action.chars() {
            if let Some(q) = quote {
                buf.push(ch);
                if ch == q && prev != '\\' {
                    quote = None;
                }
                prev = ch;
                continue;
            }
            match ch {
                '"' | '\'' | '`' => {
                    quote = Some(ch);
                    buf.push(ch);
                }
                '(' | '[' | '{' => {
                    depth += 1;
                    buf.push(ch);
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    buf.push(ch);
                }
                ';' if depth == 0 => {
                    let seg = buf.trim().to_string();
                    if !seg.is_empty() {
                        out.push(seg);
                    }
                    buf.clear();
                }
                _ => buf.push(ch),
            }
            prev = ch;
        }
        let last = buf.trim().to_string();
        if !last.is_empty() {
            out.push(last);
        }
        out
    }

    /// action → `[(字段, 表达式)]`。契约：`字段 = 表达式`（字面 `=` 首个即左值右侧分界）；
    /// 多重赋值以顶层 `;` 分隔，按序执行且**后者可见前者结果**（见 `execute_action`）。
    ///
    /// 唯一分段实现：运行期（`execute_action`）与加载期校验（`collect_expression_violations`）共用，
    /// MUST NOT 出现第二套分段口径（否则「加载期通过、运行期炸」）。
    pub(crate) fn action_assignments(action: &str) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        for seg in Self::split_assignments(action) {
            let Some(pos) = seg.find('=') else {
                return Err(format!("Unsupported action format: {seg}"));
            };
            let field = seg[..pos].trim();
            let expr = seg[pos + 1..].trim();
            if field.is_empty() {
                return Err(format!("Unsupported action format (空左值): {seg}"));
            }
            out.push((field.to_string(), expr.to_string()));
        }
        if out.is_empty() {
            return Err(format!("Unsupported action format: {action}"));
        }
        Ok(out)
    }

    /// 读集合（condition 引用变量，AST 提取；MUST NOT 文本猜测）。不可提取 → Err（含原因）。
    fn read_fields(condition: &str) -> Result<Vec<String>, String> {
        crate::expression::analysis::collect_variables(condition)
    }

    /// 动作执行（契约：`字段 = 表达式`；RHS 由 Rhai 求值）。
    /// 多重赋值按序执行，且**后者可见前者结果**（`a = 1; b = a + 1` ⇒ b = 2）。
    ///
    /// 唯一动作执行器：规则面（`execute_rule`）与状态机 transition `action`（`extension.rs`）共用，
    /// MUST NOT 出现第二套 `字段 = 表达式` 求值口径。
    pub(crate) fn execute_action(
        action: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<Vec<(String, Value)>, String> {
        let assignments = Self::action_assignments(action)?;
        let mut mutations = Vec::new();
        for (field, expr) in assignments {
            let new_value = crate::expression::RhaiExpressionEngine::new()
                .evaluate(&expr, variables)
                .map_err(|e| format!("Action expression error: {e}"))?;
            variables.insert(field.clone(), new_value.clone());
            mutations.push((field, new_value));
        }
        Ok(mutations)
    }
}

/// 规则集**写集合 → 读集合**依赖边（`(写者下标, 读者下标)`，i ≠ j）。
///
/// 与 `RuleEngine::saturate` 的图构建**共用同一提取实现**（`read_fields` / `action_assignments`）：
/// 写集合 = action 左值；读集合 = condition 变量 ∪ 各 action 右值变量。
/// 供 **composer** 判定是否需产出 `extensions/rule_execution.yaml`（应用级饱和求解声明）——
/// 依赖非空 ⇒ 单轮顺序执行无法传播，须饱和。
///
/// 任一表达式不可 AST 提取 ⇒ `Err`（调用方自行决定降级；**MUST NOT** 文本猜测语义）。
pub fn rule_dependency_edges(rules: &[BusinessRuleConfig]) -> Result<Vec<(usize, usize)>, String> {
    let mut writes: Vec<Vec<String>> = Vec::with_capacity(rules.len());
    let mut reads: Vec<Vec<String>> = Vec::with_capacity(rules.len());
    for rule in rules {
        let mut read_set = RuleEngine::read_fields(&rule.condition)?;
        let mut write_set: Vec<String> = Vec::new();
        if !rule.action.trim().is_empty() {
            for (lhs, rhs) in RuleEngine::action_assignments(&rule.action)? {
                write_set.push(lhs);
                read_set.extend(RuleEngine::read_fields(&rhs)?);
            }
        }
        write_set.sort();
        write_set.dedup();
        read_set.sort();
        read_set.dedup();
        writes.push(write_set);
        reads.push(read_set);
    }

    let mut edges: Vec<(usize, usize)> = Vec::new();
    for (i, w) in writes.iter().enumerate() {
        for (j, r) in reads.iter().enumerate() {
            if i != j && w.iter().any(|key| r.contains(key)) {
                edges.push((i, j));
            }
        }
    }
    Ok(edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(vars: &[(&str, Value)]) -> HashMap<String, Value> {
        vars.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_vip_discount_rule() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "vip_discount".to_string(),
            trigger: "onCreate".to_string(),
            condition: "customer_level == \"VIP\"".to_string(),
            action: "discount_rate = 0.15".to_string(),
            priority: 100,
            error_message: "VIP客户自动享受15%折扣".to_string(),
            blocking: false,
        }];

        let mut vars = ctx(&[
            ("customer_level", json!("VIP")),
            ("discount_rate", json!(0.0)),
        ]);

        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert_eq!(vars.get("discount_rate").unwrap(), &json!(0.15));
    }

    #[test]
    fn test_condition_not_met() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "vip_discount".to_string(),
            trigger: "onCreate".to_string(),
            condition: "customer_level == \"VIP\"".to_string(),
            action: "discount_rate = 0.15".to_string(),
            priority: 100,
            error_message: "VIP客户自动享受15%折扣".to_string(),
            blocking: false,
        }];

        let mut vars = ctx(&[
            ("customer_level", json!("NORMAL")),
            ("discount_rate", json!(0.0)),
        ]);

        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert!(!result.executions[0].triggered);
        assert_eq!(vars.get("discount_rate").unwrap(), &json!(0.0));
    }

    #[test]
    fn test_action_with_expression() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "calc_total".to_string(),
            trigger: "onCreate".to_string(),
            condition: "quantity > 0".to_string(),
            action: "total = quantity * price".to_string(),
            priority: 100,
            error_message: "计算总价失败".to_string(),
            blocking: false,
        }];

        let mut vars = ctx(&[
            ("quantity", json!(5)),
            ("price", json!(20.0)),
            ("total", json!(0)),
        ]);

        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert_eq!(vars.get("total").unwrap(), &json!(100.0));
    }

    #[test]
    fn test_blocking_rule_blocks_when_condition_met() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "block_test".to_string(),
            trigger: "onCreate".to_string(),
            condition: "code == \"test\"".to_string(),
            action: "".to_string(),
            priority: 100,
            error_message: "不允许使用 test 编码".to_string(),
            blocking: true,
        }];

        let mut vars = ctx(&[("code", json!("test"))]);
        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(!result.all_passed);
        assert!(result.executions[0].error.is_some());
        assert_eq!(
            result.executions[0].error.as_deref(),
            Some("不允许使用 test 编码")
        );
    }

    #[test]
    fn test_blocking_rule_passes_when_condition_not_met() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "block_test".to_string(),
            trigger: "onCreate".to_string(),
            condition: "code == \"test\"".to_string(),
            action: "".to_string(),
            priority: 100,
            error_message: "不允许使用 test 编码".to_string(),
            blocking: true,
        }];

        let mut vars = ctx(&[("code", json!("normal"))]);
        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert!(!result.executions[0].triggered);
    }

    #[test]
    fn test_non_blocking_rule_does_not_block_when_condition_met() {
        let rules = vec![BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: "set_default".to_string(),
            trigger: "onCreate".to_string(),
            condition: "status == ()".to_string(),
            action: "status = \"active\"".to_string(),
            priority: 100,
            error_message: "设置默认状态".to_string(),
            blocking: false,
        }];

        let mut vars = ctx(&[("status", json!(null))]);
        let result = RuleEngine::execute(&rules, &mut vars);
        assert!(result.all_passed);
        assert!(result.executions[0].triggered);
        assert!(result.executions[0].error.is_none());
        assert_eq!(vars.get("status").unwrap(), &json!("active"));
    }

    // ── 求解层（saturate）：依赖传播 / 不动点 / 上限留痕 / 冲突消解 / 多赋值 / 降级 ──

    fn mk_rule(name: &str, condition: &str, action: &str, priority: i32) -> BusinessRuleConfig {
        BusinessRuleConfig {
            entity: "Order".to_string(),
            rule_name: name.to_string(),
            trigger: "onCreate".to_string(),
            condition: condition.to_string(),
            action: action.to_string(),
            priority,
            error_message: format!("{name} failed"),
            blocking: false,
        }
    }

    #[test]
    fn saturate_propagates_dependencies_across_rounds() {
        // B 读 x（priority 小 ⇒ 先跑），A 写 x（后跑）：单轮顺序下 B 永不触发 ⇒ 只有迭代才可传播
        let rules = vec![
            mk_rule("b_reads_x", "x == 1", "y = x + 1", 100),
            mk_rule("a_writes_x", "true", "x = 1", 200),
        ];
        let mut single_vars = ctx(&[("x", json!(0))]);
        let single = RuleEngine::execute(&rules, &mut single_vars);
        assert!(!single.saturated, "单轮入口 MUST NOT 声称做过依赖传播");
        assert!(
            !single.mutations.contains_key("y"),
            "单轮不传播（B 先跑且条件不成立）：{:?}",
            single.mutations
        );

        let mut vars = ctx(&[("x", json!(0))]);
        let res = RuleEngine::saturate(&rules, &mut vars, &SaturationConfig::default());
        assert!(res.all_passed, "{:?}", res.errors);
        assert!(res.saturated, "应达不动点");
        assert!(res.degraded.is_none());
        assert_eq!(vars.get("x").unwrap(), &json!(1));
        assert_eq!(vars.get("y").unwrap(), &json!(2), "链式传播应产生 y");
        assert!(res.rounds >= 2, "rounds={}", res.rounds);
    }

    #[test]
    fn saturate_stops_at_fixpoint_on_idempotent_write() {
        let rules = vec![mk_rule("set_x", "true", "x = 1", 100)];
        let mut vars = ctx(&[("x", json!(1))]);
        let res = RuleEngine::saturate(&rules, &mut vars, &SaturationConfig::default());
        assert!(res.saturated);
        assert_eq!(res.rounds, 1, "值未变化 ⇒ 一轮即不动点");
        assert!(res.conflicts.is_empty());
    }

    #[test]
    fn saturate_records_remaining_triggers_at_cap() {
        let rules = vec![mk_rule("inc_x", "x < 100", "x = x + 1", 100)];
        let mut vars = ctx(&[("x", json!(0))]);
        let cfg = SaturationConfig {
            max_rounds: 3,
            on_conflict: ConflictPolicy::FirstWins,
        };
        let res = RuleEngine::saturate(&rules, &mut vars, &cfg);
        assert!(!res.saturated, "达上限 MUST 标记未饱和");
        assert_eq!(res.rounds, 3);
        assert_eq!(res.remaining_triggers, vec!["inc_x".to_string()]);
        assert_eq!(vars.get("x").unwrap(), &json!(3));
    }

    #[test]
    fn saturate_conflict_policies_are_deterministic() {
        let rules = vec![
            mk_rule("first", "true", "x = 1", 100),
            mk_rule("second", "true", "x = 2", 200),
        ];
        let run = |policy: ConflictPolicy| {
            let mut vars = ctx(&[("x", json!(0))]);
            let res = RuleEngine::saturate(
                &rules,
                &mut vars,
                &SaturationConfig {
                    max_rounds: 4,
                    on_conflict: policy,
                },
            );
            (vars.get("x").unwrap().clone(), res)
        };
        let (v1, r1) = run(ConflictPolicy::FirstWins);
        let (v2, _) = run(ConflictPolicy::FirstWins);
        assert_eq!(v1, json!(1), "先写者胜（priority 升序）");
        assert_eq!(v1, v2, "同输入多次运行 MUST 一致（确定性）");
        assert_eq!(r1.conflicts.len(), 1);
        assert_eq!(r1.conflicts[0].winner, "first");
        assert_eq!(r1.conflicts[0].loser, "second");
        let (v3, _) = run(ConflictPolicy::LastWins);
        assert_eq!(v3, json!(2));
        let (_, r4) = run(ConflictPolicy::Error);
        assert!(!r4.all_passed, "Error 策略 MUST 阻断");
        assert!(
            r4.errors.iter().any(|e| e.contains("规则冲突")),
            "{:?}",
            r4.errors
        );
    }

    #[test]
    fn multi_assignment_action_executes_sequentially() {
        let rules = vec![mk_rule("two_writes", "true", "a = 1; b = a + 1", 100)];
        let mut vars = ctx(&[]);
        let res = RuleEngine::saturate(&rules, &mut vars, &SaturationConfig::default());
        assert!(res.all_passed, "{:?}", res.errors);
        assert_eq!(vars.get("a").unwrap(), &json!(1));
        assert_eq!(vars.get("b").unwrap(), &json!(2), "后者 MUST 可见前者结果");
    }

    #[test]
    fn unparseable_condition_degrades_with_reason() {
        let rules = vec![mk_rule("broken", "(((", "x = 1", 100)];
        let mut vars = ctx(&[]);
        let res = RuleEngine::saturate(&rules, &mut vars, &SaturationConfig::default());
        assert!(!res.saturated);
        let reason = res.degraded.expect("降级 MUST 留痕");
        assert!(reason.contains("不可 AST 提取"), "{reason}");
    }

    #[test]
    fn blocking_rules_keep_semantics_under_saturation() {
        let rules = vec![
            BusinessRuleConfig {
                entity: "Order".to_string(),
                rule_name: "block_test".to_string(),
                trigger: "onCreate".to_string(),
                condition: "code == \"test\"".to_string(),
                action: String::new(),
                priority: 100,
                error_message: "不允许 test".to_string(),
                blocking: true,
            },
            mk_rule("set_flag", "true", "flag = 1", 200),
        ];
        let mut vars = ctx(&[("code", json!("test"))]);
        let res = RuleEngine::saturate(&rules, &mut vars, &SaturationConfig::default());
        assert!(!res.all_passed);
        assert!(res.errors.iter().any(|e| e == "不允许 test"));
        assert!(res.saturated, "屏蔽错误不阻塞收敛（自身无写入）");
        assert_eq!(vars.get("flag").unwrap(), &json!(1));
    }

    #[test]
    fn dependency_edges_detect_cross_rule_write_read() {
        let rules = vec![
            mk_rule("write_total", "true", "total = 7", 100),
            mk_rule("read_total", "total > 5", "flag = 1", 200),
        ];
        let edges = rule_dependency_edges(&rules).expect("依赖边可提取");
        assert_eq!(edges, vec![(0, 1)], "写 total 者 → 读 total 者");
    }

    #[test]
    fn dependency_edges_include_action_rhs_reads() {
        // 读者只经 action 右值引用写者字段（condition 无关）⇒ 仍是依赖边。
        let rules = vec![
            mk_rule("write_base", "true", "base = 3", 100),
            mk_rule("derive", "true", "derived = base * 2", 200),
        ];
        assert_eq!(rule_dependency_edges(&rules).unwrap(), vec![(0, 1)]);
    }

    #[test]
    fn dependency_edges_empty_for_independent_rules() {
        let rules = vec![
            mk_rule("a", "true", "x = 1", 100),
            mk_rule("b", "true", "y = 2", 200),
        ];
        assert!(
            rule_dependency_edges(&rules).unwrap().is_empty(),
            "写读不相交 ⇒ 无依赖（单轮执行即可）"
        );
    }

    #[test]
    fn dependency_edges_exclude_self_loops() {
        let rules = vec![mk_rule("incr", "true", "x = x + 1", 100)];
        assert!(
            rule_dependency_edges(&rules).unwrap().is_empty(),
            "自写自读非跨规则依赖"
        );
    }

    #[test]
    fn dependency_edges_fail_closed_on_unparseable_expression() {
        let rules = vec![mk_rule("broken", "(((", "x = 1", 100)];
        assert!(
            rule_dependency_edges(&rules).is_err(),
            "不可 AST 提取 MUST 报错，不得文本猜测语义"
        );
    }
}
