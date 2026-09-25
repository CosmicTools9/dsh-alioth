//! 扩展 YAML 表达式的**文法契约**回归（change `unify-expression-dsl-on-rhai`）
//!
//! 判据（正本：`docs/specs/ALIOTH_ONTOLOGY_SPEC.md` §四 作者面文法）：
//! `Pre-Proc/*/Apps/*/extensions/{constraints,rules,statemachines,workflows}.yaml`
//! 中承载**表达式**的字段 MUST 通过**上下文无关**校验层 = `validate_functions`
//! （语法 + 自由函数白名单）。标识符层（引用 ⊆ 实体字段集）在本 crate 不可达——
//! 它需要 plan entity ↔ 本体 domain 映射（`Meta/backend/app-agent/src/domain_keys.rs`），
//! 而 Framework 不得依赖 Meta；组装期标识符层校验在 composer 侧落地。
//!
//! 实现口径（MUST）：
//! - 用标准解析器读 YAML（`yaml_serde` → `runtime_contract` 扩展类型），MUST NOT 手剥
//!   标量（转义/`null`/块标量都会错判）；
//! - 规则动作为「`字段 = 表达式`」契约 ⇒ 仅校验右侧表达式；
//! - 无法判定为表达式的字段（状态机 `guard`/`action` 的函数名、`WorkflowAction` 的
//!   映射类变体）保守跳过。

use runtime_contract::extension::{
    ConstraintExtension, RuleExtension, StateMachineExtension, WorkflowAction, WorkflowDefinition,
};
use runtime_engine::RhaiExpressionEngine;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repo root")
}

/// 递归收集扩展 YAML；跳过构建/依赖/备份目录（`Pre-Proc/**/target` 体量巨大）。
fn collect_yaml(dir: &Path, out: &mut Vec<PathBuf>) {
    const SKIP_DIRS: [&str; 6] = [
        "target",
        "node_modules",
        "dist",
        ".git",
        "Backup",
        ".backups",
    ];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            collect_yaml(&path, out);
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".yaml") {
                out.push(path);
            }
        }
    }
}

/// `字段 = 表达式` 契约：取右侧表达式（无 `=` 或左侧不像标识符 → None=跳过）
fn action_expr(action: &str) -> Option<&str> {
    let (lhs, rhs) = action.split_once('=')?;
    let lhs = lhs.trim();
    let rhs = rhs.trim();
    if rhs.is_empty() || lhs.is_empty() {
        return None;
    }
    let is_ident = lhs
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    is_ident.then_some(rhs)
}

/// 从一个扩展 YAML 文件按其类型抽取 (标签, 表达式) 列表；解析失败即 Err（文件本身不合法）
fn expressions_of(path: &Path, text: &str) -> Result<Vec<(String, String)>, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let mut out: Vec<(String, String)> = Vec::new();
    match name {
        "constraints.yaml" => {
            for (i, c) in yaml_serde::from_str::<Vec<ConstraintExtension>>(text)
                .map_err(|e| e.to_string())?
                .into_iter()
                .enumerate()
            {
                out.push((format!("constraint[{i}].expression"), c.expression));
            }
        }
        "rules.yaml" => {
            for (i, r) in yaml_serde::from_str::<Vec<RuleExtension>>(text)
                .map_err(|e| e.to_string())?
                .into_iter()
                .enumerate()
            {
                out.push((format!("rule[{i}].condition"), r.condition));
                if let Some(rhs) = action_expr(&r.action) {
                    out.push((format!("rule[{i}].action(rhs)"), rhs.to_string()));
                }
            }
        }
        "statemachines.yaml" => {
            for (i, sm) in yaml_serde::from_str::<Vec<StateMachineExtension>>(text)
                .map_err(|e| e.to_string())?
                .into_iter()
                .enumerate()
            {
                for (j, t) in sm.transitions.iter().enumerate() {
                    // guard/action 可为「函数名」而非表达式：语法层校验对裸标识符同样通过
                    if let Some(g) = t.guard.as_deref() {
                        out.push((
                            format!("statemachine[{i}].transition[{j}].guard"),
                            g.to_string(),
                        ));
                    }
                    if let Some(a) = t.action.as_deref() {
                        let expr = action_expr(a).unwrap_or(a).to_string();
                        out.push((format!("statemachine[{i}].transition[{j}].action"), expr));
                    }
                }
            }
        }
        "workflows.yaml" => {
            for (i, wf) in yaml_serde::from_str::<Vec<WorkflowDefinition>>(text)
                .map_err(|e| e.to_string())?
                .into_iter()
                .enumerate()
            {
                if let Some(c) = wf.trigger.condition.as_deref() {
                    out.push((format!("workflow[{i}].trigger.condition"), c.to_string()));
                }
                for (j, step) in wf.steps.iter().enumerate() {
                    if let Some(c) = step.condition.as_deref() {
                        out.push((format!("workflow[{i}].step[{j}].condition"), c.to_string()));
                    }
                    if let WorkflowAction::SetField { expression, .. } = &step.action {
                        out.push((
                            format!("workflow[{i}].step[{j}].action.expression"),
                            expression.clone(),
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

#[test]
fn extension_yaml_expressions_are_valid_rhai() {
    let root = repo_root().join("Pre-Proc");
    let mut files = Vec::new();
    collect_yaml(&root, &mut files);
    assert!(
        !files.is_empty(),
        "Pre-Proc 下未找到扩展 YAML（路径解析错误？）"
    );

    let eng = RhaiExpressionEngine::new();
    let mut checked = 0usize;
    let mut skipped_files = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for file in &files {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if ![
            "constraints.yaml",
            "rules.yaml",
            "statemachines.yaml",
            "workflows.yaml",
        ]
        .contains(&name)
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        if text.trim().is_empty() {
            skipped_files += 1;
            continue;
        }
        if let Err(e) = expressions_of(file, &text) {
            failures.push(format!(
                "{} — YAML 解析失败: {e}",
                file.strip_prefix(&root).unwrap_or(file).display()
            ));
            continue;
        }
        let Ok(exprs) = expressions_of(file, &text) else {
            continue;
        };
        for (label, expr) in exprs {
            if expr.trim().is_empty() {
                continue;
            }
            checked += 1;
            if let Err(e) = eng.validate_functions(&expr) {
                failures.push(format!(
                    "{} #{label} — {expr}\n      {e}",
                    file.strip_prefix(&root).unwrap_or(file).display()
                ));
            }
        }
    }

    println!(
        "extension_yaml_dialect: 检查 {checked} 条表达式 / 跳过空文件 {skipped_files} / 失败 {}",
        failures.len()
    );
    assert!(checked > 0, "未抽取到任何表达式（抽取器失效？）");
    assert!(
        failures.is_empty(),
        "{} 条表达式不是合法 Rhai（共检查 {} 条，跳过空文件 {}）：\n  - {}",
        failures.len(),
        checked,
        skipped_files,
        failures.join("\n  - ")
    );
}

/// §4 存量样本端到端验收（change `add-runtime-rule-inference`）：
/// ct-git「提交差异统计」是 R7（连字符键裸用）/ R8（未注册自由函数）两类盲区的**首个实例**，
/// 迁移后 MUST 同时满足：① `validate_all`（语法 + 标识符 + 自由函数白名单）通过；
/// ② 夹具 ctx 下求值得到期望聚合；③ 缺子集合键时走防御分支取 0，而非求值错误。
#[test]
fn ct_git_rules_action_is_evaluable() {
    use std::collections::HashMap;

    let path = repo_root().join("Pre-Proc/Cosmic-Tools/Apps/ct-git/extensions/rules.yaml");
    if !path.is_file() {
        eprintln!("跳过：{} 不存在", path.display());
        return;
    }
    let raw = std::fs::read_to_string(&path).expect("读 rules.yaml");
    let rules: Vec<RuleExtension> = yaml_serde::from_str(&raw).expect("解析 rules.yaml");
    let rule = rules
        .iter()
        .find(|r| r.name == "提交差异统计")
        .expect("应含「提交差异统计」规则");

    // ① 全量校验：已知键 = 该规则依赖的子集合键
    let known = vec!["zc_id_prod-file".to_string()];
    let eng = RhaiExpressionEngine::new();
    eng.validate_all(&rule.condition, &known)
        .expect("condition SHOULD 通过全量校验");
    let segments: Vec<&str> = rule.action.split(';').collect();
    assert_eq!(segments.len(), 2, "多重赋值契约：两段（总量/删除量）");
    for seg in &segments {
        let (_, rhs) = seg.split_once('=').expect("段契约 `字段 = 表达式`");
        eng.validate_all(rhs.trim(), &known)
            .unwrap_or_else(|e| panic!("action 段全量校验失败（{seg}）: {e}"));
    }
    // 回归锚点：R7/R8 两类盲区形态 MUST NOT 再出现
    assert!(
        !rule.action.contains("zc_id_prod-file."),
        "连字符键 MUST NOT 以裸属性路径书写（会被解析为减法）"
    );
    assert!(
        !rule.action.contains("sum(zc_id_prod"),
        "未注册自由函数形态 MUST NOT 回归"
    );

    // ② 夹具 ctx 求值
    let mut vars: HashMap<String, serde_json::Value> = HashMap::new();
    vars.insert(
        "zc_id_prod-file".to_string(),
        serde_json::json!([
            {"additions": 3, "deletions": 1},
            {"additions": 4, "deletions": 2}
        ]),
    );
    let (_, first_rhs) = segments[0].split_once('=').unwrap();
    let (_, second_rhs) = segments[1].split_once('=').unwrap();
    assert_eq!(
        eng.evaluate(first_rhs.trim(), &vars).unwrap(),
        serde_json::json!(7),
        "total_additions 应为子集合 additions 之和"
    );
    assert_eq!(
        eng.evaluate(second_rhs.trim(), &vars).unwrap(),
        serde_json::json!(3),
        "total_deletions 应为子集合 deletions 之和"
    );

    // ③ 缺子集合键 ⇒ 防御分支取 0（而非类型错）
    let empty: HashMap<String, serde_json::Value> = HashMap::new();
    assert_eq!(
        eng.evaluate(first_rhs.trim(), &empty).unwrap(),
        serde_json::json!(0)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// §Plan 面表达式合法性（plan ↔ 产物同方言；判据键集 = 组合期 `validate_extension_expressions`
// 所消费的键：`constraints[].expression` / `business_rules[].condition` / `business_rules[].action`）
//
// 为什么需要：plan 面（`flow-plan.json`）曾是判据盲区——含 `implies`/`null`/SQL 语法的表达式
// 在组合期才被 fail-closed 拦下，长期不可见（2026-09-23 审计实测 ct-bv-local / wz-yy-wms）。
// `core_constraints[]`（自然语言描述）与 `computations[].formula`（SQL 承载）**不属本面**。
// ─────────────────────────────────────────────────────────────────────────────

/// 收集全部 LIVE plan 文件：`Pre-Proc/{ns}/Apps/{app}/flow-plan.json`（不递归、不含快照副本）
fn collect_flow_plans(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(ns_dirs) = std::fs::read_dir(root.join("Pre-Proc")) else {
        return out;
    };
    for ns in ns_dirs.flatten() {
        let Ok(app_dirs) = std::fs::read_dir(ns.path().join("Apps")) else {
            continue;
        };
        for app in app_dirs.flatten() {
            let plan = app.path().join("flow-plan.json");
            if plan.is_file() {
                out.push(plan);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn plan_face_expressions_are_valid_rhai() {
    let eng = RhaiExpressionEngine::new();
    let root = repo_root();
    let files = collect_flow_plans(&root);
    // 2026-09-24 namespace 拆分后：flow-plan.json 属 ns 面（`Pre-Proc/{ns}/Apps/*/flow-plan.json`），
    // 平台仓不持有（开发机为指向 ns 仓的符号链接、干净检出中不存在）⇒ 收集为空是**预期状态**，
    // 跳过而非失败（判据面随 ns 迁出；该测试在持有 ns 树的场景仍照常校验）。
    if files.is_empty() {
        eprintln!("⏭ plan_face_expressions_are_valid_rhai：未定位到任何 flow-plan.json（平台仓不持有 ns 树）—— 跳过");
        return;
    }

    // 存量归线（ratchet 只减不增）：违规必须 ⊆ 基线；已修复的基线条目未删除 ⇒ stale 失败。
    let baseline_path = root.join("scripts/check/baselines/plan-face-dialect.json");
    let allowed: Vec<String> = std::fs::read_to_string(&baseline_path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| {
            v.get("allowed").and_then(|a| a.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|e| e.get("key").and_then(|k| k.as_str()).map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default();

    let mut checked = 0usize;
    let mut violations: Vec<(String, String)> = Vec::new();
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .display()
            .to_string();
        let plan: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => {
                violations.push((format!("{rel}#<json>"), "JSON 解析失败".to_string()));
                continue;
            }
        };
        // 键集唯一实现 = `runtime_engine::plan_face_expressions`（与提案写径共用）
        for (label, expr) in runtime_engine::plan_face_expressions(&plan) {
            if expr.trim().is_empty() {
                continue;
            }
            checked += 1;
            if let Err(e) = eng.validate_functions(&expr) {
                violations.push((format!("{rel}#{label}"), format!("{expr}\n      {e}")));
            }
        }
    }

    let unexpected: Vec<&(String, String)> = violations
        .iter()
        .filter(|(k, _)| !allowed.contains(k))
        .collect();
    let stale: Vec<&String> = allowed
        .iter()
        .filter(|k| !violations.iter().any(|(v, _)| v == *k))
        .collect();

    println!(
        "plan_face_dialect: 检查 {} 文件 / {checked} 条表达式 / 违规 {} / 基线 {} / 新增 {} / stale {}",
        files.len(),
        violations.len(),
        allowed.len(),
        unexpected.len(),
        stale.len()
    );
    assert!(
        unexpected.is_empty(),
        "{} 条 plan 面表达式不是合法 Rhai 且未在基线内：\n  - {}",
        unexpected.len(),
        unexpected
            .iter()
            .map(|(k, d)| format!("{k} — {d}"))
            .collect::<Vec<_>>()
            .join("\n  - ")
    );
    assert!(
        stale.is_empty(),
        "基线 stale（已修复的条目 MUST 从 scripts/check/baselines/plan-face-dialect.json 删除）：{:?}",
        stale
    );
}
