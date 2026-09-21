//! 扩展声明执行面（单一装配路径）
//!
//! 为「对一份 `extensions/*.yaml` 做声明级执行」提供**唯一装配入口**：
//! 装配（内存扩展或目录加载）→ 引擎执行（create / update / transition）→ 声明清单（inventory）。
//!
//! # 职责边界
//!
//! - 本模块**只做装配、执行与声明枚举**；不做覆盖判定、不给通过性结论、不落产物——
//!   那些属 AppAgent 的 `verify_extensions`（见 openspec change
//!   `add-appagent-runtime-verify-eval-and-rollback`）。
//! - 本模块**不引入任何求值实现**：约束/规则/状态机一律经 `ConstraintEngine` /
//!   `RuleEngine` / `StateMachineEngine`（本 crate 既有引擎）。
//! - 实体/字段引用合法性复用 `ExtensionLoader::validate_entities`，不另写一份。
//!
//! # 单一装配路径（合同）
//!
//! 既有测试 `tests/extension_pipeline.rs` 与 AppAgent 的扩展验证 MUST 经本模块装配；
//! 禁止在调用方内联 `AppExtensionRegistry::new()` + `register()` 的第二套装配
//! （两份装配在同一 crate 内最不易察觉，却会让「测试通过」与「真实产物通过」脱钩）。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::extension::{AppExtensionRegistry, ExtensionLoader, ExtensionRuntimeError};
use crate::{AppLogicExtension, ConstraintValidationResult, ExtensionResult, RuleExecutionResult};

/// 扩展声明执行面：绑定单个 `app_code` 的装配 + 执行 + 声明清单入口
#[derive(Debug, Clone)]
pub struct ExtensionSurface {
    registry: AppExtensionRegistry,
    app_code: String,
}

impl ExtensionSurface {
    /// 由内存中的扩展配置装配（测试与派生用例构造用）
    pub fn from_extension(extension: AppLogicExtension) -> Self {
        let app_code = extension.app_code.clone();
        let registry = AppExtensionRegistry::new();
        registry.register(extension);
        Self { registry, app_code }
    }

    /// 由真实产物目录装配：`ExtensionLoader::load_from_dir`（目录不存在 → 空扩展，同既有语义）
    pub fn from_dir(app_code: impl Into<String>, dir: impl AsRef<Path>) -> Result<Self, String> {
        let app_code = app_code.into();
        let extension = ExtensionLoader::load_from_dir(&app_code, dir)?;
        Ok(Self::from_extension(extension))
    }

    /// 当前装配的 `app_code`
    pub fn app_code(&self) -> &str {
        &self.app_code
    }

    /// 当前装配的扩展配置（未装配 → None）
    pub fn profile(&self) -> Option<AppLogicExtension> {
        self.registry.get_profile(&self.app_code)
    }

    /// 创建前执行（约束 → 初始状态 → onCreate 规则）
    pub fn create(
        &self,
        entity: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        self.registry
            .before_create(&self.app_code, entity, variables)
    }

    /// 更新前执行（约束 → 状态转换 → onUpdate 规则）
    pub fn update(
        &self,
        entity: &str,
        variables: &mut HashMap<String, Value>,
        current_variables: &HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        self.registry
            .before_update(&self.app_code, entity, variables, current_variables)
    }

    /// 状态转换执行（验证 `from → to` 是否被声明允许并执行 guard / action）
    pub fn transition(
        &self,
        entity: &str,
        from_state: &str,
        to_state: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<ExtensionResult, ExtensionRuntimeError> {
        self.registry
            .on_transition(&self.app_code, entity, from_state, to_state, variables)
    }

    /// 实体 / 字段引用合法性（复用 `ExtensionLoader::validate_entities`，禁第二份）
    ///
    /// `known_entities`: entity_name → 该实体已知字段集合。
    pub fn validate_constraints(
        &self,
        entity: &str,
        variables: &HashMap<String, Value>,
    ) -> Result<ConstraintValidationResult, ExtensionRuntimeError> {
        self.registry
            .validate_constraints(&self.app_code, entity, variables)
    }

    pub fn execute_rules(
        &self,
        entity: &str,
        trigger: &str,
        variables: &mut HashMap<String, Value>,
    ) -> Result<RuleExecutionResult, ExtensionRuntimeError> {
        self.registry
            .execute_rules(&self.app_code, entity, trigger, variables)
    }

    pub fn validate_entity_references(
        &self,
        known_entities: &HashMap<String, HashSet<String>>,
    ) -> Vec<String> {
        match self.profile() {
            Some(ext) => ExtensionLoader::validate_entities(&ext, known_entities),
            None => Vec::new(),
        }
    }

    /// 声明清单：覆盖判定与用例派生的输入面
    pub fn inventory(&self) -> DeclarationInventory {
        let Some(ext) = self.profile() else {
            return DeclarationInventory::default();
        };

        let constraints = ext
            .constraints
            .iter()
            .enumerate()
            .map(|(i, c)| ConstraintDecl {
                id: format!(
                    "constraint[{i}]:{}.{}",
                    c.entity,
                    c.field.as_deref().unwrap_or("*")
                ),
                entity: c.entity.clone(),
                field: c.field.clone(),
                expression: c.expression.clone(),
                level: json_scalar_string(&c.level).to_lowercase(),
                message: c.message.clone(),
            })
            .collect();

        let rules = ext
            .business_rules
            .iter()
            .enumerate()
            .map(|(i, r)| RuleDecl {
                id: format!("rule[{i}]:{}.{}", r.entity, r.name),
                entity: r.entity.clone(),
                name: r.name.clone(),
                trigger: r.trigger.clone(),
                condition: r.condition.clone(),
                action: r.action.clone(),
                blocking: r.blocking,
            })
            .collect();

        let state_machines = ext
            .state_machines
            .iter()
            .map(|sm| StateMachineDecl {
                entity: sm.entity.clone(),
                state_field: sm.state_field.clone(),
                states: sm.states.iter().map(|s| s.name.clone()).collect(),
                initial_state: sm.initial_state.clone(),
                transitions: sm
                    .transitions
                    .iter()
                    .enumerate()
                    .map(|(i, t)| TransitionDecl {
                        id: format!(
                            "transition[{i}]:{}.{}:{}->{}",
                            sm.entity,
                            t.event,
                            t.from.join("|"),
                            t.to
                        ),
                        entity: sm.entity.clone(),
                        event: t.event.clone(),
                        from: t.from.clone(),
                        to: t.to.clone(),
                        has_guard: t.guard.is_some(),
                    })
                    .collect(),
            })
            .collect();

        let workflows = ext
            .workflows
            .iter()
            .enumerate()
            .map(|(i, w)| WorkflowDecl {
                id: format!("workflow[{i}]:{}", w.name),
                name: w.name.clone(),
                entity: w.trigger.entity.clone(),
                event: json_scalar_string(&w.trigger.event),
                condition: w.trigger.condition.clone(),
                steps: w.steps.len(),
            })
            .collect();

        DeclarationInventory {
            constraints,
            rules,
            state_machines,
            workflows,
        }
    }
}

/// 声明清单：四类声明的结构化枚举（顺序 = 声明顺序，供稳定 id 派生）
#[derive(Debug, Clone, Default, Serialize)]
pub struct DeclarationInventory {
    pub constraints: Vec<ConstraintDecl>,
    pub rules: Vec<RuleDecl>,
    pub state_machines: Vec<StateMachineDecl>,
    pub workflows: Vec<WorkflowDecl>,
}

impl DeclarationInventory {
    /// 覆盖单元总数：约束 + 规则 + 状态机迁移 + 工作流
    pub fn total(&self) -> usize {
        self.constraints.len()
            + self.rules.len()
            + self
                .state_machines
                .iter()
                .map(|sm| sm.transitions.len())
                .sum::<usize>()
            + self.workflows.len()
    }

    /// 无任何声明
    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

/// 约束声明
#[derive(Debug, Clone, Serialize)]
pub struct ConstraintDecl {
    /// 稳定 id（`constraint[序]:实体.字段`）
    pub id: String,
    pub entity: String,
    /// 目标字段；`None` = 跨字段 / 实体级约束
    pub field: Option<String>,
    pub expression: String,
    /// `error` / `warning`
    pub level: String,
    pub message: String,
}

/// 业务规则声明
#[derive(Debug, Clone, Serialize)]
pub struct RuleDecl {
    /// 稳定 id（`rule[序]:实体.规则名`）
    pub id: String,
    pub entity: String,
    pub name: String,
    pub trigger: String,
    pub condition: String,
    pub action: String,
    /// 阻塞规则：条件不满足时阻止操作
    pub blocking: bool,
}

/// 状态机声明
#[derive(Debug, Clone, Serialize)]
pub struct StateMachineDecl {
    pub entity: String,
    pub state_field: String,
    pub states: Vec<String>,
    pub initial_state: String,
    pub transitions: Vec<TransitionDecl>,
}

/// 状态机迁移声明（覆盖单元）
#[derive(Debug, Clone, Serialize)]
pub struct TransitionDecl {
    /// 稳定 id（`transition[序]:实体.事件:from->to`）
    pub id: String,
    pub entity: String,
    pub event: String,
    pub from: Vec<String>,
    pub to: String,
    pub has_guard: bool,
}

/// 工作流声明（覆盖单元 = 其 trigger）
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowDecl {
    /// 稳定 id（`workflow[序]:名称`）
    pub id: String,
    pub name: String,
    pub entity: String,
    /// 生命周期事件（`onCreate` / `onUpdate` …）
    pub event: String,
    pub condition: Option<String>,
    pub steps: usize,
}

/// 枚举值 → 字符串（类型是新变体时由 serde 决定，不手写 match 以免漂移）
fn json_scalar_string<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}
