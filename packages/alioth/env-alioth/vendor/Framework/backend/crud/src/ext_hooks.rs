//! 扩展钩子执行面（`extensions/*.yaml` 声明的运行期入口）
//!
//! 标准路由（`crud_routes` / `crud_routes_with_refs`）在写径前后调用同一实现；**手写写径**
//! （自有 handler / service 直调 repository）MUST 经本模块调用——否则该实体的扩展声明
//! 永不执行（「声明了却无运行期影响」）。
//!
//! 本模块只提供**公开名字与契约**；实现唯一，位于 `crate::handler`（标准路由与手写写径共用，
//! MUST NOT 出现第二套钩子语义）。
//!
//! # 契约
//!
//! - 钩子在**线载荷**（`serde_json::Value`）上运行；`before_*` 的 mutations 合并入载荷**之后**
//!   才反序列化为 DTO ⇒ 请求 DTO 不要求实现 `Serialize`（手写写径若从类型化请求出发，
//!   须 `serde_json::to_value(&req)` ⇒ 该请求类型须 `Serialize`）。
//! - `before_*` 阻断（约束 Error 级 / 状态机 guard 或 action 失败 / 规则错误）⇒ `BadRequest`（400）。
//! - `after_*` best-effort：不阻断主操作，失败与未全通过一律留痕（静默 = 生产不可见）。
//! - `None` 注册表 ⇒ 直通（语义与未声明扩展一致）。
//!
//! # 实体键
//!
//! 两种既有声明口径 MUST 同时可解析：业务名（`AliothDbEntity::ENTITY_NAME`）与物理表名
//! （去 schema/引号，2026-09-22 起 YAML 口径）。标准路由用 [`entity_keys`]；手写写径可传
//! 自己的键集合（如 `&["subjects", "zc_id_subjects"]`）。
//!
//! # 手写写径接法
//!
//! ```ignore
//! use crud::ext_hooks;
//!
//! // 创建：钩子在线载荷上运行，mutations 合并后才反序列化
//! let mut payload = serde_json::to_value(&req)?;
//! ext_hooks::before_create(Some(&registry), ext_hooks::entity_keys::<Requirement>(), &app_code, &mut payload)?;
//! let req: CreateRequirementRequest = serde_json::from_value(payload)?;
//! // …落库…
//! ext_hooks::after_create(Some(&registry), keys, &app_code, &saved);
//!
//! // 更新：`current` = 存量行变量（状态机 from 状态 / guard 存量字段）
//! let current = ext_hooks::dto_to_variables(&existing);
//! ext_hooks::before_update(Some(&registry), keys, &app_code, &mut payload, &current)?;
//!
//! // 删除：上下文 MUST 用 `delete_variables` 构造（存量行 ⊕ `id`）
//! let mut variables = ext_hooks::delete_variables(Some(&existing), id);
//! ext_hooks::before_delete(Some(&registry), keys, &app_code, &mut variables)?;
//! ```

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use crate::entity::AliothDbEntity;
use common::AliothError;

/// 装配面类型再导出：调用方（手写写径）无需直接依赖 `runtime-engine`
pub use runtime_engine::{AppContext, AppExtensionRegistry};

/// 实体候选键（业务名 ∪ 物理表名去 schema/引号）——标准路由与手写写径共用的唯一构造点
pub fn entity_keys<E: AliothDbEntity>() -> [&'static str; 2] {
    crate::handler::entity_keys::<E>()
}

/// DTO / 实体 → 表达式变量（`serde` 视图 = 声明面字段名）
pub fn dto_to_variables<C: Serialize>(dto: &C) -> HashMap<String, Value> {
    crate::handler::dto_to_variables(dto)
}

/// 删除期钩子上下文 = **存量行** ⊕ `id`（唯一构造点）
///
/// 约束是**行状态谓词**：只给 `id` 时，引用其他字段的 Error 级约束会因变量缺失求值失败
/// （求值失败按 `level` 处理 ⇒ Error 即阻断）⇒ 该实体删除恒阻断。
pub fn delete_variables<C: Serialize>(row: Option<&C>, id: i64) -> HashMap<String, Value> {
    crate::handler::delete_variables(row, id)
}

/// 创建前扩展（约束 → 初始状态 → `onCreate` 规则）：阻断 ⇒ 400；mutations 回写线载荷。
pub fn before_create(
    registry: Option<&runtime_engine::AppExtensionRegistry>,
    entity_keys: &[&str],
    app_code: &str,
    payload: &mut Value,
) -> Result<(), AliothError> {
    crate::handler::ext_before_create(registry, entity_keys, app_code, payload)
}

/// 创建后扩展（`afterCreate`/`always` 规则 + 工作流触发标记）：best-effort。
pub fn after_create(
    registry: Option<&runtime_engine::AppExtensionRegistry>,
    entity_keys: &[&str],
    app_code: &str,
    item: &impl Serialize,
) {
    crate::handler::ext_after_create(registry, entity_keys, app_code, item)
}

/// 更新前扩展（约束 → 状态转换 guard 与 action → `onUpdate` 规则）：阻断 ⇒ 400；mutations 回写线载荷。
///
/// `current` = **存量行**变量（状态机需 `from` 状态、guard 需存量字段）；`payload` = 本次提交的新值。
pub fn before_update(
    registry: Option<&runtime_engine::AppExtensionRegistry>,
    entity_keys: &[&str],
    app_code: &str,
    payload: &mut Value,
    current: &HashMap<String, Value>,
) -> Result<(), AliothError> {
    crate::handler::ext_before_update(registry, entity_keys, app_code, payload, current)
}

/// 更新后扩展（`afterUpdate`/`always` 规则 + 工作流触发标记）：best-effort。
pub fn after_update(
    registry: Option<&runtime_engine::AppExtensionRegistry>,
    entity_keys: &[&str],
    app_code: &str,
    item: &impl Serialize,
) {
    crate::handler::ext_after_update(registry, entity_keys, app_code, item)
}

/// 删除前扩展（约束 → `onDelete` 规则）：阻断 ⇒ 400。
///
/// `variables` MUST 由 [`delete_variables`] 构造（存量行 ⊕ `id`）。
pub fn before_delete(
    registry: Option<&runtime_engine::AppExtensionRegistry>,
    entity_keys: &[&str],
    app_code: &str,
    variables: &mut HashMap<String, Value>,
) -> Result<(), AliothError> {
    crate::handler::ext_before_delete(registry, entity_keys, app_code, variables)
}

/// 删除后扩展（`afterDelete`/`always` 规则 + 工作流触发标记）：best-effort。
pub fn after_delete(
    registry: Option<&runtime_engine::AppExtensionRegistry>,
    entity_keys: &[&str],
    app_code: &str,
    id: i64,
) {
    crate::handler::ext_after_delete(registry, entity_keys, app_code, id)
}
