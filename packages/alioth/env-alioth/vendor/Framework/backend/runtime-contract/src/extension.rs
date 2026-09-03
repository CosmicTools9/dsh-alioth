//! 应用级逻辑扩展接口契约
//!
//! 定义应用（Application）向模块（Module）注入业务逻辑的声明式配置类型。
//!
//! # 设计原则
//!
//! 1. **声明式配置**：所有扩展以 YAML/JSON 形式存在，LLM-Agent 生成配置文件而非 Rust 代码。
//! 2. **应用隔离**：扩展按 `app_code` 隔离，不同应用的同一模块可以有不同逻辑。
//! 3. **实体锚定**：每条扩展都锚定到具体实体（entity），运行时按实体名称匹配。
//! 4. **不侵入模块**：模块通过标准 CRUD seam 自动接入扩展，无需修改模块代码。
//!
//! # 文件位置
//!
//! ```text
//! Pre-Proc/{namespace}/Apps/{app}/
//! ├── app.json              ← 模块组合配置
//! └── extensions/
//!     ├── constraints.yaml  ← 约束验证
//!     ├── rules.yaml        ← 业务规则
//!     ├── statemachines.yaml← 状态机覆盖
//!     ├── workflows.yaml    ← 流程编排
//!     └── profiles.yaml     ← 领域模型配置单（叶表激活决策）
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::behavior::{LifecycleEvent, State, Transition};
use crate::model_registry::AppModelConfig;
use crate::swrl::SwrlRule;

// ===================================================================
// AppContext — 应用上下文标识
// ===================================================================

/// 应用上下文 — 标识当前请求所属的应用代码
///
/// Gateway 在每个应用 scope 中注入此类型，Handler 可通过
/// `web::Data<AppContext>` 提取当前应用代码。
#[derive(Debug, Clone)]
pub struct AppContext {
    pub app_code: String,
}

// ===================================================================
// AppLogicExtension — 应用级逻辑扩展总配置
// ===================================================================

/// 应用级逻辑扩展配置
///
/// 由 Gateway 在 `register_apps()` 时读取并注册到 `AppExtensionRegistry`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppLogicExtension {
    /// 应用代码（如 "oms", "wms"）
    pub app_code: String,
    /// 扩展配置版本
    #[serde(default = "default_version")]
    pub version: String,
    /// 约束验证配置
    #[serde(default)]
    pub constraints: Vec<ConstraintExtension>,
    /// 业务规则配置
    #[serde(default)]
    pub business_rules: Vec<RuleExtension>,
    /// 状态机定义/覆盖
    #[serde(default)]
    pub state_machines: Vec<StateMachineExtension>,
    /// 工作流定义
    #[serde(default)]
    pub workflows: Vec<WorkflowDefinition>,
    /// SWRL 语义规则
    #[serde(default)]
    pub swrl_rules: Vec<SwrlRule>,
    /// 领域模型配置单：profile_name -> AppModelConfig
    #[serde(default)]
    pub model_profiles: HashMap<String, AppModelConfig>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

impl AppLogicExtension {
    /// 创建空的扩展配置
    pub fn new(app_code: impl Into<String>) -> Self {
        Self {
            app_code: app_code.into(),
            version: default_version(),
            constraints: Vec::new(),
            business_rules: Vec::new(),
            state_machines: Vec::new(),
            workflows: Vec::new(),
            swrl_rules: Vec::new(),
            model_profiles: HashMap::new(),
        }
    }

    /// 检查是否有任何扩展定义。
    ///
    /// 注意：含 `model_profiles` 的扩展必须视为「非空」并注册到 `AppExtensionRegistry`，
    /// 否则仅有 `profiles.yaml`（领域模型档案）的应用会被整体跳过（原 bug：Gateway
    /// `main.rs` 在 `ext.is_empty()` 为真时放弃注册，导致模型档案永不生效）。
    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
            && self.business_rules.is_empty()
            && self.state_machines.is_empty()
            && self.workflows.is_empty()
            && self.swrl_rules.is_empty()
            && self.model_profiles.is_empty()
    }

    /// 检查是否有领域模型配置单
    pub fn has_model_profiles(&self) -> bool {
        !self.model_profiles.is_empty()
    }

    /// 按名称列表合并多个领域配置单，返回合并后的 AppModelConfig
    ///
    /// 若 `profile_names` 为空或未命中任何配置单，返回默认的 `AppModelConfig`
    ///（即全部启用，向后兼容）。
    pub fn merge_profiles(&self, profile_names: &[String]) -> AppModelConfig {
        let mut merged = AppModelConfig::default();
        for name in profile_names {
            if let Some(profile) = self.model_profiles.get(name) {
                merged.merge(profile);
            }
        }
        merged
    }
}

// ─────────────────────────────────────────────────────────────
// ConstraintExtension — 约束验证扩展
// ─────────────────────────────────────────────────────────────

/// 实体约束扩展
///
/// 在标准 CRUD 的 create/update 前执行验证。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintExtension {
    /// 目标实体名称（如 "Order", "Inventory"）
    pub entity: String,
    /// 目标字段（None 表示跨字段/实体级约束）
    #[serde(default)]
    pub field: Option<String>,
    /// 约束表达式（runtime-engine 表达式语法）
    pub expression: String,
    /// 约束级别
    #[serde(default)]
    pub level: ConstraintSeverity,
    /// 验证失败时的提示消息
    pub message: String,
}

/// 约束验证严重级别
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ConstraintSeverity {
    /// 验证失败阻止操作
    #[default]
    Error,
    /// 验证失败仅警告，不阻止操作
    Warning,
}

// ─────────────────────────────────────────────────────────────
// RuleExtension — 业务规则扩展
// ─────────────────────────────────────────────────────────────

/// 实体业务规则扩展
///
/// 条件-动作模式：当条件满足时执行动作（字段赋值、副作用等）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleExtension {
    /// 目标实体名称
    pub entity: String,
    /// 规则名称（唯一标识）
    pub name: String,
    /// 触发时机（如 "onCreate", "onUpdate", "onDelete", "always"）
    pub trigger: String,
    /// 条件表达式
    pub condition: String,
    /// 动作表达式（如 `discount_rate = 0.15`）
    pub action: String,
    /// 优先级（数值越大越先执行）
    #[serde(default)]
    pub priority: i32,
    /// 错误提示（条件不满足且 blocking=true 时返回）
    #[serde(default)]
    pub error_message: String,
    /// 是否为阻塞规则（条件不满足时阻止操作）
    #[serde(default = "default_true")]
    pub blocking: bool,
}

fn default_true() -> bool {
    true
}

// ─────────────────────────────────────────────────────────────
// StateMachineExtension — 状态机扩展
// ─────────────────────────────────────────────────────────────

/// 实体状态机扩展
///
/// 覆盖或增强模块实体的状态生命周期。若模块实体已有状态机，
/// 应用级定义可以**补充** transitions（不替换原有 states）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateMachineExtension {
    /// 目标实体名称
    pub entity: String,
    /// 状态字段名（如 "status", "t_state"）
    pub state_field: String,
    /// 状态列表
    pub states: Vec<State>,
    /// 状态转换定义
    pub transitions: Vec<Transition>,
    /// 初始状态
    pub initial_state: String,
}

// ─────────────────────────────────────────────────────────────
// WorkflowDefinition — 工作流扩展
// ─────────────────────────────────────────────────────────────

/// 工作流定义
///
/// 跨实体的业务流程编排。当触发条件满足时，按顺序执行步骤。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    /// 工作流名称
    pub name: String,
    /// 描述
    #[serde(default)]
    pub description: Option<String>,
    /// 触发器
    pub trigger: WorkflowTrigger,
    /// 执行步骤
    pub steps: Vec<WorkflowStep>,
}

/// 工作流触发器
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowTrigger {
    /// 目标实体名称
    pub entity: String,
    /// 生命周期事件
    pub event: LifecycleEvent,
    /// 可选的触发条件表达式
    #[serde(default)]
    pub condition: Option<String>,
}

/// 工作流步骤
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// 步骤名称
    pub name: String,
    /// 步骤动作
    pub action: WorkflowAction,
    /// 可选的执行条件表达式
    #[serde(default)]
    pub condition: Option<String>,
    /// 错误处理方式
    #[serde(default)]
    pub on_error: WorkflowErrorHandling,
}

/// 工作流动作类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowAction {
    /// 设置实体字段值
    SetField {
        /// 字段名
        field: String,
        /// 赋值表达式
        expression: String,
    },
    /// 创建关联实体
    CreateRelated {
        /// 目标实体名称
        entity: String,
        /// 字段映射（目标字段 -> 源表达式）
        field_map: HashMap<String, String>,
    },
    /// 调用存储过程/函数
    CallProcedure {
        /// 过程名称
        name: String,
        /// 参数表达式列表
        params: Vec<String>,
    },
    /// 发送通知
    Notify {
        /// 通知渠道（如 "email", "sms", "webhook", "inapp"）
        channel: String,
        /// 通知模板标识
        template: String,
    },
    /// 状态转换
    Transition {
        /// 目标状态
        to_state: String,
        /// 转换事件名
        event: String,
    },
}

/// 工作流错误处理策略
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum WorkflowErrorHandling {
    /// 终止工作流，回滚已执行步骤
    #[default]
    Abort,
    /// 忽略错误，继续执行后续步骤
    Continue,
    /// 重试
    Retry {
        /// 最大重试次数
        max_attempts: u32,
        /// 退避间隔（毫秒）
        backoff_ms: u64,
    },
}

// ─────────────────────────────────────────────────────────────
// ModuleExtensionPoint — 模块扩展点声明
// ─────────────────────────────────────────────────────────────

/// 模块扩展点声明
///
/// 由模块在 `module.json` 中声明，告知应用组装器该模块支持哪些扩展。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleExtensionPoints {
    /// 模块 ID
    pub module_id: String,
    /// 可扩展实体列表
    pub entities: Vec<EntityExtensionPoint>,
}

/// 实体扩展点声明
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityExtensionPoint {
    /// 实体名称（业务概念名，如 "Order"）
    pub entity_name: String,
    /// 物理表名（如 `isahl."zc_id_orde-retail"`）
    pub table_name: String,
    /// 支持的生命周期事件扩展
    #[serde(default)]
    pub supported_events: Vec<LifecycleEvent>,
    /// 是否有内置状态机
    #[serde(default)]
    pub has_state_machine: bool,
    /// 状态字段名（如有）
    #[serde(default)]
    pub state_field: Option<String>,
    /// 可约束字段列表
    #[serde(default)]
    pub constrainable_fields: Vec<String>,
}

// ─────────────────────────────────────────────────────────────
// ExtensionResult — 扩展执行结果
// ─────────────────────────────────────────────────────────────

/// 单条扩展执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionEvaluation {
    /// 扩展类型
    pub extension_type: ExtensionType,
    /// 扩展名称/标识
    pub name: String,
    /// 是否通过/成功
    pub passed: bool,
    /// 错误信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 字段变更（如规则动作修改了字段值）
    #[serde(default)]
    pub mutations: HashMap<String, serde_json::Value>,
    /// 执行时间戳
    pub evaluated_at: String,
}

/// 扩展类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExtensionType {
    Constraint,
    BusinessRule,
    StateMachine,
    Workflow,
    SwrlRule,
}

/// 批量扩展执行结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtensionResult {
    /// 所有执行结果
    pub evaluations: Vec<ExtensionEvaluation>,
    /// 是否全部通过（无阻塞失败）
    pub all_passed: bool,
    /// 字段变更汇总
    #[serde(default)]
    pub mutations: HashMap<String, serde_json::Value>,
    /// 阻塞性错误
    #[serde(default)]
    pub blocking_errors: Vec<String>,
    /// 警告信息
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl ExtensionResult {
    pub fn new() -> Self {
        Self {
            evaluations: Vec::new(),
            all_passed: true,
            mutations: HashMap::new(),
            blocking_errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn add(&mut self, eval: ExtensionEvaluation) {
        if !eval.passed {
            self.all_passed = false;
        }
        if let Some(ref err) = eval.error {
            self.blocking_errors.push(err.clone());
        }
        for (k, v) in &eval.mutations {
            self.mutations.insert(k.clone(), v.clone());
        }
        self.evaluations.push(eval);
    }

    pub fn with_mutation(mut self, field: impl Into<String>, value: serde_json::Value) -> Self {
        self.mutations.insert(field.into(), value);
        self
    }
}
