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
  expression: "name != () && name != \"\""
  level: Error
  message: "客户名称不能为空"

- entity: Subject
  field: code
  expression: "code == () || code == \"\" || code != \"INVALID\""
  level: Error
  message: "客户编码不能为 INVALID"

- entity: Subject
  field: null
  expression: "ctx[\"public\"] != true || name != ()"
  level: Error
  message: "公开客户必须填写名称"

- entity: Subject
  field: null
  expression: "_f_ in [\"personal\", \"company\", \"government\"] || _f_ == () || _f_ == \"\""
  level: Error
  message: "业务形态必须是 personal、company、government 或留空"
"#;

/// rules.yaml 的内容（内嵌以保持测试自包含）
const RULES_YAML: &str = r#"
- entity: Subject
  name: auto_company_for_public
  trigger: onCreate
  condition: "ctx[\"public\"] == true && (_f_ == () || _f_ == \"\")"
  action: "_f_ = \"company\""
  priority: 100
  error_message: "公开客户自动设为公司形态"
  blocking: false

- entity: Subject
  name: default_name_from_code
  trigger: onCreate
  condition: "(name == () || name == \"\") && code != () && code != \"\""
  action: "name = code"
  priority: 200
  error_message: "已根据编码自动填充名称"
  blocking: false

- entity: Subject
  name: block_test_code
  trigger: onCreate
  condition: "code == \"test\""
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
    assert!(
        result.all_passed,
        "名称非空应通过; errors={:?}",
        result.blocking_errors
    );
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

/// 带 transition action 的状态机（`字段 = 表达式` 契约，含多重赋值与带 guard 的一条）
fn make_action_state_machine_ext(app_code: &str) -> runtime_engine::AppLogicExtension {
    use runtime_contract::extension::StateMachineExtension;

    let mut ext = runtime_engine::AppLogicExtension::new(app_code);
    ext.state_machines = vec![StateMachineExtension {
        entity: "Order".to_string(),
        state_field: "t_state".to_string(),
        states: vec![
            runtime_contract::behavior::State::new("Pending"),
            runtime_contract::behavior::State::new("Confirmed"),
            runtime_contract::behavior::State::new("Cancelled"),
        ],
        transitions: vec![
            runtime_contract::behavior::Transition::new("confirm", "Pending", "Confirmed")
                .with_action("grade = \"B\"; seq = 1; seq = seq + 41"),
            runtime_contract::behavior::Transition {
                event: "cancel".to_string(),
                from: vec!["Pending".to_string()],
                to: "Cancelled".to_string(),
                guard: Some("paid == true".to_string()),
                action: Some("grade = \"C\"".to_string()),
                is_default: false,
            },
        ],
        initial_state: "Pending".to_string(),
    }];
    ext
}

#[test]
fn test_before_update_transition_action_writes_fields() {
    let surface = ExtensionSurface::from_extension(make_action_state_machine_ext("test-app"));

    let mut new_vars = vars(&[
        ("t_state", serde_json::json!("Confirmed")),
        ("event", serde_json::json!("confirm")),
    ]);
    let current_vars = vars(&[("t_state", serde_json::json!("Pending"))]);

    let result = surface
        .update("Order", &mut new_vars, &current_vars)
        .unwrap();

    assert!(result.all_passed, "无 guard 的转换应通过");
    assert_eq!(
        result.mutations.get("grade"),
        Some(&serde_json::json!("B")),
        "transition action MUST 产出字段 mutation：{:?}",
        result.mutations
    );
    assert_eq!(
        result.mutations.get("seq"),
        Some(&serde_json::json!(42)),
        "多重赋值按序执行且后者可见前者结果：{:?}",
        result.mutations
    );
    assert_eq!(
        new_vars.get("grade"),
        Some(&serde_json::json!("B")),
        "action 结果 MUST 就地写入变量面（同一次 update 的后续规则可见）"
    );
    assert!(
        !result.mutations.contains_key("transition_action_result"),
        "MUST NOT 再产出伪 mutation 键：{:?}",
        result.mutations
    );
}

#[test]
fn test_before_update_transition_action_skipped_when_guard_blocks() {
    let surface = ExtensionSurface::from_extension(make_action_state_machine_ext("test-app"));

    let mut new_vars = vars(&[
        ("t_state", serde_json::json!("Cancelled")),
        ("event", serde_json::json!("cancel")),
        ("paid", serde_json::json!(false)),
    ]);
    let current_vars = vars(&[
        ("t_state", serde_json::json!("Pending")),
        ("paid", serde_json::json!(false)),
    ]);

    let result = surface
        .update("Order", &mut new_vars, &current_vars)
        .unwrap();

    assert!(!result.all_passed, "guard 未过应拒绝转换");
    assert!(
        result.mutations.is_empty(),
        "guard 未过 MUST NOT 执行 action：{:?}",
        result.mutations
    );
}

#[test]
fn test_before_update_transition_action_failure_blocks_write() {
    // 语法合法（加载期校验通过）但运行期引用不存在的变量 ⇒ MUST fail-closed（不得静默跳过）
    let mut ext = make_action_state_machine_ext("test-app");
    ext.state_machines[0].transitions[0].action = Some("grade = ghost_field".to_string());
    let surface = ExtensionSurface::from_extension(ext);

    let mut new_vars = vars(&[
        ("t_state", serde_json::json!("Confirmed")),
        ("event", serde_json::json!("confirm")),
    ]);
    let current_vars = vars(&[("t_state", serde_json::json!("Pending"))]);

    let result = surface
        .update("Order", &mut new_vars, &current_vars)
        .unwrap();

    assert!(!result.all_passed, "action 求值失败 MUST 阻断");
    assert!(
        result
            .blocking_errors
            .iter()
            .any(|e| e.contains("转换动作求值失败")),
        "阻塞原因应指名 action 求值：{:?}",
        result.blocking_errors
    );
}

#[test]
fn test_before_update_transition_without_action_produces_no_mutation() {
    let surface = ExtensionSurface::from_extension(make_state_machine_extension("test-app"));

    let mut new_vars = vars(&[
        ("t_state", serde_json::json!("Confirmed")),
        ("event", serde_json::json!("confirm")),
    ]);
    let current_vars = vars(&[("t_state", serde_json::json!("Pending"))]);

    let result = surface
        .update("Order", &mut new_vars, &current_vars)
        .unwrap();

    assert!(result.all_passed, "无 action 的合法转换应通过");
    assert!(
        result.mutations.is_empty(),
        "无 action MUST NOT 产生 mutation：{:?}",
        result.mutations
    );
}

#[test]
fn test_on_transition_executes_action_with_same_contract() {
    let surface = ExtensionSurface::from_extension(make_action_state_machine_ext("test-app"));

    let mut vars_paid = vars(&[("paid", serde_json::json!(true))]);
    let allowed = surface
        .transition("Order", "Pending", "Cancelled", &mut vars_paid)
        .unwrap();

    assert!(allowed.all_passed, "guard 通过应放行转换");
    assert_eq!(
        allowed.mutations.get("grade"),
        Some(&serde_json::json!("C")),
        "on_transition MUST 按 `字段 = 表达式` 产出字段 mutation：{:?}",
        allowed.mutations
    );
    assert_eq!(
        vars_paid.get("grade"),
        Some(&serde_json::json!("C")),
        "变量面 MUST 同步写入"
    );
    assert!(
        !allowed.mutations.contains_key("transition_action_result"),
        "MUST NOT 保留伪 mutation 键：{:?}",
        allowed.mutations
    );
}

/// 删除期钩子的求值上下文 = **存量行**（不是 `{id}`）
///
/// 实据（`Pre-Proc/WZ/Apps/wz-yy-wms/extensions/constraints.yaml` 原样摘录）：Error 级约束引用 `qty` /
/// `fk_subject` / `fk_object`。删除期若只给 `{id}`，Rhai 求值「变量不存在」⇒ 求值失败 ⇒ 按 `level`
/// 处理（Error ⇒ 阻断）⇒ **该实体的删除被恒阻断**。写径必须装配行上下文（见 crud `ext_before_delete`）。
#[test]
fn delete_hook_context_must_carry_existing_row() {
    let constraints_yaml = r#"
- entity: zc_id_orde-storage
  field: null
  expression: qty > 0
  level: Error
  message: "交换数量为正 violated"
- entity: zc_id_orde-storage
  field: null
  expression: fk_subject != fk_object
  level: Error
  message: "出入方不得相同 violated"
"#;
    let constraints: Vec<runtime_contract::extension::ConstraintExtension> =
        yaml_serde::from_str(constraints_yaml).expect("无效的 constraints.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new("wz-yy-wms");
    ext.constraints = constraints;
    let surface = ExtensionSurface::from_extension(ext);

    // ① 仅 id：约束不可求值 ⇒ 阻断（引擎口径；故写径 MUST NOT 只给 id）
    let mut id_only = vars(&[("id", serde_json::json!(7))]);
    let blocked = surface.delete("zc_id_orde-storage", &mut id_only).unwrap();
    assert!(
        !blocked.all_passed,
        "仅 id 上下文 ⇒ Error 级约束求值失败 ⇒ 阻断（写径须避开该上下文）"
    );

    // ② 存量行：约束可真求值 ⇒ 放行
    let mut with_row = vars(&[
        ("id", serde_json::json!(7)),
        ("qty", serde_json::json!(5)),
        ("fk_subject", serde_json::json!(1)),
        ("fk_object", serde_json::json!(2)),
    ]);
    let allowed = surface.delete("zc_id_orde-storage", &mut with_row).unwrap();
    assert!(
        allowed.all_passed,
        "存量行上下文 ⇒ 约束可求值 ⇒ 放行：{:?}",
        allowed.blocking_errors
    );
}

/// 更新期上下文 = **存量行 ⊕ 本次提交**（提交值覆盖）
///
/// 判据：约束引用**未随本次 update 提交**的字段（PATCH 语义）⇒ 必须由存量行补齐，否则求值失败
/// （求值失败按 level 处理 ⇒ Error 级即阻断）⇒ 部分字段更新被误 400。
#[test]
fn update_context_merges_existing_row_with_submitted_values() {
    let surface = test_surface("test-app"); // 约束：name 非空 / code != INVALID / public → name 必填

    // ① 仅提交 public，存量行提供 name/code/_f_ ⇒ 约束可求值 ⇒ 放行（修复前：name 缺失 ⇒ 求值失败 ⇒ 阻断）
    let mut new_vars = vars(&[("public", serde_json::json!(false))]);
    let current_vars = vars(&[
        ("name", serde_json::json!("Acme")),
        ("code", serde_json::json!("C001")),
        ("_f_", serde_json::json!("company")),
    ]);
    let merged = surface
        .update("Subject", &mut new_vars, &current_vars)
        .unwrap();
    assert!(
        merged.all_passed,
        "未提交字段 MUST 由存量行补齐：{:?}",
        merged.blocking_errors
    );

    // ② 存量行自身违反约束 ⇒ 必须仍然阻断（证明存量行**真的参与**求值，而非一律放行）
    let mut new_vars = vars(&[("public", serde_json::json!(false))]);
    let current_vars = vars(&[
        ("name", serde_json::json!("")),
        ("_f_", serde_json::json!("company")),
    ]);
    let blocked = surface
        .update("Subject", &mut new_vars, &current_vars)
        .unwrap();
    assert!(!blocked.all_passed, "存量行的违规字段 MUST 参与求值 ⇒ 阻断");

    // ③ 提交值覆盖存量值（约束看的是「写完成后的行状态」）
    let mut new_vars = vars(&[
        ("name", serde_json::json!("")),
        ("code", serde_json::json!("C001")),
    ]);
    let current_vars = vars(&[
        ("name", serde_json::json!("Acme")),
        ("_f_", serde_json::json!("company")),
    ]);
    let overwritten = surface
        .update("Subject", &mut new_vars, &current_vars)
        .unwrap();
    assert!(
        !overwritten.all_passed,
        "提交值 MUST 覆盖存量值（空 name 提交 ⇒ 阻断）"
    );
}

/// 真实产物锚定：AVIC `requirement` 的扩展声明 MUST 在引擎面可执行
///
/// 背景（2026-09-23）：该 app 的 `constraints.yaml`/`rules.yaml` 原用 `title` / `fk_subj_provider`
/// 引用**实体不存在的字段** ⇒ 在 Rhai 侧「变量不存在」⇒ 求值失败 ⇒ Error 级判违规 ⇒
/// **合法载荷也会被拒**（且该实体写径当时是手写 handler，声明从不执行 ⇒ 缺陷静默）。
/// 本用例直接加载**真实目录**：修正后合法载荷 MUST 放行、`name-trim` MUST 回写、越界 MUST 阻断。
#[test]
fn avic_requirement_declarations_are_executable() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../Pre-Proc/AVIC-CAASEC/Apps/ai-98ab565fc9610cfd/extensions");
    let surface =
        ExtensionSurface::from_dir("ai-98ab565fc9610cfd", &dir).expect("加载 AVIC 扩展目录");

    // 上下文形态 = DTO serde 视图：`Option` ⇒ null（键在）、i64 ⇒ 字符串（ID_JSON_PRECISION）
    // ① 合法载荷：约束全部可求值 ⇒ 放行；`name-trim` 回写 trim 后的 name
    let mut input = vars(&[
        ("code", serde_json::json!("REQ-001")),
        ("name", serde_json::json!("  需求标题  ")),
        ("priority", serde_json::json!("3")),
        ("fk_subj_demand", serde_json::Value::Null),
        ("subj_provider", serde_json::Value::Null),
        ("process", serde_json::Value::Null),
    ]);
    let ok = surface.create("requirement", &mut input).unwrap();
    assert!(
        ok.all_passed,
        "合法载荷 MUST 放行（修正前 `title` 求值失败 ⇒ 阻断）：{:?}",
        ok.blocking_errors
    );
    assert_eq!(
        ok.mutations.get("name"),
        Some(&serde_json::json!("需求标题")),
        "`name-trim` MUST 回写 trim 后的 name：{:?}",
        ok.mutations
    );

    // ② code 为空 ⇒ 阻断（约束真在跑）
    let mut input = vars(&[
        ("code", serde_json::json!("")),
        ("name", serde_json::json!("需求标题")),
        ("priority", serde_json::Value::Null),
    ]);
    let blocked = surface.create("requirement", &mut input).unwrap();
    assert!(
        blocked
            .blocking_errors
            .iter()
            .any(|e| e.contains("需求编号")),
        "空 code MUST 阻断：{:?}",
        blocked.blocking_errors
    );

    // ③ name 长度越界 ⇒ 阻断（原 `title` 口径下该判据永不生效）
    let mut input = vars(&[
        ("code", serde_json::json!("REQ-002")),
        ("name", serde_json::json!("A")),
        ("priority", serde_json::Value::Null),
    ]);
    let blocked = surface.create("requirement", &mut input).unwrap();
    assert!(
        blocked.blocking_errors.iter().any(|e| e.contains("2-200")),
        "1 字符标题 MUST 阻断：{:?}",
        blocked.blocking_errors
    );

    // ④ PATCH 语义：只提交 comments，存量行提供 code/name ⇒ MUST NOT 误阻断
    //    （存量行上下文 = 实体 serde 视图 ⇒ 键齐全；`X != ()` 类条件依赖键存在）
    let mut input = vars(&[("comments", serde_json::json!("补充说明"))]);
    let current = vars(&[
        ("code", serde_json::json!("REQ-003")),
        ("name", serde_json::json!("存量标题")),
        ("priority", serde_json::Value::Null),
        ("process", serde_json::Value::Null),
        ("fk_subj_demand", serde_json::Value::Null),
        ("subj_provider", serde_json::Value::Null),
    ]);
    let patched = surface.update("requirement", &mut input, &current).unwrap();
    assert!(
        patched.all_passed,
        "部分字段更新 MUST 由存量行补齐：{:?}",
        patched.blocking_errors
    );
}

/// 约束 `level` 语义：`error` 阻断写径，`warning` **留痕但不阻断**
///
/// 历史缺陷：任何未通过都置 `all_passed = false` ⇒ warning 级也阻断（level 语义丢失）。
#[test]
fn warning_level_constraint_does_not_block_writes() {
    let yaml = r#"
- entity: Order
  field: priority
  expression: "priority == () || (priority >= 1 && priority <= 5)"
  level: Warning
  message: "优先级超出 1-5 星范围"
"#;
    let constraints: Vec<runtime_contract::extension::ConstraintExtension> =
        yaml_serde::from_str(yaml).expect("无效的 constraints.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new("warn-app");
    ext.constraints = constraints;
    let surface = ExtensionSurface::from_extension(ext);

    // ① warning 未通过（越界）⇒ 不阻断
    let mut input = vars(&[("priority", serde_json::json!(9))]);
    let passed = surface.create("Order", &mut input).unwrap();
    assert!(
        passed.all_passed,
        "warning 级未通过 MUST NOT 阻断写径：{:?}",
        passed.blocking_errors
    );
    assert!(
        passed.blocking_errors.is_empty(),
        "warning 文案 MUST NOT 进入阻断错误列表：{:?}",
        passed.blocking_errors
    );

    // ② 同表达式升为 error ⇒ 阻断（同一判据，仅 level 不同）
    let yaml = yaml.replace("level: Warning", "level: Error");
    let constraints: Vec<runtime_contract::extension::ConstraintExtension> =
        yaml_serde::from_str(&yaml).expect("无效的 constraints.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new("warn-app");
    ext.constraints = constraints;
    let surface = ExtensionSurface::from_extension(ext);
    let mut input = vars(&[("priority", serde_json::json!(9))]);
    let blocked = surface.create("Order", &mut input).unwrap();
    assert!(
        !blocked.all_passed && blocked.blocking_errors.iter().any(|e| e.contains("1-5")),
        "error 级未通过 MUST 阻断：{:?}",
        blocked.blocking_errors
    );
}

#[test]
fn violations_flag_action_segments_without_assignment() {
    // 动作契约 = `字段 = 表达式`：段缺 `=`（内嵌 `let …; x.y()` 语句序列）在运行期是
    // `Unsupported action format` ⇒ 加载期 MUST 报违规（AVIC `requirement` 原 `title-trim`
    // 动作 `title = let t = title; t.trim(); t` 即此形态：加载期静默、接线后运行期炸）
    let rules_yaml = r#"
- entity: Requirement
  name: title-trim
  trigger: "OnCreate"
  condition: "name != ()"
  action: "name = let t = name; t.trim(); t"
"#;
    let rules: Vec<runtime_contract::extension::RuleExtension> =
        yaml_serde::from_str(rules_yaml).expect("无效的 rules.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new("seg-app");
    ext.business_rules = rules;

    let violations = runtime_engine::collect_expression_violations(&ext);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert!(
        violations[0].contains("business_rules[0](title-trim).action")
            && violations[0].contains("Unsupported action format"),
        "{violations:?}"
    );

    // 合法形态（纯表达式 RHS / 顶层 `;` 多赋值）MUST NOT 误报
    let ok_yaml = r#"
- entity: Requirement
  name: ok
  trigger: "OnCreate"
  condition: "name != ()"
  action: "name = name.trim(); code = code + 1"
"#;
    let rules: Vec<runtime_contract::extension::RuleExtension> =
        yaml_serde::from_str(ok_yaml).expect("无效的 rules.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new("seg-app");
    ext.business_rules = rules;
    assert!(
        runtime_engine::collect_expression_violations(&ext).is_empty(),
        "合法动作形态 MUST NOT 报违规"
    );
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

// ─────────────────────────────────────────────────────
// 测试：规则集求解语义（rule_execution.yaml → 饱和求解 / 冲突裁决）
// ─────────────────────────────────────────────────────

/// 跨规则依赖夹具：`b_reads_a`（priority 100）读 a 写 b；`a_writes_a`（priority 200）写 a。
/// 单轮顺序下 b_reads_a 先跑、条件不成立 ⇒ b 永不写入；只有依赖驱动迭代才能传播。
const RULES_CHAINED_YAML: &str = r#"
- entity: Subject
  name: b_reads_a
  trigger: onCreate
  condition: "a == 1"
  action: "b = a + 1"
  priority: 100
  blocking: false

- entity: Subject
  name: a_writes_a
  trigger: onCreate
  condition: "a == 0"
  action: "a = 1"
  priority: 200
  blocking: false
"#;

/// 同字段多写夹具（冲突裁决）
const RULES_CONFLICT_YAML: &str = r#"
- entity: Subject
  name: writer_first
  trigger: onCreate
  condition: "true"
  action: "flag = \"first\""
  priority: 100
  blocking: false

- entity: Subject
  name: writer_second
  trigger: onCreate
  condition: "true"
  action: "flag = \"second\""
  priority: 200
  blocking: false
"#;

fn surface_with_execution(
    app_code: &str,
    rules_yaml: &str,
    execution: runtime_contract::extension::RuleExecutionConfig,
) -> ExtensionSurface {
    let rules: Vec<runtime_contract::extension::RuleExtension> =
        yaml_serde::from_str(rules_yaml).expect("无效的 rules.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new(app_code);
    ext.business_rules = rules;
    ext.rule_execution = execution;
    ExtensionSurface::from_extension(ext)
}

fn default_execution(saturate: bool) -> runtime_contract::extension::RuleExecutionConfig {
    runtime_contract::extension::RuleExecutionConfig {
        saturate,
        max_rounds: None,
        on_conflict: None,
    }
}

#[test]
fn test_rule_execution_config_loaded_from_dir() {
    let dir = unique_tmp_dir("ruleexec");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rules.yaml"), RULES_CHAINED_YAML).unwrap();
    std::fs::write(
        dir.join("rule_execution.yaml"),
        "saturate: true\nmax_rounds: 3\non_conflict: last_wins\n",
    )
    .unwrap();

    let surface = ExtensionSurface::from_dir("test-app", &dir).expect("目录加载应成功");
    let cfg = surface.profile().expect("应装配到 profile").rule_execution;
    assert!(cfg.saturate, "rule_execution.yaml 的 saturate 应生效");
    assert_eq!(cfg.max_rounds, Some(3));
    assert_eq!(
        cfg.on_conflict,
        Some(runtime_contract::extension::ConflictPolicy::LastWins)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_saturation_propagates_dependencies_across_rules() {
    // 对照：未声明饱和 ⇒ 单轮顺序执行（历史语义逐字不变）
    let single =
        surface_with_execution("chain-single", RULES_CHAINED_YAML, default_execution(false));
    let mut v1 = vars(&[("a", serde_json::json!(0))]);
    let r1 = single
        .execute_rules("Subject", "onCreate", &mut v1)
        .unwrap();
    assert!(!r1.saturated, "单轮入口 MUST NOT 声称做过依赖传播");
    assert_eq!(r1.rounds, 0, "单轮 rounds 恒 0");
    assert!(!v1.contains_key("b"), "单轮不传播：{:?}", v1);
    assert!(
        runtime_engine::solve_warnings(&r1).is_empty(),
        "单轮语义不产生留痕"
    );

    // 声明饱和 ⇒ 依赖驱动不动点：a 写入后 b 于后续轮次收敛
    let saturated = surface_with_execution(
        "chain-saturated",
        RULES_CHAINED_YAML,
        default_execution(true),
    );
    let mut v2 = vars(&[("a", serde_json::json!(0))]);
    let r2 = saturated
        .execute_rules("Subject", "onCreate", &mut v2)
        .unwrap();
    assert!(r2.saturated, "应达不动点：{:?}", r2.remaining_triggers);
    assert!(r2.rounds >= 2, "跨规则依赖至少两轮：{}", r2.rounds);
    assert_eq!(v2.get("a").unwrap(), &serde_json::json!(1));
    assert_eq!(
        v2.get("b").unwrap(),
        &serde_json::json!(2),
        "依赖规则 MUST 在后续轮次生效：{:?}",
        r2.mutations
    );
    assert!(
        runtime_engine::solve_warnings(&r2).is_empty(),
        "收敛且无冲突 ⇒ 无留痕：{:?}",
        runtime_engine::solve_warnings(&r2)
    );
}

#[test]
fn test_conflict_policy_is_deterministic_and_logged() {
    let first = surface_with_execution("cf-first", RULES_CONFLICT_YAML, default_execution(true));
    let mut v1 = vars(&[]);
    let r1 = first.execute_rules("Subject", "onCreate", &mut v1).unwrap();
    assert_eq!(v1.get("flag").unwrap(), &serde_json::json!("first"));
    assert_eq!(r1.conflicts.len(), 1, "同字段两写者应留痕一条冲突");
    assert_eq!(r1.conflicts[0].winner, "writer_first");
    assert_eq!(r1.conflicts[0].field, "flag");
    assert!(
        runtime_engine::solve_warnings(&r1)
            .iter()
            .any(|w| w.contains("flag")),
        "冲突 MUST 可读留痕：{:?}",
        runtime_engine::solve_warnings(&r1)
    );

    let last = surface_with_execution(
        "cf-last",
        RULES_CONFLICT_YAML,
        runtime_contract::extension::RuleExecutionConfig {
            saturate: true,
            max_rounds: None,
            on_conflict: Some(runtime_contract::extension::ConflictPolicy::LastWins),
        },
    );
    let mut v2 = vars(&[]);
    let r2 = last.execute_rules("Subject", "onCreate", &mut v2).unwrap();
    assert_eq!(v2.get("flag").unwrap(), &serde_json::json!("second"));
    assert_eq!(r2.conflicts[0].winner, "writer_second");
}

// ─────────────────────────────────────────────────────
// 测试：表达式面非致命提示（裸标识符 guard；单一实现 = collect_expression_advisories）
// ─────────────────────────────────────────────────────

#[test]
fn advisories_flag_only_bare_identifier_guards() {
    let sm_yaml = r#"
- entity: Order
  state_field: t_state
  states:
    - name: Draft
    - name: Published
  initial_state: Draft
  transitions:
    - event: publish
      from: [Draft]
      to: Published
      guard: "version_higher_than_current"
    - event: archive
      from: [Published]
      to: Draft
      guard: "t_state == \"Published\""
"#;
    let sms: Vec<runtime_contract::extension::StateMachineExtension> =
        yaml_serde::from_str(sm_yaml).expect("无效的 statemachines.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new("advisory-app");
    ext.state_machines = sms;

    let advisories = runtime_engine::collect_expression_advisories(&ext);
    assert_eq!(
        advisories.len(),
        1,
        "仅裸标识符 guard 记提示：{advisories:?}"
    );
    assert!(
        advisories[0].contains("裸标识符") && advisories[0].contains("version_higher_than_current"),
        "{advisories:?}"
    );

    // 空扩展 ⇒ 无提示（判据不误伤）
    assert!(runtime_engine::collect_expression_advisories(
        &runtime_engine::AppLogicExtension::new("empty-app")
    )
    .is_empty());
}

#[test]
fn bare_guard_requires_injected_variable() {
    // 单一文法（短路通道已删）下裸名 guard 的语义锚点：
    // ① 注入了同名变量 ⇒ 正常求值（行为不回退）；② 未注入 ⇒ 求值错误 ⇒ 迁移 fail-closed
    //    （这正是 advisories 提示存在的理由：未注入的裸名 guard 恒阻断）
    let eng = runtime_engine::RhaiExpressionEngine::new();
    let mut vars = std::collections::HashMap::new();
    vars.insert("approved".to_string(), serde_json::json!(true));
    assert!(
        eng.evaluate_bool("approved", &vars).unwrap(),
        "注入变量时裸名 guard 应正常求值"
    );
    assert!(
        eng.evaluate("approved", &std::collections::HashMap::new())
            .is_err(),
        "未注入 ⇒ 求值错误（fail-closed 阻断来源）"
    );
}

#[test]
fn violations_flag_transition_action_expressions() {
    // transition.action 会被 on_transition 执行（DSL 缺口 #1）⇒ 加载期必须校验，
    // 否则「写错表达式 ⇒ 加载期静默、运行期炸」
    let sm_yaml = r#"
- entity: Order
  state_field: t_state
  states:
    - name: Draft
    - name: Published
  initial_state: Draft
  transitions:
    - event: publish
      from: [Draft]
      to: Published
      action: "total = (("
"#;
    let sms: Vec<runtime_contract::extension::StateMachineExtension> =
        yaml_serde::from_str(sm_yaml).expect("无效的 statemachines.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new("viol-app");
    ext.state_machines = sms;

    let violations = runtime_engine::collect_expression_violations(&ext);
    assert_eq!(
        violations.len(),
        1,
        "非法 transition.action 应被 violations 捕获：{violations:?}"
    );
    assert!(
        violations[0].contains("transitions[0].action"),
        "{violations:?}"
    );
}

#[test]
fn violations_flag_workflow_action_expression_fields() {
    // CreateRelated.field_map 值 / CallProcedure.params 元素是表达式承载字段（DSL 缺口 #2）
    // ⇒ 虽暂无执行器，加载期语法校验防死配置
    let wf_yaml = r#"
- name: auto_flow
  trigger:
    entity: Order
    event: onCreate
  steps:
    - name: link
      action:
        type: create_related
        entity: Line
        field_map:
          orderId: "(("
    - name: call
      action:
        type: call_procedure
        name: recalc
        params:
          - "amount > 0"
          - "(()"
"#;
    let wfs: Vec<runtime_contract::extension::WorkflowDefinition> =
        yaml_serde::from_str(wf_yaml).expect("无效的 workflows.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new("viol-app");
    ext.workflows = wfs;

    let violations = runtime_engine::collect_expression_violations(&ext);
    assert_eq!(
        violations.len(),
        2,
        "field_map 非法值 + params 非法元素各一条（合法 params[0] 不误报）：{violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.contains("field_map[orderId]")),
        "{violations:?}"
    );
    assert!(
        violations.iter().any(|v| v.contains("params[1]")),
        "{violations:?}"
    );
}

#[test]
fn test_max_rounds_cap_is_traced_not_silent() {
    // a 反复 +1（非幂等）⇒ 达 3 轮上限，MUST 留痕残余触发，MUST NOT 静默截断
    const RULES_NON_IDEMPOTENT: &str = r#"
- entity: Subject
  name: inc_a
  trigger: onCreate
  condition: "a < 100"
  action: "a = a + 1"
  priority: 100
  blocking: false
"#;
    let surface = surface_with_execution(
        "cap-app",
        RULES_NON_IDEMPOTENT,
        runtime_contract::extension::RuleExecutionConfig {
            saturate: true,
            max_rounds: Some(3),
            on_conflict: None,
        },
    );
    let mut v = vars(&[("a", serde_json::json!(0))]);
    let r = surface
        .execute_rules("Subject", "onCreate", &mut v)
        .unwrap();
    assert!(!r.saturated, "达上限 MUST 标记未饱和");
    assert_eq!(r.rounds, 3);
    assert_eq!(r.remaining_triggers, vec!["inc_a".to_string()]);
    assert!(
        runtime_engine::solve_warnings(&r)
            .iter()
            .any(|w| w.contains("未达不动点")),
        "达上限 MUST 可读留痕：{:?}",
        runtime_engine::solve_warnings(&r)
    );
}

// ─────────────────────────────────────────────────────
// 测试：工作流触发实体归属（判据唯一实现 = ExtensionLoader::validate_entities）
// ─────────────────────────────────────────────────────

/// 由触发实体名构造含单条工作流的扩展（经 YAML 解析，避免依赖结构体字段全集）
fn workflow_extension(app_code: &str, trigger_entity: &str) -> runtime_engine::AppLogicExtension {
    let yaml = format!(
        "- name: wf\n  trigger:\n    entity: {trigger_entity}\n    event: onCreate\n  steps: []\n"
    );
    let workflows: Vec<runtime_contract::extension::WorkflowDefinition> =
        yaml_serde::from_str(&yaml).expect("无效的 workflows.yaml");
    let mut ext = runtime_engine::AppLogicExtension::new(app_code);
    ext.workflows = workflows;
    ext
}

fn known_entities(
    pairs: &[(&str, &[&str])],
) -> std::collections::HashMap<String, std::collections::HashSet<String>> {
    pairs
        .iter()
        .map(|(name, fields)| {
            (
                name.to_string(),
                fields.iter().map(|f| f.to_string()).collect(),
            )
        })
        .collect()
}

#[test]
fn workflow_unknown_trigger_entity_is_error() {
    let ext = workflow_extension("wf-app", "TitleExchange");
    let errors = runtime_engine::ExtensionLoader::validate_entities(
        &ext,
        &known_entities(&[("zc_id_orde-storage", &["id", "qty"])]),
    );
    assert_eq!(errors.len(), 1, "应恰有一条工作流实体错误：{errors:?}");
    assert!(
        errors[0].contains("workflow 'wf'") && errors[0].contains("TitleExchange"),
        "错误文案应含 workflow 名与实体名：{}",
        errors[0]
    );
}

#[test]
fn workflow_known_trigger_entity_passes() {
    let ext = workflow_extension("wf-app", "zc_id_orde-storage");
    let errors = runtime_engine::ExtensionLoader::validate_entities(
        &ext,
        &known_entities(&[("zc_id_orde-storage", &["id", "qty"])]),
    );
    assert!(errors.is_empty(), "已知实体不得报错：{errors:?}");
}
