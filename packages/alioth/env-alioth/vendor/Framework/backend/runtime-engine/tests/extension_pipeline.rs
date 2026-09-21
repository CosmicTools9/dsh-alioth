//! Extension 管线端到端集成测试
//!
//! 验证从 YAML → `ExtensionSurface`（单一装配路径）→ 引擎执行的完整链路，
//! 不依赖 Gateway HTTP 层和数据库。
//!
//! 装配一律经 `ExtensionSurface`（禁止本文件内联 `AppExtensionRegistry::new()` +
//! `register()` 的第二套装配）：测试通过 == 真实产物路径通过。

use runtime_engine::ExtensionSurface;

/// constraints.yaml 的内容（内嵌以保持测试自包含）
const CONSTRAINTS_YAML: &str = r#"
- entity: Subject
  field: name
  expression: "name != null AND name != ''"
  level: Error
  message: "客户名称不能为空"

- entity: Subject
  field: code
  expression: "code == null OR code == '' OR code != 'INVALID'"
  level: Error
  message: "客户编码不能为 INVALID"

- entity: Subject
  field: null
  expression: "public != true OR name != null"
  level: Error
  message: "公开客户必须填写名称"

- entity: Subject
  field: null
  expression: "_f_ == 'personal' OR _f_ == 'company' OR _f_ == 'government' OR _f_ == null OR _f_ == ''"
  level: Error
  message: "业务形态必须是 personal、company、government 或留空"
"#;

/// rules.yaml 的内容（内嵌以保持测试自包含）
const RULES_YAML: &str = r#"
- entity: Subject
  name: auto_company_for_public
  trigger: onCreate
  condition: "public == true AND (_f_ == null OR _f_ == '')"
  action: "_f_ = 'company'"
  priority: 100
  error_message: "公开客户自动设为公司形态"
  blocking: false

- entity: Subject
  name: default_name_from_code
  trigger: onCreate
  condition: "(name == null OR name == '') AND code != null AND code != ''"
  action: "name = code"
  priority: 200
  error_message: "已根据编码自动填充名称"
  blocking: false

- entity: Subject
  name: block_test_code
  trigger: onCreate
  condition: "code == 'test'"
  action: ""
  priority: 300
  error_message: "不允许使用 'test' 作为客户编码"
  blocking: true
"#;

/// 从内嵌 YAML 构造 AppLogicExtension
fn make_test_extension(app_code: &str) -> runtime_engine::AppLogicExtension {
    // 解析 constraints
    let constraints: Vec<runtime_contract::extension::ConstraintExtension> =
        yaml_serde::from_str(CONSTRAINTS_YAML).expect("无效的 constraints.yaml");

    // 解析 rules
    let rules: Vec<runtime_contract::extension::RuleExtension> =
        yaml_serde::from_str(RULES_YAML).expect("无效的 rules.yaml");

    let mut ext = runtime_engine::AppLogicExtension::new(app_code);
    ext.constraints = constraints;
    ext.business_rules = rules;
    ext
}

/// 从内嵌 YAML 装配执行面（唯一装配入口）
fn test_surface(app_code: &str) -> ExtensionSurface {
    ExtensionSurface::from_extension(make_test_extension(app_code))
}

/// 从变量对构造 HashMap
fn vars(
    pairs: &[(&str, serde_json::Value)],
) -> std::collections::HashMap<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

// ─────────────────────────────────────────────────────
// 测试：约束验证
// ─────────────────────────────────────────────────────

#[test]
fn test_constraint_name_required() {
    let surface = test_surface("test-app");

    // name 为空 → 约束应失败
    let mut variables = vars(&[
        ("name", serde_json::json!("")),
        ("code", serde_json::json!("C001")),
        ("public", serde_json::json!(true)),
        ("_f_", serde_json::json!("company")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(!result.all_passed, "名称为空应不通过约束");
    assert!(!result.blocking_errors.is_empty(), "应有阻塞错误");

    // name 非空 → 应通过
    let mut variables = vars(&[
        ("name", serde_json::json!("Acme Corp")),
        ("code", serde_json::json!("C001")),
        ("public", serde_json::json!(true)),
        ("_f_", serde_json::json!("company")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(result.all_passed, "名称非空应通过");
}

#[test]
fn test_constraint_code_invalid() {
    let surface = test_surface("test-app");

    // code == 'INVALID' → 应不通过
    let mut variables = vars(&[
        ("name", serde_json::json!("Acme")),
        ("code", serde_json::json!("INVALID")),
        ("public", serde_json::json!(true)),
        ("_f_", serde_json::json!("company")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(!result.all_passed, "INVALID 编码应不通过");

    // code 为空 → 应通过（可选字段）
    let mut variables = vars(&[
        ("name", serde_json::json!("Acme")),
        ("code", serde_json::json!("")),
        ("public", serde_json::json!(true)),
        ("_f_", serde_json::json!("company")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(result.all_passed, "空编码应通过");
}

#[test]
fn test_constraint_cross_field_public_requires_name() {
    let surface = test_surface("test-app");

    // public=true + name=null → 应不通过
    let mut variables = vars(&[
        ("name", serde_json::json!(null)),
        ("code", serde_json::json!("C001")),
        ("public", serde_json::json!(true)),
        ("_f_", serde_json::json!("company")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(!result.all_passed, "公开客户无名称应不通过");

    // public=false + name=有值 → 应通过
    let mut variables = vars(&[
        ("name", serde_json::json!("Acme")),
        ("code", serde_json::json!("C001")),
        ("public", serde_json::json!(false)),
        ("_f_", serde_json::json!("personal")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(result.all_passed, "非公开客户有名称应通过");
}

// ─────────────────────────────────────────────────────
// 测试：业务规则
// ─────────────────────────────────────────────────────

#[test]
fn test_rule_auto_company_for_public() {
    let surface = test_surface("test-app");

    // public=true + _f_ 未设置 → 规则应自动填充 company
    let mut variables = vars(&[
        ("name", serde_json::json!("Acme")),
        ("code", serde_json::json!("C001")),
        ("public", serde_json::json!(true)),
        ("_f_", serde_json::json!(null)),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(result.all_passed, "规则执行后应全部通过");
    // 验证 mutations 中存在 _f_ = 'company'
    assert_eq!(
        variables.get("_f_"),
        Some(&serde_json::json!("company")),
        "_f_ 应被规则自动填充为 company"
    );
}

#[test]
fn test_rule_does_not_trigger_when_name_conflicts_with_constraint() {
    // 验证：约束先于规则执行。
    // `default_name_from_code` 规则条件是 (name==null)，
    // 但约束 1 要求 name != null AND name != '' —— 互斥。
    // 这意味着规则在约束验证后永远无法触发。
    // 这是设计选择（约束优先于规则），此处仅验证行为。
    let surface = test_surface("test-app");

    // name=null → 约束 1 失败，规则不会执行
    let mut variables = vars(&[
        ("name", serde_json::json!(null)),
        ("code", serde_json::json!("C001")),
        ("public", serde_json::json!(false)),
        ("_f_", serde_json::json!("personal")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(!result.all_passed, "name=null 时约束应先于规则执行");
    // name 不应被规则填充
    assert_eq!(
        variables.get("name"),
        Some(&serde_json::json!(null)),
        "规则未执行，name 应保持 null"
    );
}

// ─────────────────────────────────────────────────────
// 测试：阻塞规则
// ─────────────────────────────────────────────────────

#[test]
fn test_blocking_rule_rejects_test_code() {
    let surface = test_surface("test-app");

    // code='test' → 阻塞规则应阻止 create
    let mut variables = vars(&[
        ("name", serde_json::json!("Acme")),
        ("code", serde_json::json!("test")),
        ("public", serde_json::json!(true)),
        ("_f_", serde_json::json!("company")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(!result.all_passed, "test 编码应被阻塞");
    assert!(
        result.blocking_errors.iter().any(|e| e.contains("test")),
        "阻塞错误应包含 test 相关消息"
    );

    // code='normal' → 应通过
    let mut variables = vars(&[
        ("name", serde_json::json!("Acme")),
        ("code", serde_json::json!("normal")),
        ("public", serde_json::json!(true)),
        ("_f_", serde_json::json!("company")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(result.all_passed, "normal 编码应通过");
}

// ─────────────────────────────────────────────────────
// 测试：app_code 隔离
// ─────────────────────────────────────────────────────

#[test]
fn test_app_code_isolation() {
    // app-a 有扩展；app-b 无扩展（空扩展装配）
    let surface_b =
        ExtensionSurface::from_extension(runtime_engine::AppLogicExtension::new("app-b"));

    let mut variables = vars(&[
        ("name", serde_json::json!("")), // 即使数据无效
        ("code", serde_json::json!("INVALID")),
    ]);
    let result = surface_b.create("Subject", &mut variables).unwrap();
    assert!(result.all_passed, "未注册的 app 不应有约束验证");
    assert!(
        result.evaluations.is_empty(),
        "未注册的 app 不应有任何 evaluation"
    );
}

// ─────────────────────────────────────────────────────
// 测试：ExtensionLoader 加载真实 YAML 文件
// ─────────────────────────────────────────────────────

#[test]
fn test_extension_loader_parses_yaml() {
    let app_code = "test-app";

    // 使用 ExtensionLoader 直接从字符串解析（模拟文件加载）
    let mut ext = runtime_engine::AppLogicExtension::new(app_code);

    let constraints: Vec<runtime_contract::extension::ConstraintExtension> =
        yaml_serde::from_str(CONSTRAINTS_YAML).unwrap();
    let rules: Vec<runtime_contract::extension::RuleExtension> =
        yaml_serde::from_str(RULES_YAML).unwrap();

    ext.constraints = constraints;
    ext.business_rules = rules;

    assert_eq!(ext.constraints.len(), 4, "应有 4 条约束");
    assert_eq!(ext.business_rules.len(), 3, "应有 3 条规则");

    // 验证 blocking 默认值
    assert!(
        ext.business_rules
            .iter()
            .find(|r| r.name == "block_test_code")
            .unwrap()
            .blocking,
        "block_test_code 的 blocking 应默认为 true"
    );
    assert!(
        !ext.business_rules
            .iter()
            .find(|r| r.name == "auto_company_for_public")
            .unwrap()
            .blocking,
        "auto_company_for_public 的 blocking 应为 false"
    );
}

/// 唯一临时目录（进程内互不覆盖；无 tempfile 依赖）
fn unique_tmp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("rt-ext-{}-{}-{}", tag, std::process::id(), nanos))
}

#[test]
fn test_surface_from_dir_loads_real_artifacts() {
    let dir = unique_tmp_dir("fromdir");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("constraints.yaml"), CONSTRAINTS_YAML).unwrap();
    std::fs::write(dir.join("rules.yaml"), RULES_YAML).unwrap();

    let surface = ExtensionSurface::from_dir("test-app", &dir).expect("目录加载应成功");
    let inventory = surface.inventory();
    assert_eq!(inventory.constraints.len(), 4, "目录加载应含 4 条约束");
    assert_eq!(inventory.rules.len(), 3, "目录加载应含 3 条规则");
    assert_eq!(inventory.total(), 7, "覆盖单元总数 = 约束 + 规则");

    // 真实加载的扩展应可执行（与内存装配同一条执行路径）
    let mut variables = vars(&[
        ("name", serde_json::json!("")),
        ("code", serde_json::json!("C001")),
        ("public", serde_json::json!(true)),
        ("_f_", serde_json::json!("company")),
    ]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(!result.all_passed, "目录加载的约束应生效");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_surface_from_missing_dir_is_empty_not_error() {
    let dir = unique_tmp_dir("missing");
    let surface = ExtensionSurface::from_dir("test-app", &dir).expect("目录缺失不应报错");
    assert!(
        surface.inventory().is_empty(),
        "缺失目录 → 无声明（空扩展，同 loader 既有语义）"
    );

    let mut variables = vars(&[("name", serde_json::json!(""))]);
    let result = surface.create("Subject", &mut variables).unwrap();
    assert!(result.all_passed, "空扩展不应产生阻塞");
    assert!(result.evaluations.is_empty(), "空扩展不应产生 evaluation");
}

// ─────────────────────────────────────────────────────
// 测试：声明清单（覆盖判定输入面）
// ─────────────────────────────────────────────────────

#[test]
fn test_surface_inventory_ids_are_stable_and_unique() {
    let surface = test_surface("test-app");
    let inv = surface.inventory();

    assert_eq!(inv.constraints.len(), 4);
    assert_eq!(inv.rules.len(), 3);
    assert_eq!(inv.state_machines.len(), 0);
    assert_eq!(inv.total(), 7);

    // id 稳定且唯一（覆盖报告以 id 为键）
    let mut ids: Vec<&str> = inv
        .constraints
        .iter()
        .map(|c| c.id.as_str())
        .chain(inv.rules.iter().map(|r| r.id.as_str()))
        .collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), before, "声明 id 必须唯一");

    // 字段级 / 跨字段约束可区分
    assert_eq!(inv.constraints[0].field.as_deref(), Some("name"));
    assert!(
        inv.constraints[2].field.is_none(),
        "跨字段约束 field 为 None"
    );
    assert_eq!(inv.constraints[0].level, "error");
    assert!(inv.rules.iter().any(|r| r.blocking), "应含阻塞规则声明");
}

// ─────────────────────────────────────────────────────
// 测试：状态机（before_update 转换验证 + on_transition guard）
// ─────────────────────────────────────────────────────

/// 带状态机定义的测试扩展（含带 guard 的迁移）
fn make_state_machine_extension(app_code: &str) -> runtime_engine::AppLogicExtension {
    use runtime_contract::extension::StateMachineExtension;

    let mut ext = runtime_engine::AppLogicExtension::new(app_code);
    ext.state_machines = vec![StateMachineExtension {
        entity: "Order".to_string(),
        state_field: "t_state".to_string(),
        states: vec![
            runtime_contract::behavior::State::new("Pending"),
            runtime_contract::behavior::State::new("Confirmed"),
            runtime_contract::behavior::State::new("Shipped"),
            runtime_contract::behavior::State::new("Delivered"),
            runtime_contract::behavior::State::new("Cancelled"),
        ],
        transitions: vec![
            runtime_contract::behavior::Transition::new("confirm", "Pending", "Confirmed"),
            runtime_contract::behavior::Transition::new("ship", "Confirmed", "Shipped"),
            runtime_contract::behavior::Transition::new("deliver", "Shipped", "Delivered"),
            runtime_contract::behavior::Transition {
                event: "cancel".to_string(),
                from: vec!["Pending".to_string(), "Confirmed".to_string()],
                to: "Cancelled".to_string(),
                guard: None,
                action: None,
                is_default: false,
            },
        ],
        initial_state: "Pending".to_string(),
    }];
    ext
}

/// 带 guard 的状态机（确认须先付款）
fn make_guarded_state_machine_ext(app_code: &str) -> runtime_engine::AppLogicExtension {
    use runtime_contract::extension::StateMachineExtension;

    let mut ext = runtime_engine::AppLogicExtension::new(app_code);
    ext.state_machines = vec![StateMachineExtension {
        entity: "Order".to_string(),
        state_field: "t_state".to_string(),
        states: vec![
            runtime_contract::behavior::State::new("Pending"),
            runtime_contract::behavior::State::new("Confirmed"),
        ],
        transitions: vec![runtime_contract::behavior::Transition::new(
            "confirm",
            "Pending",
            "Confirmed",
        )
        .with_guard("paid == true")],
        initial_state: "Pending".to_string(),
    }];
    ext
}

#[test]
fn test_before_update_state_machine_valid_transition() {
    let surface = ExtensionSurface::from_extension(make_state_machine_extension("test-app"));

    // 当前状态: Pending, 更新请求: t_state = Confirmed, event = confirm
    let mut new_vars = vars(&[
        ("t_state", serde_json::json!("Confirmed")),
        ("event", serde_json::json!("confirm")),
    ]);
    let current_vars = vars(&[("t_state", serde_json::json!("Pending"))]);

    let result = surface
        .update("Order", &mut new_vars, &current_vars)
        .unwrap();
    assert!(result.all_passed, "Pending → Confirmed 应通过状态机验证");
}

#[test]
fn test_before_update_state_machine_invalid_transition() {
    let surface = ExtensionSurface::from_extension(make_state_machine_extension("test-app"));

    // 当前状态: Pending, 更新请求: t_state = Delivered (跳过两步，不允许)
    let mut new_vars = vars(&[
        ("t_state", serde_json::json!("Delivered")),
        ("event", serde_json::json!("deliver")),
    ]);
    let current_vars = vars(&[("t_state", serde_json::json!("Pending"))]);

    let result = surface
        .update("Order", &mut new_vars, &current_vars)
        .unwrap();
    assert!(!result.all_passed, "Pending → Delivered 应被状态机阻止");
}

#[test]
fn test_before_update_state_machine_no_state_change() {
    let surface = ExtensionSurface::from_extension(make_state_machine_extension("test-app"));

    // 状态未变化（Pending → Pending），不应触发转换验证
    let mut new_vars = vars(&[
        ("t_state", serde_json::json!("Pending")),
        ("notice", serde_json::json!("更新备注")),
    ]);
    let current_vars = vars(&[
        ("t_state", serde_json::json!("Pending")),
        ("notice", serde_json::json!("旧备注")),
    ]);

    let result = surface
        .update("Order", &mut new_vars, &current_vars)
        .unwrap();
    assert!(result.all_passed, "状态未变化时应通过");
}

#[test]
fn test_before_update_no_state_machine_defined() {
    // 未定义状态机的实体，before_update 不应报错
    let surface = test_surface("test-app"); // 只有约束/规则，无状态机

    let mut new_vars = vars(&[
        ("name", serde_json::json!("Acme")),
        ("code", serde_json::json!("C001")),
        ("public", serde_json::json!(false)),
        ("_f_", serde_json::json!("company")),
    ]);
    let current_vars = vars(&[
        ("name", serde_json::json!("Old Name")),
        ("code", serde_json::json!("C001")),
    ]);

    let result = surface
        .update("Subject", &mut new_vars, &current_vars)
        .unwrap();
    assert!(result.all_passed, "未定义状态机的实体更新应通过");
}

#[test]
fn test_before_update_constraint_still_works() {
    // 即使传了 current_variables，约束验证仍应对 new_variables 生效
    let surface = test_surface("test-app");

    let mut new_vars = vars(&[
        ("name", serde_json::json!("")), // 空名称违反约束
        ("public", serde_json::json!(true)),
    ]);
    let current_vars = vars(&[
        ("name", serde_json::json!("Old Name")),
        ("public", serde_json::json!(true)),
    ]);

    let result = surface
        .update("Subject", &mut new_vars, &current_vars)
        .unwrap();
    assert!(!result.all_passed, "空名称应违反约束");
}

#[test]
fn test_on_transition_guard_blocks_and_allows() {
    let surface = ExtensionSurface::from_extension(make_guarded_state_machine_ext("test-app"));

    // guard 为假 → 转换被拒
    let mut vars_unpaid = vars(&[("paid", serde_json::json!(false))]);
    let blocked = surface
        .transition("Order", "Pending", "Confirmed", &mut vars_unpaid)
        .unwrap();
    assert!(!blocked.all_passed, "未付款时 guard 应拒绝转换");

    // guard 为真 → 转换放行
    let mut vars_paid = vars(&[("paid", serde_json::json!(true))]);
    let allowed = surface
        .transition("Order", "Pending", "Confirmed", &mut vars_paid)
        .unwrap();
    assert!(allowed.all_passed, "已付款时 guard 应放行转换");
}

#[test]
fn test_surface_inventory_enumerates_state_machine_transitions() {
    let surface = ExtensionSurface::from_extension(make_state_machine_extension("test-app"));
    let inv = surface.inventory();

    assert_eq!(inv.state_machines.len(), 1);
    let sm = &inv.state_machines[0];
    assert_eq!(sm.state_field, "t_state");
    assert_eq!(sm.states.len(), 5, "应枚举 5 个状态");
    assert_eq!(sm.transitions.len(), 4, "应枚举 4 条迁移（覆盖单元）");
    assert_eq!(inv.total(), 4, "无约束/规则时覆盖单元 = 迁移数");
    assert_eq!(sm.transitions[3].from.len(), 2, "多源迁移应保留全部来源");
    assert!(!sm.transitions[0].has_guard, "无 guard 迁移应标记 false");
}

#[test]
fn test_surface_inventory_marks_guarded_transition() {
    let surface = ExtensionSurface::from_extension(make_guarded_state_machine_ext("test-app"));
    let sm = &surface.inventory().state_machines[0];
    assert!(sm.transitions[0].has_guard, "带 guard 迁移应标记 true");
}
