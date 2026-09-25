//! Gateway 启动期扩展实体引用校验 —— `runtime-engine/src/extension.rs:917` 既定意图的落地调用点
//!
//! 加载 extensions 后调用 `ExtensionLoader::validate_entities`（引擎既有实现，禁第二份）
//! 校验 constraints / rules / statemachines 引用的实体与字段，违规**逐条**记录 ERROR 级别
//! 日志（经 `common::telemetry`，底层为 `log` crate——业务模块禁直接用 `tracing`）。
//! 校验失败 MUST NOT 阻止扩展加载、MUST NOT fail-fast：运行期可用性不因存量脏数据受损，
//! 阻断职责归 compose-time / repo 门禁（openspec change `enforce-semantic-verification-gates` D3）。
//!
//! # 已知实体面来源（启动期可得的独立真值）
//!
//! 校验 MUST NOT 从声明自身取实体集（自证恒真）。Gateway 启动期可得的候选与取舍：
//!
//! |候选|为何不取 / 取|
//! |---|---|
//! |`isahl_meta.meta_*`（模型中心实体表）|**容器边界禁止** Gateway 读 —— `CONTAINER_BOUNDARY`：Gateway 只在 `isahl`(只读)/`isahl_auth` 域内|
//! |`flow-plan.json.known_entities`|语义是**平台已有表名**（`zc_id_*`），与扩展声明的业务实体名不同命名空间 → 用作真值会系统性误报|
//! |`{app_dir}/Services/*/service.json` 的 `ontology.entities[]`|**取此面**：与 `load_extensions` 同属「app 产物目录」这一启动期既有加载面（无需新依赖、新通道），且就是扩展 `entity` 所指的实体声明面；字段集取 `field_mappings[].json_path`|
//!
//! 该面缺失（app 未链接 Service 产物 / 服务未声明本体实体）→ 空面，调用方跳过校验并 WARN：
//! 「缺失即跳过并告警」，避免把未知当缺失产生假阳性（与 `seed/ns_seed.rs` 判定面缺失、
//! `app_agent::extension_verify` 无 FlowPlan 降级同一语义）。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use runtime_engine::{AppLogicExtension, ExtensionLoader};

/// 已知实体面：`entity_name → 该实体已知字段集`。
///
/// 字段集为空 = 字段级知识不可得 → 只保留实体级判定（不得把未知字段当缺失）。
pub type KnownEntityFace = HashMap<String, HashSet<String>>;

/// 从 app 产物目录装配已知实体面。
///
/// 扫描 `{app_dir}/Services/{unit}/service.json`，取 `ontology.entities[].name` 为实体名、
/// `field_mappings[].json_path` 为字段名（同一实体跨单元取并集）。
/// 读不动 / 解析失败 / 无实体声明的单元静默跳过（不阻断，不影响调用方加载语义）。
pub fn known_entity_face_from_app_dir(app_dir: &Path) -> KnownEntityFace {
    let mut face = KnownEntityFace::new();

    let services = app_dir.join("Services");
    let Ok(units) = std::fs::read_dir(&services) else {
        return face;
    };

    for unit in units.flatten() {
        let service_json = unit.path().join("service.json");
        if !service_json.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&service_json) else {
            continue;
        };
        let Ok(doc) = serde_json::from_str::<ServiceOntologyDoc>(&content) else {
            continue;
        };
        for entity in doc.ontology.entities {
            let Some(name) = entity.name.filter(|n| !n.is_empty()) else {
                continue;
            };
            let fields = face.entry(name).or_default();
            for mapping in entity.field_mappings {
                if let Some(json_path) = mapping.json_path.filter(|p| !p.is_empty()) {
                    fields.insert(json_path);
                }
            }
        }
    }

    face
}

/// 校验单个已加载扩展的实体/字段引用**与表达式**并逐条 ERROR 日志；返回违规条数（0 = 全部通过）。
///
/// 违规文本由 `validate_entities`（实体/字段引用）与 `collect_expression_violations`
/// （表达式：语法 + 自由函数白名单，引擎单一实现）生成；本函数只负责**可见性**：
/// 逐条 ERROR + 不改变任何加载状态（运行期可用性优先，阻断职责归 compose-time / repo 门禁）。
pub fn validate_and_log(extension: &AppLogicExtension, known: &KnownEntityFace) -> usize {
    // 空字段集 = 字段级知识不可得：以声明自身字段填充使该实体字段级退化为恒真
    // （不把未知当缺失），实体级判定保持有效——与 app-agent 旧形态条目同一降级语义。
    let mut sanitized = known.clone();
    for constraint in &extension.constraints {
        if let Some(fields) = sanitized.get_mut(&constraint.entity) {
            if fields.is_empty() {
                if let Some(f) = &constraint.field {
                    fields.insert(f.clone());
                }
            }
        }
    }
    let violations = ExtensionLoader::validate_entities(extension, &sanitized);
    for violation in &violations {
        common::telemetry::error!("[extension-entities] {}", violation);
    }
    // 表达式面（加载期唯一实现 = runtime-engine `collect_expression_violations`）：
    // 退役文法 / 未注册自由函数在此逐条 ERROR 可见（不阻断加载，同实体面策略）。
    let expr_violations = runtime_engine::collect_expression_violations(extension);
    for violation in &expr_violations {
        common::telemetry::error!("[extension-expressions] {}", violation);
    }
    // 非致命提示（裸标识符 guard 等）：启动期 WARN 可见，不阻断、不计入违规数
    // （覆盖「无 flow-plan ⇒ verify-extensions 整体跳过」的 app —— 加载期是其唯一机械可见面）。
    for advisory in runtime_engine::collect_expression_advisories(extension) {
        common::telemetry::warn!("[extension-advisory] {}", advisory);
    }
    violations.len() + expr_violations.len()
}

// ── service.json 最小反序列化面（只取实体/字段声明，其余字段忽略） ──────────────

#[derive(serde::Deserialize)]
struct ServiceOntologyDoc {
    #[serde(default)]
    ontology: ServiceOntology,
}

#[derive(serde::Deserialize, Default)]
struct ServiceOntology {
    #[serde(default)]
    entities: Vec<ServiceOntologyEntity>,
}

#[derive(serde::Deserialize)]
struct ServiceOntologyEntity {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    field_mappings: Vec<ServiceFieldMapping>,
}

#[derive(serde::Deserialize)]
struct ServiceFieldMapping {
    #[serde(default)]
    json_path: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, LazyLock, Mutex};

    use runtime_engine::{
        AppExtensionRegistry, ConstraintExtension, ConstraintSeverity, RuleExtension,
        StateMachineExtension,
    };

    use super::*;

    /// 捕获 logger：断言「违规确实以 ERROR 级别发出」，而非只断言返回值。
    struct CaptureLogger;

    /// 捕获到的日志行（级别 + 文本）。
    type RecordedLogs = Arc<Mutex<Vec<(log::Level, String)>>>;

    static RECORDS: LazyLock<RecordedLogs> = LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));
    static LOGGER: CaptureLogger = CaptureLogger;

    impl log::Log for CaptureLogger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            RECORDS
                .lock()
                .expect("records mutex")
                .push((record.level(), record.args().to_string()));
        }

        fn flush(&self) {}
    }

    /// 安装捕获 logger；已有 logger 时返回 None（此时调用方退化为断言返回值）。
    fn capture_errors() -> Option<&'static RecordedLogs> {
        RECORDS.lock().expect("records mutex").clear();
        if log::set_logger(&LOGGER).is_err() {
            return None;
        }
        log::set_max_level(log::LevelFilter::Error);
        Some(&RECORDS)
    }

    fn extension_with_dirty_references() -> AppLogicExtension {
        let mut extension = AppLogicExtension::new("dirty-app");
        extension.constraints = vec![ConstraintExtension {
            entity: "ghost_entity".to_string(),
            field: Some("ghost_field".to_string()),
            expression: "ghost_field > 0".to_string(),
            level: ConstraintSeverity::Error,
            message: "幽灵实体约束".to_string(),
        }];
        extension.business_rules = vec![RuleExtension {
            entity: "ghost_entity".to_string(),
            name: "ghost_rule".to_string(),
            trigger: "OnCreate".to_string(),
            condition: "1 == 1".to_string(),
            action: String::new(),
            priority: 1,
            error_message: String::new(),
            blocking: true,
        }];
        extension.state_machines = vec![StateMachineExtension {
            entity: "ghost_state_entity".to_string(),
            state_field: "t_state".to_string(),
            states: vec![],
            transitions: vec![],
            initial_state: "Draft".to_string(),
        }];
        extension
    }

    fn face_of(names: &[(&str, &[&str])]) -> KnownEntityFace {
        names
            .iter()
            .map(|(name, fields)| {
                (
                    (*name).to_string(),
                    fields
                        .iter()
                        .map(|f| (*f).to_string())
                        .collect::<HashSet<_>>(),
                )
            })
            .collect()
    }

    /// 引用不存在实体 → ERROR 日志逐条产出，且扩展照常加载（不中断、不 fail-fast）。
    #[test]
    fn unknown_entity_references_log_errors_and_do_not_block_loading() {
        let captured = capture_errors();
        let registry = AppExtensionRegistry::new();
        let extension = extension_with_dirty_references();

        // 启动加载语义：先注册，再校验（校验结果不得回滚注册）
        registry.register(extension.clone());
        let violations =
            validate_and_log(&extension, &face_of(&[("real_entity", &["real_field"])]));

        assert!(
            registry.get_profile("dirty-app").is_some(),
            "校验失败 MUST NOT 阻止扩展加载"
        );
        assert!(
            violations >= 3,
            "constraint/rule/state machine 三处脏引用都应被判定，实际 {violations}"
        );

        if let Some(records) = captured {
            let guard = records.lock().expect("records mutex");
            let errors: Vec<&String> = guard
                .iter()
                .filter(|(level, _)| *level == log::Level::Error)
                .map(|(_, message)| message)
                .collect();
            assert!(
                errors
                    .iter()
                    .any(|m| m.contains("dirty-app") && m.contains("ghost_entity")),
                "缺少数值明细的 ERROR 记录：{errors:?}"
            );
            assert!(
                errors.iter().any(|m| m.contains("ghost_state_entity")),
                "状态机脏引用 ERROR 记录缺失：{errors:?}"
            );
        }
    }

    /// 已知实体面的字段级知识：实体存在、字段存在 → 无违规；字段不存在 → 违规（不自证恒真）。
    #[test]
    fn field_level_verdict_follows_known_face() {
        let mut extension = AppLogicExtension::new("clean-app");
        extension.constraints = vec![ConstraintExtension {
            entity: "real_entity".to_string(),
            field: Some("real_field".to_string()),
            expression: "real_field > 0".to_string(),
            level: ConstraintSeverity::Error,
            message: "字段约束".to_string(),
        }];
        assert_eq!(
            validate_and_log(&extension, &face_of(&[("real_entity", &["real_field"])])),
            0
        );

        extension.constraints[0].field = Some("absent_field".to_string());
        assert_eq!(
            validate_and_log(&extension, &face_of(&[("real_entity", &["real_field"])])),
            1
        );
    }

    /// 字段集不可得（空集）→ 只做实体级判定：不得把未知字段当缺失。
    #[test]
    fn empty_field_set_keeps_entity_level_only() {
        let mut extension = AppLogicExtension::new("entity-level-app");
        extension.constraints = vec![ConstraintExtension {
            entity: "real_entity".to_string(),
            field: Some("any_field".to_string()),
            expression: "any_field > 0".to_string(),
            level: ConstraintSeverity::Error,
            message: "字段约束".to_string(),
        }];
        assert_eq!(
            validate_and_log(&extension, &face_of(&[("real_entity", &[])])),
            0
        );
    }

    /// 已知实体面装配：读 app 产物目录 Services/*/service.json 的实体与字段，缺失面为空。
    #[test]
    fn known_face_assembles_from_app_artifact_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let app_dir = dir.path().join("Apps").join("some-app");
        let unit_dir = app_dir.join("Services").join("orders");
        std::fs::create_dir_all(&unit_dir).expect("mkdir services unit");
        std::fs::write(
            unit_dir.join("service.json"),
            r#"{
              "id": "orders",
              "ontology": {
                "entities": [
                  {"name": "service_order", "table": "isahl.zc_id_stat-trade_order",
                   "field_mappings": [{"column": "notice", "json_path": "name"}, {"column": "qk_amount", "json_path": "amount"}]},
                  {"name": "order_item", "field_mappings": [{"column": "notice", "json_path": "name"}]}
                ]
              }
            }"#,
        )
        .expect("write service.json");
        // 无实体声明的单元不得污染已知面（也不得让装配失败）
        let noise = app_dir.join("Services").join("noise");
        std::fs::create_dir_all(&noise).expect("mkdir noise unit");
        std::fs::write(noise.join("service.json"), r#"{"id":"noise"}"#).expect("write noise");

        let face = known_entity_face_from_app_dir(&app_dir);
        assert_eq!(face.len(), 2, "只应有声明实体的单元贡献实体：{face:?}");
        assert!(face["service_order"].contains("name") && face["service_order"].contains("amount"));
        assert!(face["order_item"].contains("name"));

        // 无 Services/ 目录（app 未链接 Service 产物）→ 空面（调用方跳过校验，不误报）
        assert!(
            known_entity_face_from_app_dir(&dir.path().join("Apps").join("bare-app")).is_empty()
        );
    }
}
