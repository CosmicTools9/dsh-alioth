//! 应用模型注册表契约
//!
//! 定义 AppModelConfig 及其相关类型，供 Gateway 启动时解析 `app.json` 中的
//! `config.modelRegistry` 字段，建立内存注册表。
//!
//! 设计原则：
//! - 纯数据类型，无运行时依赖
//! - 向后兼容：缺失 modelRegistry 时默认启用全部实体
//! - 按 App 代码隔离，按模块/实体两级索引

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────
// AppModelConfig
// ─────────────────────────────────────────────────────────────

/// 单个应用的模型配置
///
/// 从 `app.json` 的 `config.modelRegistry` 字段反序列化而来。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppModelConfig {
    /// 模块级模型配置：module_id -> ModuleModelConfig
    #[serde(default)]
    pub modules: HashMap<String, ModuleModelConfig>,
}

/// 单个模块内的实体启用配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModuleModelConfig {
    /// 启用的实体列表（entity_name）。
    /// 为空表示启用该模块下所有实体（向后兼容默认值）。
    #[serde(default)]
    pub enabled_entities: Vec<String>,
    /// 禁用的实体列表（优先级高于 enabled_entities）。
    #[serde(default)]
    pub disabled_entities: Vec<String>,
}

// ─────────────────────────────────────────────────────────────
// 查询逻辑
// ─────────────────────────────────────────────────────────────

impl AppModelConfig {
    /// 检查指定模块是否在当前应用中被启用
    ///
    /// 规则：模块必须在 `modules` 中存在，且不能全部实体都被禁用。
    pub fn is_module_enabled(&self, module_id: &str) -> bool {
        match self.modules.get(module_id) {
            Some(config) => {
                // 如果模块配置存在，但没有任何实体被启用，视为禁用
                config.is_any_entity_enabled()
            }
            None => {
                // 未声明 modelRegistry 或该模块未在 modelRegistry 中配置：默认启用
                true
            }
        }
    }

    /// 检查指定模块下的指定实体是否被启用
    ///
    /// 判定优先级：
    /// 1. 若 entity_name 在 `disabled_entities` 中 → 禁用
    /// 2. 若 `enabled_entities` 为空 → 启用全部（向后兼容）
    /// 3. 若 entity_name 在 `enabled_entities` 中 → 启用
    /// 4. 其他 → 禁用
    pub fn is_entity_enabled(&self, module_id: &str, entity_name: &str) -> bool {
        match self.modules.get(module_id) {
            Some(config) => config.is_entity_enabled(entity_name),
            None => {
                // 该模块未在 modelRegistry 中配置：默认启用
                true
            }
        }
    }

    /// 检查指定实体是否在任何模块中被启用（跨模块查询）
    ///
    /// 用于扩展加载过滤：扩展文件中的 entity 不携带 module_id，
    /// 因此需要遍历所有模块配置，只要任一模块启用了该实体即视为启用。
    ///
    /// 若 modelRegistry 为空（未配置），返回 true（向后兼容）。
    pub fn is_entity_enabled_any_module(&self, entity_name: &str) -> bool {
        if self.modules.is_empty() {
            return true;
        }
        self.modules
            .values()
            .any(|config| config.is_entity_enabled(entity_name))
    }

    /// 合并另一个应用模型配置
    ///
    /// 按模块维度合并：对同一模块的配置调用 `ModuleModelConfig::merge`，
    /// 新模块直接插入。
    pub fn merge(&mut self, other: &AppModelConfig) {
        for (module_id, other_config) in &other.modules {
            self.modules
                .entry(module_id.clone())
                .or_default()
                .merge(other_config);
        }
    }
}

impl ModuleModelConfig {
    /// 检查指定实体是否被启用
    pub fn is_entity_enabled(&self, entity_name: &str) -> bool {
        // 1. 显式禁用优先
        if self.disabled_entities.iter().any(|e| e == entity_name) {
            return false;
        }

        // 2. enabled_entities 为空表示启用全部（向后兼容）
        if self.enabled_entities.is_empty() {
            return true;
        }

        // 3. 必须在 enabled_entities 中才算启用
        self.enabled_entities.iter().any(|e| e == entity_name)
    }

    /// 检查该模块下是否有任何实体被启用
    fn is_any_entity_enabled(&self) -> bool {
        if self.enabled_entities.is_empty() {
            // 向后兼容：默认全部启用
            true
        } else {
            // 显式声明了 enabled_entities，至少有一个
            !self.enabled_entities.is_empty()
        }
    }

    /// 合并另一个模块配置
    ///
    /// 合并规则：
    /// - `disabled_entities`：取并集（任一配置单禁用的都禁用）
    /// - `enabled_entities`：如果任一方为空（表示全部启用），合并后也为空；
    ///   否则取并集
    pub fn merge(&mut self, other: &ModuleModelConfig) {
        // disabled: 并集
        for e in &other.disabled_entities {
            if !self.disabled_entities.contains(e) {
                self.disabled_entities.push(e.clone());
            }
        }
        // enabled: 任一方为空表示全部启用，合并后也为空
        if self.enabled_entities.is_empty() || other.enabled_entities.is_empty() {
            self.enabled_entities.clear();
        } else {
            for e in &other.enabled_entities {
                if !self.enabled_entities.contains(e) {
                    self.enabled_entities.push(e.clone());
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────
// 过滤辅助函数
// ─────────────────────────────────────────────────────────────

impl AppModelConfig {
    /// 过滤约束列表，仅保留启用的实体对应的约束
    ///
    /// 供 Gateway 加载 `extensions/*.yaml` 时使用。
    pub fn filter_constraints(
        &self,
        module_id: &str,
        constraints: Vec<crate::extension::ConstraintExtension>,
    ) -> Vec<crate::extension::ConstraintExtension> {
        constraints
            .into_iter()
            .filter(|c| self.is_entity_enabled(module_id, &c.entity))
            .collect()
    }

    /// 过滤业务规则列表，仅保留启用的实体对应的规则
    pub fn filter_rules(
        &self,
        module_id: &str,
        rules: Vec<crate::extension::RuleExtension>,
    ) -> Vec<crate::extension::RuleExtension> {
        rules
            .into_iter()
            .filter(|r| self.is_entity_enabled(module_id, &r.entity))
            .collect()
    }

    /// 过滤状态机列表，仅保留启用的实体对应的状态机
    pub fn filter_state_machines(
        &self,
        module_id: &str,
        state_machines: Vec<crate::extension::StateMachineExtension>,
    ) -> Vec<crate::extension::StateMachineExtension> {
        state_machines
            .into_iter()
            .filter(|sm| self.is_entity_enabled(module_id, &sm.entity))
            .collect()
    }

    /// 过滤工作流列表，仅保留启用的实体触发的工作流
    pub fn filter_workflows(
        &self,
        module_id: &str,
        workflows: Vec<crate::extension::WorkflowDefinition>,
    ) -> Vec<crate::extension::WorkflowDefinition> {
        workflows
            .into_iter()
            .filter(|w| self.is_entity_enabled(module_id, &w.trigger.entity))
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────
// 单元测试
// ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_enable_all() {
        let config = AppModelConfig::default();
        assert!(config.is_entity_enabled("product", "ManuProduct"));
        assert!(config.is_entity_enabled("orders", "Order"));
    }

    #[test]
    fn test_module_not_configured_defaults_to_enabled() {
        let mut config = AppModelConfig::default();
        config.modules.insert(
            "orders".to_string(),
            ModuleModelConfig {
                enabled_entities: vec!["Order".to_string()],
                disabled_entities: vec![],
            },
        );
        // product 模块未配置：默认启用
        assert!(config.is_entity_enabled("product", "ManuProduct"));
    }

    #[test]
    fn test_enabled_entities_list() {
        let module_config = ModuleModelConfig {
            enabled_entities: vec!["SalesProduct".to_string()],
            ..Default::default()
        };
        let mut config = AppModelConfig::default();
        config.modules.insert("product".to_string(), module_config);

        assert!(config.is_entity_enabled("product", "SalesProduct"));
        assert!(!config.is_entity_enabled("product", "ManuProduct"));
    }

    #[test]
    fn test_disabled_entities_priority() {
        let module_config = ModuleModelConfig {
            enabled_entities: vec!["ManuProduct".to_string(), "SalesProduct".to_string()],
            disabled_entities: vec!["ManuProduct".to_string()],
        };

        let mut config = AppModelConfig::default();
        config.modules.insert("product".to_string(), module_config);

        // disabled 优先级高于 enabled
        assert!(!config.is_entity_enabled("product", "ManuProduct"));
        assert!(config.is_entity_enabled("product", "SalesProduct"));
    }

    #[test]
    fn test_empty_enabled_means_all() {
        let module_config = ModuleModelConfig::default();
        let mut config = AppModelConfig::default();
        config.modules.insert("product".to_string(), module_config);

        assert!(config.is_entity_enabled("product", "ManuProduct"));
        assert!(config.is_entity_enabled("product", "SalesProduct"));
    }

    #[test]
    fn test_disabled_in_all_enabled() {
        let module_config = ModuleModelConfig {
            disabled_entities: vec!["ManuProduct".to_string()],
            ..Default::default()
        };
        let mut config = AppModelConfig::default();
        config.modules.insert("product".to_string(), module_config);

        // enabled_entities 为空 → 全部启用，但 disabled 优先
        assert!(!config.is_entity_enabled("product", "ManuProduct"));
        assert!(config.is_entity_enabled("product", "SalesProduct"));
    }
}
