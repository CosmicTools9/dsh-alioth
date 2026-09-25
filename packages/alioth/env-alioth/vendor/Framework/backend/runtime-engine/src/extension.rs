//! 应用级逻辑扩展运行时注册表
//!
//! 提供 `AppExtensionRegistry`：按应用代码隔离的扩展配置注册表，
//! 支持在实体 CRUD 生命周期中注入约束验证、业务规则、工作流等逻辑。
//!
//! # 使用方式
//!
//! ```rust,ignore
//! use runtime_engine::AppExtensionRegistry;
//! use runtime_contract::AppLogicExtension;
//!
//! let registry = AppExtensionRegistry::new();
//! registry.register(app_logic_extension);
//!
//! // 在 CRUD handler 中调用
//! let result = registry.before_create("oms", "Order", &variables)?;
//! if !result.all_passed {
//!     return Err(ApiError::ValidationFailed(result.blocking_errors));
//! }
//! ```

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use runtime_contract::behavior::LifecycleEvent;
use runtime_contract::extension::*;

use crate::engine::constraint::{ConstraintEngine, ConstraintValidationResult};
use crate::engine::rule::{RuleEngine, RuleExecutionResult};

/// 求解留痕（**非致命**可读条目）——规则集执行后由调用方（CRUD 流）告警或落审计。
///
/// 判据：降级（退回单轮）/ 未达不动点（留残余触发）/ 冲突裁决 —— 三者都必须可见
/// （`MUST NOT` 静默：链式依赖未收敛等同「规则没生效」，但不阻断主操作）。
/// 单轮语义（未声明 `saturate`）恒返回空：`saturated=false` 且 `rounds=0` 不构成告警。
pub fn solve_warnings(result: &RuleExecutionResult) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(reason) = &result.degraded {
        out.push(format!("规则求解降级（退回单轮顺序执行）：{reason}"));
    }
    if !result.saturated && result.rounds > 0 {
        out.push(format!(
            "规则集未达不动点（已 {} 轮，待处理规则：{:?}）",
            result.rounds, result.remaining_triggers
        ));
    }
    for c in &result.conflicts {
        out.push(format!(
            "规则冲突：字段 {} 由 {} 覆盖 {}（策略 {:?}）",
            c.field, c.winner, c.loser, c.policy
        ));
    }
    out
}

/// 扩展声明的**表达式**校验（上下文无关层：语法 + 自由函数白名单）。
///
/// 返回违规条目（`<面>: <label> = <expr> → <err>`；空 = 全部通过）。
/// **策略不入本函数**：调用方决定阻断（组装/验证侧 fail-closed）还是记错继续
/// （Gateway 加载期现行策略「违规 ERROR、不阻断启动」，运行期可用性优先）。
///
/// 判据面 = 四类扩展声明的全部表达式承载字段：`constraints.expression` /
/// `business_rules.condition` 与 `action` 的**各 RHS 段**（顶层 `;` 分段；LHS 是赋值目标，
/// 不入判据）/ `state_machines.transitions[].guard` 与 `transitions[].action`
/// （action 会被 `on_transition` 执行——不校验即加载期静默、运行期炸）/
/// `workflows[].trigger.condition` 与 `steps[].condition` 与步骤动作的表达式承载字段
/// （`SetField.expression` / `CreateRelated.field_map` 各值 / `CallProcedure.params` 各元素）。
///
/// 标识符层（引用 ⊆ 实体字段集）需实体字段集，加载期不可得 ⇒ 组装期由
/// `Meta/backend/app-agent/src/domain_keys.rs`（plan entity ↔ 本体 domain 映射）承担。
/// 单个表达式承载字段的校验：空 ⇒ 跳过；语法/白名单不过 ⇒ 记违规
/// （自由函数而非闭包：调用点需要同时向同一 `out` 追加分段级违规，闭包会与之争借用）
fn check_expr(
    engine: &crate::expression::RhaiExpressionEngine,
    out: &mut Vec<String>,
    label: String,
    expr: &str,
) {
    if expr.trim().is_empty() {
        return;
    }
    if let Err(e) = engine.validate_functions(expr) {
        out.push(format!("{label} = {expr} → {e}"));
    }
}

pub fn collect_expression_violations(ext: &AppLogicExtension) -> Vec<String> {
    let engine = crate::expression::RhaiExpressionEngine::new();
    let mut out: Vec<String> = Vec::new();

    for (i, c) in ext.constraints.iter().enumerate() {
        check_expr(
            &engine,
            &mut out,
            format!(
                "constraints[{i}]({}.{})",
                c.entity,
                c.field.as_deref().unwrap_or("*")
            ),
            &c.expression,
        );
    }
    for (i, r) in ext.business_rules.iter().enumerate() {
        check_expr(
            &engine,
            &mut out,
            format!("business_rules[{i}]({}).condition", r.name),
            &r.condition,
        );
        // 分段 MUST 走引擎同一实现（`action_assignments`）：段缺 `=`（如内嵌 `let …; x.y()` 语句序列）
        // 在运行期是 `Unsupported action format` ⇒ 加载期 MUST 同样报违规（否则「加载期静默、运行期炸」）
        match RuleEngine::action_assignments(&r.action) {
            Err(e) => out.push(format!("business_rules[{i}]({}).action → {e}", r.name)),
            Ok(assignments) => {
                for (j, (_lhs, rhs)) in assignments.iter().enumerate() {
                    check_expr(
                        &engine,
                        &mut out,
                        format!("business_rules[{i}]({}).action[{j}]", r.name),
                        rhs,
                    );
                }
            }
        }
    }
    for (i, sm) in ext.state_machines.iter().enumerate() {
        for (j, t) in sm.transitions.iter().enumerate() {
            if let Some(g) = &t.guard {
                check_expr(
                    &engine,
                    &mut out,
                    format!("state_machines[{i}]({}).transitions[{j}].guard", sm.entity),
                    g,
                );
            }
            if let Some(a) = &t.action {
                // 与运行期 `on_transition` / `before_update` 的动作执行同分段口径
                match RuleEngine::action_assignments(a) {
                    Err(e) => out.push(format!(
                        "state_machines[{i}]({}).transitions[{j}].action → {e}",
                        sm.entity
                    )),
                    Ok(assignments) => {
                        for (k, (_lhs, rhs)) in assignments.iter().enumerate() {
                            check_expr(
                                &engine,
                                &mut out,
                                format!(
                                    "state_machines[{i}]({}).transitions[{j}].action[{k}]",
                                    sm.entity
                                ),
                                rhs,
                            );
                        }
                    }
                }
            }
        }
    }
    for (i, wf) in ext.workflows.iter().enumerate() {
        if let Some(c) = &wf.trigger.condition {
            check_expr(
                &engine,
                &mut out,
                format!("workflows[{i}]({}).trigger.condition", wf.name),
                c,
            );
        }
        for (j, step) in wf.steps.iter().enumerate() {
            if let Some(c) = &step.condition {
                check_expr(
                    &engine,
                    &mut out,
                    format!("workflows[{i}]({}).steps[{j}].condition", wf.name),
                    c,
                );
            }
            match &step.action {
                WorkflowAction::SetField { expression, .. } => {
                    check_expr(
                        &engine,
                        &mut out,
                        format!("workflows[{i}]({}).steps[{j}].action", wf.name),
                        expression,
                    );
                }
                WorkflowAction::CreateRelated { field_map, .. } => {
                    for (target_field, source_expr) in field_map {
                        check_expr(
                            &engine,
                            &mut out,
                            format!(
                                "workflows[{i}]({}).steps[{j}].action.field_map[{target_field}]",
                                wf.name
                            ),
                            source_expr,
                        );
                    }
                }
                WorkflowAction::CallProcedure { params, .. } => {
                    for (k, param) in params.iter().enumerate() {
                        check_expr(
                            &engine,
                            &mut out,
                            format!("workflows[{i}]({}).steps[{j}].action.params[{k}]", wf.name),
                            param,
                        );
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// 表达式面**非致命提示**（advisories）——**单一实现**，供启动期（Gateway WARN）与
/// 验证期（`verify-extensions` note）共同消费；`MUST NOT` 阻断任何流程。
///
/// 判据面：`state_machines[].transitions[].guard` 为**裸标识符**（单标识符表达式，无运算符/
/// 字面量/调用）。该形态按 Rhai 变量读取求值，语义上**依赖调用方注入同名变量**——历史
/// 「布尔字段引用」短路通道已删除（2026-09-22 单一文法裁定）；若该变量从不注入，guard 恒抛
/// `Variable not found` ⇒ 该迁移被 fail-closed **永久阻断**（实测
/// `Pre-Proc/AVIC-CAASEC/Apps/ai-98ab565fc9610cfd/extensions/statemachines.yaml` 两支即此形态，
/// 而该 app 无 flow-plan ⇒ 验证期整体跳过 ⇒ 加载期是该缺陷唯一的机械可见面）。
pub fn collect_expression_advisories(ext: &AppLogicExtension) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for sm in &ext.state_machines {
        for (i, t) in sm.transitions.iter().enumerate() {
            let Some(guard) = t.guard.as_deref().map(str::trim).filter(|g| !g.is_empty()) else {
                continue;
            };
            if is_bare_identifier(guard) {
                out.push(format!(
                    "state_machines[{}].transitions[{i}] ({}: {}→{}) guard 为裸标识符 `{guard}`：\
                     按 Rhai 变量读取求值，依赖调用方注入同名变量；未注入则恒 fail-closed 阻断该迁移",
                    sm.entity,
                    t.event,
                    t.from.join("|"),
                    t.to
                ));
            }
        }
    }
    out
}

/// 是否为「单标识符」串（`[A-Za-z_][A-Za-z0-9_]*`，无运算符/字面量/访问/调用）
fn is_bare_identifier(expr: &str) -> bool {
    let mut chars = expr.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ─────────────────────────────────────────────────────────────
// Errors
// ─────────────────────────────────────────────────────────────

/// 扩展运行时错误
#[derive(Debug, Clone)]
pub enum ExtensionRuntimeError {
    ProfileNotFound(String),
    EvaluationFailed(String),
    ConstraintViolated(Vec<String>),
    RuleExecutionFailed(Vec<String>),
}

impl std::fmt::Display for ExtensionRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionRuntimeError::ProfileNotFound(app) => {
                write!(f, "Extension profile not found for app: {}", app)
            }
            ExtensionRuntimeError::EvaluationFailed(msg) => {
                write!(f, "Extension evaluation failed: {}", msg)
            }
            ExtensionRuntimeError::ConstraintViolated(errors) => {
                write!(f, "Constraint violations: {:?}", errors)
            }
            ExtensionRuntimeError::RuleExecutionFailed(errors) => {
                write!(f, "Rule execution failed: {:?}", errors)
            }
        }
    }
}

impl std::error::Error for ExtensionRuntimeError {}

// ─────────────────────────────────────────────────────────────
// AppExtensionRegistry
// ─────────────────────────────────────────────────────────────

/// 应用级逻辑扩展注册表
///
/// 线程安全的单例注册表，按 `app_code` 隔离存储各应用的扩展配置。
/// 在 Gateway `register_apps()` 时初始化并注册所有应用的扩展。
#[derive(Debug, Clone)]
pub struct AppExtensionRegistry {
    profiles: Arc<RwLock<HashMap<String, AppLogicExtension>>>,
}

/// 扩展声明种类（实体解析与声明过滤用）
#[derive(Clone, Copy, PartialEq, Eq)]
enum DeclKind {
    Constraint,
    Rule,
    StateMachine,
    Workflow,
}

/// 该 profile 是否在指定种类下声明了该实体（声明名 = 实体键，字面相等）。
fn profile_declares(profile: &AppLogicExtension, entity: &str, kind: DeclKind) -> bool {
    match kind {
        DeclKind::Constraint => profile.constraints.iter().any(|c| c.entity == entity),
        DeclKind::Rule => profile.business_rules.iter().any(|r| r.entity == entity),
        DeclKind::StateMachine => profile.state_machines.iter().any(|s| s.entity == entity),
        DeclKind::Workflow => profile.workflows.iter().any(|w| w.trigger.entity == entity),
    }
}

impl AppExtensionRegistry {
    /// 创建空的注册表
    pub fn new() -> Self {
        Self {
            profiles: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 注册应用的扩展配置
    pub fn register(&self, profile: AppLogicExtension) {
        let mut profiles = self.profiles.write().unwrap();
        profiles.insert(profile.app_code.clone(), profile);
    }

    /// 取消注册应用扩展
    pub fn unregister(&self, app_code: &str) {
        let mut profiles = self.profiles.write().unwrap();
        profiles.remove(app_code);
    }

    /// 获取应用的扩展配置
    pub fn get_profile(&self, app_code: &str) -> Option<AppLogicExtension> {
        self.profiles.read().unwrap().get(app_code).cloned()
    }

    /// 检查应用是否有扩展配置
    pub fn has_profile(&self, app_code: &str) -> bool {
        self.profiles.read().unwrap().contains_key(app_code)
    }

    /// 获取所有已注册的应用代码
    pub fn registered_apps(&self) -> Vec<String> {
        self.profiles.read().unwrap().keys().cloned().collect()
    }

    /// 清空所有注册表
    pub fn clear(&self) {
        self.profiles.write().unwrap().clear();
    }

    // =============================================================
    // 约束验证
    // =============================================================

    /// 实体解析：`app_code` 命中该实体的声明 ⇒ 用它（精确优先）；否则按实体名跨注册表解析。
    ///
    /// **为什么需要兜底**：`app_code`（Gateway 作用域码 / ns server 的 `AppContext`）与注册键
    /// （扩展目录名，如 `wz-yy-wms`）**不是同一命名空间**——单进程服务同一 namespace 内多个 app 时
    /// 无法用单一 `app_code` 表达；「空 app_code」= 调用方明确不要精确提示。
    /// **实体名是唯一跨 app 稳定的解析键**（CRUD handler 以 `ENTITY_NAME ∪ 物理表名` 两种口径逐个尝试）。
    /// 多 app 声明同一实体 ⇒ ERROR 留痕（MUST NOT 静默择一），按 app_code 稳定序取首个。
    fn resolve_app(&self, app_code: &str, entity: &str, kind: DeclKind) -> Option<String> {
        let profiles = self.profiles.read().unwrap();
        if let Some(profile) = profiles.get(app_code) {
            if profile_declares(profile, entity, kind) {
                return Some(app_code.to_string());
            }
        }
        let mut hits: Vec<&String> = profiles
            .iter()
            .filter(|(_, p)| profile_declares(p, entity, kind))
            .map(|(code, _)| code)
            .collect();
        hits.sort();
        match hits.len() {
            0 => None,
            1 => Some(hits[0].clone()),
            n => {
                log::error!(
                    "[extensions] 实体 '{entity}' 被 {n} 个 app 声明（{hits:?}）——取稳定序首个；请收敛声明面或提供精确 app_code"
                );
                Some(hits[0].clone())
            }
        }
    }

    /// 获取指定应用、指定实体的所有约束
    pub fn get_constraints(&self, app_code: &str, entity: &str) -> Vec<ConstraintExtension> {
        let Some(resolved) = self.resolve_app(app_code, entity, DeclKind::Constraint) else {
            return Vec::new();
        };
        self.profiles
            .read()
            .unwrap()
            .get(&resolved)
            .map(|p| {
                p.constraints
                    .iter()
                    .filter(|c| c.entity == entity)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 验证指定应用、指定实体的所有约束
    pub fn validate_constraints(
        &self,
        app_code: &str,
        entity: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<ConstraintValidationResult, ExtensionRuntimeError> {
        let constraints = self.get_constraints(app_code, entity);
        if constraints.is_empty() {
            return Ok(ConstraintValidationResult::new());
        }

        let engine_constraints: Vec<crate::engine::constraint::ConstraintConfig> = constraints
            .into_iter()
            .map(|c| crate::engine::constraint::ConstraintConfig {
                entity: c.entity,
                field: c.field,
                expression: c.expression,
                level: match c.level {
                    ConstraintSeverity::Error => "error".to_string(),
                    ConstraintSeverity::Warning => "warning".to_string(),
                },
                message: c.message,
            })
            .collect();

        Ok(ConstraintEngine::validate(&engine_constraints, variables))
    }

    // =============================================================
    // 状态机
    // =============================================================

    /// 获取指定应用、指定实体的状态机定义（第一个匹配）
    pub fn get_state_machine(&self, app_code: &str, entity: &str) -> Option<StateMachineExtension> {
        let resolved = self.resolve_app(app_code, entity, DeclKind::StateMachine)?;
        self.profiles.read().unwrap().get(&resolved).and_then(|p| {
            p.state_machines
                .iter()
                .find(|sm| sm.entity == entity)
                .cloned()
        })
    }

    // =============================================================
    // 业务规则
    // =============================================================

    /// 获取指定应用、指定实体、指定触发器的所有规则
    ///
    /// `trigger` 匹配为**大小写不敏感**：`extensions/rules.yaml` 同时存在 `onCreate` 与 `OnCreate`
    /// 两种历史口径（2026-09-23 普查：全仓 5 条声明中 3 条为大写驼峰），字面相等会让大写口径的规则
    /// 永不执行（声明了却无运行期影响）。两种口径 MUST 同时可解析——与实体键双口径同一判据
    /// （MUST NOT 以「改 YAML 迁就运行时」的单向收口替代）。
    pub fn get_rules(&self, app_code: &str, entity: &str, trigger: &str) -> Vec<RuleExtension> {
        let Some(resolved) = self.resolve_app(app_code, entity, DeclKind::Rule) else {
            return Vec::new();
        };
        self.profiles
            .read()
            .unwrap()
            .get(&resolved)
            .map(|p| {
                p.business_rules
                    .iter()
                    .filter(|r| r.entity == entity && r.trigger.eq_ignore_ascii_case(trigger))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 执行指定应用、指定实体、指定触发器的所有规则
    ///
    /// 求解语义由应用 `extensions/rule_execution.yaml`（`AppLogicExtension::rule_execution`）裁定：
    /// - 未声明 `saturate` ⇒ **单轮顺序执行**（`RuleEngine::execute`；历史语义逐字不变，`saturated = false`）；
    /// - `saturate: true` ⇒ **依赖驱动不动点求解**（`RuleEngine::saturate`）：跨规则链式传播，
    ///   达上限 / 降级 / 冲突裁决均随结果留痕（见 [`solve_warnings`]）。
    pub fn execute_rules(
        &self,
        app_code: &str,
        entity: &str,
        trigger: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<RuleExecutionResult, ExtensionRuntimeError> {
        let rules = self.get_rules(app_code, entity, trigger);
        if rules.is_empty() {
            return Ok(RuleExecutionResult::new());
        }

        let saturation = self
            .profiles
            .read()
            .unwrap()
            .get(app_code)
            .map(|p| p.rule_execution.to_saturation())
            .unwrap_or(None);

        let engine_rules: Vec<crate::engine::rule::BusinessRuleConfig> = rules
            .into_iter()
            .map(|r| crate::engine::rule::BusinessRuleConfig {
                entity: r.entity,
                rule_name: r.name,
                trigger: r.trigger,
                condition: r.condition,
                action: r.action,
                priority: r.priority,
                error_message: r.error_message,
                blocking: r.blocking,
            })
            .collect();

        let result = match saturation {
            Some(cfg) => RuleEngine::saturate(
                &engine_rules,
                variables,
                &crate::engine::rule::SaturationConfig::from_extension(cfg),
            ),
            None => RuleEngine::execute(&engine_rules, variables),
        };

        // 求解留痕 MUST 可见（不饱和/降级/冲突 = 规则未完全生效，但不阻断主操作）
        for w in solve_warnings(&result) {
            log::warn!("[rules] app '{app_code}' {entity}.{trigger}: {w}");
        }
        Ok(result)
    }

    // =============================================================
    // 生命周期钩子 — 统一入口
    // =============================================================

    /// 约束结果 → 评估项：**仅 `error` 级**未通过计入阻断；`warning` 级未通过**留痕但不阻断写径**。
    ///
    /// 历史缺陷（2026-09-23 修正）：任何未通过都 `result.add(passed: false)` ⇒ `all_passed = false`
    /// ⇒ 写径被 warning 级约束阻断（`level` 语义在结果面丢失）。
    fn record_constraint_outcome(
        result: &mut ExtensionResult,
        constraint_result: &ConstraintValidationResult,
    ) {
        for cr in &constraint_result.results {
            if !cr.passed && cr.level != "error" {
                log::warn!(
                    "约束未通过（{} 级，不阻断写径）：{}.{} — {}",
                    cr.level,
                    cr.entity,
                    cr.field.as_deref().unwrap_or("*"),
                    cr.message
                );
                continue;
            }
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::Constraint,
                name: format!("{}.{}", cr.entity, cr.field.as_deref().unwrap_or("*")),
                passed: cr.passed,
                error: if cr.passed {
                    None
                } else {
                    Some(cr.message.clone())
                },
                mutations: HashMap::new(),
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }
    }

    /// 实体创建前执行所有扩展逻辑
    ///
    /// 执行顺序：
    /// 1. 约束验证（失败则返回错误，阻止创建）
    /// 2. 业务规则（onCreate 触发，可能修改字段值）
    ///
    /// 返回的 `mutations` 应被应用到创建请求中。
    pub fn before_create(
        &self,
        app_code: &str,
        entity: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        // 1. 约束验证
        let constraint_result = self.validate_constraints(app_code, entity, variables)?;
        Self::record_constraint_outcome(&mut result, &constraint_result);

        if !constraint_result.is_valid {
            // 如果有 error 级别的约束失败，直接返回
            let has_errors = constraint_result
                .results
                .iter()
                .any(|r| !r.passed && r.level == "error");
            if has_errors {
                return Ok(result);
            }
        }

        // 2. 状态机验证：若 state_field 在变量中且已定义状态机，验证初始状态
        if let Some(sm) = self.get_state_machine(app_code, entity) {
            if let Some(state_val) = variables.get(&sm.state_field) {
                if let Some(state_str) = state_val.as_str() {
                    if let crate::engine::state_machine::StateMachineResult::Failed(msg) =
                        crate::engine::state_machine::StateMachineEngine::validate_initial_state(
                            &sm.states, state_str,
                        )
                    {
                        result.add(ExtensionEvaluation {
                            extension_type: ExtensionType::StateMachine,
                            name: format!("{}.initial_state", entity),
                            passed: false,
                            error: Some(msg),
                            mutations: HashMap::new(),
                            evaluated_at: chrono::Utc::now().to_rfc3339(),
                        });
                        return Ok(result);
                    }
                }
            }
        }

        // 3. 业务规则（onCreate）
        let rule_result = self.execute_rules(app_code, entity, "onCreate", variables)?;
        for exec in &rule_result.executions {
            let mut mutations = HashMap::new();
            for (field, value) in &exec.mutations {
                mutations.insert(field.clone(), value.clone());
            }
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::BusinessRule,
                name: exec.rule_name.clone(),
                passed: exec.error.is_none(),
                error: exec.error.clone(),
                mutations,
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        Ok(result)
    }

    /// 实体创建后执行扩展逻辑
    ///
    /// 主要用于触发工作流、SWRL 推理等后置逻辑。
    pub fn after_create(
        &self,
        app_code: &str,
        entity: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        // 执行业务规则（post-create，trigger = "afterCreate" 或 "always"）
        let mut vars = variables.clone();
        let rule_result = self.execute_rules(app_code, entity, "afterCreate", &mut vars)?;
        if rule_result.executions.is_empty() {
            let rule_result = self.execute_rules(app_code, entity, "always", &mut vars)?;
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        } else {
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }

        // 执行工作流（实体解析：取声明该实体工作流的 app）
        if let Some(profile) = self
            .resolve_app(app_code, entity, DeclKind::Workflow)
            .and_then(|code| self.get_profile(&code))
        {
            for workflow in &profile.workflows {
                if workflow.trigger.entity == entity
                    && workflow.trigger.event == LifecycleEvent::OnCreate
                {
                    // 检查触发条件
                    if let Some(ref condition) = workflow.trigger.condition {
                        match crate::expression::RhaiExpressionEngine::new()
                            .evaluate(condition, variables)
                        {
                            Ok(value) => {
                                if !value.as_bool().unwrap_or(false) {
                                    continue;
                                }
                            }
                            Err(e) => {
                                result.add(ExtensionEvaluation {
                                    extension_type: ExtensionType::Workflow,
                                    name: workflow.name.clone(),
                                    passed: false,
                                    error: Some(format!(
                                        "Workflow trigger evaluation failed: {}",
                                        e
                                    )),
                                    mutations: HashMap::new(),
                                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                                });
                                continue;
                            }
                        }
                    }

                    // 工作流执行成功（当前版本标记为通过，实际执行由外部调度）
                    result.add(ExtensionEvaluation {
                        extension_type: ExtensionType::Workflow,
                        name: workflow.name.clone(),
                        passed: true,
                        error: None,
                        mutations: HashMap::new(),
                        evaluated_at: chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
        }

        Ok(result)
    }

    /// 实体更新前执行扩展逻辑
    ///
    /// `current_variables` 是当前数据库中实体的状态（用于状态机转换验证等）。
    pub fn before_update(
        &self,
        app_code: &str,
        entity: &str,
        variables: &mut HashMap<String, Value>,
        current_variables: &HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        // 求值上下文 = **存量行 ⊕ 本次提交**（提交值覆盖）：语义 = 「写完成后的行状态」。
        // 只用提交面会让引用**未提交**字段的约束/规则/守卫因变量缺失而求值失败（求值失败按 level
        // 处理 ⇒ Error 级即阻断）⇒ 部分字段更新（PATCH 语义）被误 400。
        // 只填充缺省键（不覆盖提交值），且不产生 mutation ⇒ 未提交字段不会被回写。
        for (key, value) in current_variables {
            variables
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }

        // 约束验证
        let constraint_result = self.validate_constraints(app_code, entity, variables)?;
        Self::record_constraint_outcome(&mut result, &constraint_result);

        if !constraint_result.is_valid {
            let has_errors = constraint_result
                .results
                .iter()
                .any(|r| !r.passed && r.level == "error");
            if has_errors {
                return Ok(result);
            }
        }

        // 2. 状态机验证：检查状态转换是否合法
        if let Some(sm) = self.get_state_machine(app_code, entity) {
            let old_state = current_variables
                .get(&sm.state_field)
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let new_state = variables
                .get(&sm.state_field)
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if let (Some(from), Some(to)) = (old_state.as_deref(), new_state.as_deref()) {
                if from != to {
                    // 尝试用 from→to 匹配任意 event；若用户未指定 event 则自动推断
                    // （owned：后续 transition action 须可变借用 `variables` ⇒ 不可留借用自它的 &str）
                    let event = variables
                        .get("event")
                        .and_then(|v| v.as_str())
                        .unwrap_or("update")
                        .to_string();
                    // guard 求值上下文 = 上面的合并上下文（存量行 ⊕ 本次提交）的**快照**：
                    // guard 引用未随本次 update 提交的存量字段时仍可解析；快照使随后的 action 可写 `variables`
                    let guard_ctx: HashMap<String, Value> = variables.clone();
                    match crate::engine::state_machine::StateMachineEngine::find_transition(
                        &sm.transitions,
                        from,
                        to,
                        &event,
                        &guard_ctx,
                    ) {
                        Err(msg) => {
                            result.add(ExtensionEvaluation {
                                extension_type: ExtensionType::StateMachine,
                                name: format!("{}.transition.{}_{}", entity, from, to),
                                passed: false,
                                error: Some(msg),
                                mutations: HashMap::new(),
                                evaluated_at: chrono::Utc::now().to_rfc3339(),
                            });
                            return Ok(result);
                        }
                        Ok(transition) => {
                            // 转换通过 ⇒ 执行其 action（`字段 = 表达式`，与规则动作**同一执行器**）。
                            // 结果以 mutations 记录，由写径合并入线载荷后才能反序列化为 DTO。
                            // 求值失败 ⇒ fail-closed（声明了却求不出值的动作 = 声明与实现不一致，静默跳过不可接受）。
                            if let Some(action) = transition
                                .action
                                .as_deref()
                                .filter(|a| !a.trim().is_empty())
                            {
                                match RuleEngine::execute_action(action, variables) {
                                    Ok(assignments) => {
                                        let mutations: HashMap<String, Value> =
                                            assignments.into_iter().collect();
                                        result.add(ExtensionEvaluation {
                                            extension_type: ExtensionType::StateMachine,
                                            name: format!("{}.transition.{}_{}", entity, from, to),
                                            passed: true,
                                            error: None,
                                            mutations,
                                            evaluated_at: chrono::Utc::now().to_rfc3339(),
                                        });
                                    }
                                    Err(e) => {
                                        result.add(ExtensionEvaluation {
                                            extension_type: ExtensionType::StateMachine,
                                            name: format!("{}.transition.{}_{}", entity, from, to),
                                            passed: false,
                                            error: Some(format!("转换动作求值失败：{e}")),
                                            mutations: HashMap::new(),
                                            evaluated_at: chrono::Utc::now().to_rfc3339(),
                                        });
                                        return Ok(result);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. 业务规则（onUpdate）
        let rule_result = self.execute_rules(app_code, entity, "onUpdate", variables)?;
        for exec in &rule_result.executions {
            let mut mutations = HashMap::new();
            for (field, value) in &exec.mutations {
                mutations.insert(field.clone(), value.clone());
            }
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::BusinessRule,
                name: exec.rule_name.clone(),
                passed: exec.error.is_none(),
                error: exec.error.clone(),
                mutations,
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        Ok(result)
    }

    /// 实体更新后执行扩展逻辑
    pub fn after_update(
        &self,
        app_code: &str,
        entity: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        let mut vars = variables.clone();
        let rule_result = self.execute_rules(app_code, entity, "afterUpdate", &mut vars)?;
        if rule_result.executions.is_empty() {
            let rule_result = self.execute_rules(app_code, entity, "always", &mut vars)?;
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        } else {
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }

        // 工作流（实体解析：取声明该实体工作流的 app）
        if let Some(profile) = self
            .resolve_app(app_code, entity, DeclKind::Workflow)
            .and_then(|code| self.get_profile(&code))
        {
            for workflow in &profile.workflows {
                if workflow.trigger.entity == entity
                    && workflow.trigger.event == LifecycleEvent::OnUpdate
                {
                    if let Some(ref condition) = workflow.trigger.condition {
                        match crate::expression::RhaiExpressionEngine::new()
                            .evaluate(condition, variables)
                        {
                            Ok(value) => {
                                if !value.as_bool().unwrap_or(false) {
                                    continue;
                                }
                            }
                            Err(e) => {
                                result.add(ExtensionEvaluation {
                                    extension_type: ExtensionType::Workflow,
                                    name: workflow.name.clone(),
                                    passed: false,
                                    error: Some(format!(
                                        "Workflow trigger evaluation failed: {}",
                                        e
                                    )),
                                    mutations: HashMap::new(),
                                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                                });
                                continue;
                            }
                        }
                    }
                    result.add(ExtensionEvaluation {
                        extension_type: ExtensionType::Workflow,
                        name: workflow.name.clone(),
                        passed: true,
                        error: None,
                        mutations: HashMap::new(),
                        evaluated_at: chrono::Utc::now().to_rfc3339(),
                    });
                }
            }
        }

        Ok(result)
    }

    /// 实体删除前执行扩展逻辑
    pub fn before_delete(
        &self,
        app_code: &str,
        entity: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        // 约束验证
        let constraint_result = self.validate_constraints(app_code, entity, variables)?;
        Self::record_constraint_outcome(&mut result, &constraint_result);

        if !constraint_result.is_valid {
            let has_errors = constraint_result
                .results
                .iter()
                .any(|r| !r.passed && r.level == "error");
            if has_errors {
                return Ok(result);
            }
        }

        // 业务规则（onDelete）
        let rule_result = self.execute_rules(app_code, entity, "onDelete", variables)?;
        for exec in &rule_result.executions {
            let mut mutations = HashMap::new();
            for (field, value) in &exec.mutations {
                mutations.insert(field.clone(), value.clone());
            }
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::BusinessRule,
                name: exec.rule_name.clone(),
                passed: exec.error.is_none(),
                error: exec.error.clone(),
                mutations,
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        Ok(result)
    }

    /// 实体删除后执行扩展逻辑
    pub fn after_delete(
        &self,
        app_code: &str,
        entity: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        let mut vars = variables.clone();
        let rule_result = self.execute_rules(app_code, entity, "afterDelete", &mut vars)?;
        if rule_result.executions.is_empty() {
            let rule_result = self.execute_rules(app_code, entity, "always", &mut vars)?;
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        } else {
            for exec in &rule_result.executions {
                let mut mutations = HashMap::new();
                for (field, value) in &exec.mutations {
                    mutations.insert(field.clone(), value.clone());
                }
                result.add(ExtensionEvaluation {
                    extension_type: ExtensionType::BusinessRule,
                    name: exec.rule_name.clone(),
                    passed: exec.error.is_none(),
                    error: exec.error.clone(),
                    mutations,
                    evaluated_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }

        Ok(result)
    }

    /// 状态转换时执行扩展逻辑
    ///
    /// 在实体状态变更前调用，验证转换是否允许，执行守卫条件和动作。
    pub fn on_transition(
        &self,
        app_code: &str,
        entity: &str,
        from_state: &str,
        to_state: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        let mut result = ExtensionResult::new();

        if let Some(profile) = self.get_profile(app_code) {
            for sm in &profile.state_machines {
                if sm.entity == entity {
                    // 查找匹配的 transition
                    for transition in &sm.transitions {
                        if transition.can_transition_from(from_state) && transition.to == to_state {
                            // 检查 guard 条件
                            if let Some(ref guard) = transition.guard {
                                match crate::expression::RhaiExpressionEngine::new()
                                    .evaluate(guard, variables)
                                {
                                    Ok(value) => {
                                        if !value.as_bool().unwrap_or(false) {
                                            result.add(ExtensionEvaluation {
                                                extension_type: ExtensionType::StateMachine,
                                                name: format!(
                                                    "transition.{}.{}.{}.{}",
                                                    entity, from_state, to_state, transition.event
                                                ),
                                                passed: false,
                                                error: Some(format!(
                                                    "Transition guard failed: {}",
                                                    guard
                                                )),
                                                mutations: HashMap::new(),
                                                evaluated_at: chrono::Utc::now().to_rfc3339(),
                                            });
                                            return Ok(result);
                                        }
                                    }
                                    Err(e) => {
                                        result.add(ExtensionEvaluation {
                                            extension_type: ExtensionType::StateMachine,
                                            name: format!(
                                                "transition.{}.{}.{}.{}",
                                                entity, from_state, to_state, transition.event
                                            ),
                                            passed: false,
                                            error: Some(format!("Guard evaluation error: {}", e)),
                                            mutations: HashMap::new(),
                                            evaluated_at: chrono::Utc::now().to_rfc3339(),
                                        });
                                        return Ok(result);
                                    }
                                }
                            }

                            // 执行 transition action（`字段 = 表达式`，与本引擎规则动作**同一执行器**）
                            if let Some(action) = transition
                                .action
                                .as_deref()
                                .filter(|a| !a.trim().is_empty())
                            {
                                match RuleEngine::execute_action(action, variables) {
                                    Ok(assignments) => {
                                        for (field, value) in assignments {
                                            result = result.with_mutation(field, value);
                                        }
                                    }
                                    Err(e) => {
                                        result.add(ExtensionEvaluation {
                                            extension_type: ExtensionType::StateMachine,
                                            name: format!(
                                                "transition.{}.{}.{}.{}",
                                                entity, from_state, to_state, transition.event
                                            ),
                                            passed: false,
                                            error: Some(format!("转换动作求值失败：{e}")),
                                            mutations: HashMap::new(),
                                            evaluated_at: chrono::Utc::now().to_rfc3339(),
                                        });
                                        return Ok(result);
                                    }
                                }
                            }

                            result.add(ExtensionEvaluation {
                                extension_type: ExtensionType::StateMachine,
                                name: format!(
                                    "transition.{}.{}.{}.{}",
                                    entity, from_state, to_state, transition.event
                                ),
                                passed: true,
                                error: None,
                                mutations: HashMap::new(),
                                evaluated_at: chrono::Utc::now().to_rfc3339(),
                            });
                        }
                    }
                }
            }
        }

        // 执行业务规则（onTransition）
        let rule_result = self.execute_rules(app_code, entity, "onTransition", variables)?;
        for exec in &rule_result.executions {
            let mut mutations = HashMap::new();
            for (field, value) in &exec.mutations {
                mutations.insert(field.clone(), value.clone());
            }
            result.add(ExtensionEvaluation {
                extension_type: ExtensionType::BusinessRule,
                name: exec.rule_name.clone(),
                passed: exec.error.is_none(),
                error: exec.error.clone(),
                mutations,
                evaluated_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        Ok(result)
    }
}

impl Default for AppExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────
// RegistryLoader — 从文件系统加载扩展配置
// ─────────────────────────────────────────────────────────────

use std::path::Path;

/// 从文件系统加载应用扩展配置
pub struct ExtensionLoader;

impl ExtensionLoader {
    /// 加载单个应用的扩展配置目录
    ///
    /// 读取 `Pre-Proc/{namespace}/Apps/{app}/extensions/` 下的所有 YAML 文件，
    /// 合并为单个 `AppLogicExtension`。
    pub fn load_from_dir(
        app_code: &str,
        dir: impl AsRef<Path>,
    ) -> Result<AppLogicExtension, String> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(AppLogicExtension::new(app_code));
        }

        let mut extension = AppLogicExtension::new(app_code);

        // constraints.yaml
        let constraints_path = dir.join("constraints.yaml");
        if constraints_path.exists() {
            let content = std::fs::read_to_string(&constraints_path)
                .map_err(|e| format!("Failed to read constraints.yaml: {}", e))?;
            let constraints: Vec<ConstraintExtension> = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse constraints.yaml: {}", e))?;
            extension.constraints = constraints;
        }

        // rules.yaml
        let rules_path = dir.join("rules.yaml");
        if rules_path.exists() {
            let content = std::fs::read_to_string(&rules_path)
                .map_err(|e| format!("Failed to read rules.yaml: {}", e))?;
            let rules: Vec<RuleExtension> = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse rules.yaml: {}", e))?;
            extension.business_rules = rules;
        }

        // statemachines.yaml
        let sm_path = dir.join("statemachines.yaml");
        if sm_path.exists() {
            let content = std::fs::read_to_string(&sm_path)
                .map_err(|e| format!("Failed to read statemachines.yaml: {}", e))?;
            let state_machines: Vec<StateMachineExtension> = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse statemachines.yaml: {}", e))?;
            extension.state_machines = state_machines;
        }

        // workflows.yaml
        let wf_path = dir.join("workflows.yaml");
        if wf_path.exists() {
            let content = std::fs::read_to_string(&wf_path)
                .map_err(|e| format!("Failed to read workflows.yaml: {}", e))?;
            let workflows: Vec<WorkflowDefinition> = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse workflows.yaml: {}", e))?;
            extension.workflows = workflows;
        }

        // profiles.yaml — 领域模型配置单（叶表激活决策）
        let profiles_path = dir.join("profiles.yaml");
        if profiles_path.exists() {
            let content = std::fs::read_to_string(&profiles_path)
                .map_err(|e| format!("Failed to read profiles.yaml: {}", e))?;
            #[derive(serde::Deserialize)]
            struct ProfilesWrapper {
                #[serde(default)]
                profiles: std::collections::HashMap<String, runtime_contract::AppModelConfig>,
            }
            let wrapper: ProfilesWrapper = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse profiles.yaml: {}", e))?;
            extension.model_profiles = wrapper.profiles;
        }

        // rule_execution.yaml — 规则集求解语义（应用级）。缺省 ⇒ 默认（单轮顺序执行）。
        let rule_exec_path = dir.join("rule_execution.yaml");
        if rule_exec_path.exists() {
            let content = std::fs::read_to_string(&rule_exec_path)
                .map_err(|e| format!("Failed to read rule_execution.yaml: {}", e))?;
            let cfg: RuleExecutionConfig = yaml_serde::from_str(&content)
                .map_err(|e| format!("Failed to parse rule_execution.yaml: {}", e))?;
            extension.rule_execution = cfg;
        }

        Ok(extension)
    }

    /// 校验扩展配置的实体名和字段名是否合法。
    ///
    /// `known_entities`: entity_name → set of known field names
    ///
    /// 返回校验错误列表。若列表为空则全部通过。
    /// 校验失败不阻止扩展加载，但应记录 ERROR 级别日志。
    pub fn validate_entities(
        extension: &AppLogicExtension,
        known_entities: &HashMap<String, HashSet<String>>,
    ) -> Vec<String> {
        let mut errors = Vec::new();
        let app_code = &extension.app_code;

        for c in &extension.constraints {
            let ent = &c.entity;
            if !known_entities.contains_key(ent) {
                errors.push(format!(
                    "[{}] constraint entity '{}' not found in known entities: {:?}",
                    app_code,
                    ent,
                    known_entities.keys().collect::<Vec<_>>()
                ));
                continue;
            }
            if let Some(field) = &c.field {
                let known_fields = known_entities.get(ent).unwrap();
                if !known_fields.contains(field) {
                    errors.push(format!(
                        "[{}] constraint on '{}.{}': field '{}' not found in entity fields: {:?}",
                        app_code, ent, field, field, known_fields
                    ));
                }
            }
        }

        for r in &extension.business_rules {
            let ent = &r.entity;
            if !known_entities.contains_key(ent) {
                errors.push(format!(
                    "[{}] rule '{}' entity '{}' not found in known entities",
                    app_code, r.name, ent
                ));
            }
        }

        for sm in &extension.state_machines {
            let ent = &sm.entity;
            if !known_entities.contains_key(ent) {
                errors.push(format!(
                    "[{}] state machine entity '{}' not found in known entities",
                    app_code, ent
                ));
            }
        }

        // 工作流触发实体：运行期触发匹配为字面相等（`trigger.entity == entity`，与约束/规则/迁移
        // 共用同一实体实参名称空间）⇒ 写业务名等不可解析名称会导致工作流永不触发且无任何信号。
        for w in &extension.workflows {
            let ent = &w.trigger.entity;
            if !known_entities.contains_key(ent) {
                errors.push(format!(
                    "[{}] workflow '{}' trigger entity '{}' not found in known entities",
                    app_code, w.name, ent
                ));
            }
        }

        errors
    }
    /// 加载所有应用的扩展配置
    ///
    /// 扫描 `Pre-Proc/{namespace}/Apps/` 和 `Samples/` 目录下的所有应用。
    pub fn load_all(apps_dir: impl AsRef<Path>) -> Result<Vec<AppLogicExtension>, String> {
        let mut results = Vec::new();
        let apps_dir = apps_dir.as_ref();

        if let Ok(entries) = std::fs::read_dir(apps_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    let app_code = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if app_code.is_empty() {
                        continue;
                    }
                    let ext_dir = path.join("extensions");
                    let extension = Self::load_from_dir(app_code, ext_dir)?;
                    if !extension.is_empty() {
                        results.push(extension);
                    }
                }
            }
        }

        Ok(results)
    }
}
