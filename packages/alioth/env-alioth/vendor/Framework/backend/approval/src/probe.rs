//! 路径约束探测（add-approval-flow-simulation I3）
//!
//! 无上下文符号路径分析。条件出边/决策表单元格被归一化为**字段原子约束**
//! （field op literal：数值区间 / 枚举等值），对每个节点维护**可达抽象状态集**
//! （字段约束存储），以工作列表做单调传播（状态只新增不删，去重 + 上限截断保证
//! 终止）。终局判定遵守"零假阳性"纪律：仅当某节点**全部**可达状态都使一条边
//! 矛盾时才断言该边不可达；不支持形态（OR/contains/字段间比较/解析失败）一律
//! 视为可满足，不参与证明。升级 error 仅当节点无兜底边且全部出边被证明恒不可达。
//!
//! DMN 规则可达性：规则行自身单元格内部矛盾（或与给定样例 ctx 求值 false）→
//! `DMN_RULE_NEVER_HIT`。纯函数、无 DB。

use std::collections::{HashMap, HashSet, VecDeque};

use runtime_engine::{and_atoms, CmpOp};

// ───────────────────────────── 抽象值域 ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
struct Bound {
    v: f64,
    strict: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Atom {
    Num(f64),
    Str(String),
    Bool(bool),
}

impl Atom {
    fn num_ok(&self, lo: Option<Bound>, hi: Option<Bound>) -> bool {
        let Atom::Num(v) = self else {
            return lo.is_none() && hi.is_none();
        };
        let below_lo = match lo {
            Some(b) if b.strict => *v <= b.v,
            Some(b) => *v < b.v,
            None => false,
        };
        let above_hi = match hi {
            Some(b) if b.strict => *v >= b.v,
            Some(b) => *v > b.v,
            None => false,
        };
        !below_lo && !above_hi
    }
}

fn atom_of(v: &serde_json::Value) -> Option<Atom> {
    match v {
        serde_json::Value::Number(n) => n.as_f64().map(Atom::Num),
        serde_json::Value::String(s) => Some(Atom::Str(s.clone())),
        serde_json::Value::Bool(b) => Some(Atom::Bool(*b)),
        _ => None,
    }
}

/// 字段抽象状态
#[derive(Debug, Clone, Default, PartialEq)]
struct FieldC {
    lo: Option<Bound>,
    hi: Option<Bound>,
    eq: Option<Atom>,
    exclude: Vec<Atom>,
}

/// 原子约束：field op literal（`in [..]` 的列表展开由 `and_atoms` 完成）
#[derive(Debug, Clone)]
struct AtomC {
    field: String,
    op: CmpOp,
    lit: serde_json::Value,
}

/// 应用原子到存储；Err = 矛盾
fn apply_atom(fc: &mut FieldC, op: CmpOp, lit: &serde_json::Value) -> Result<(), ()> {
    let Some(a) = atom_of(lit) else {
        return Err(());
    };
    match op {
        // 数值区间（非数值字面量的区间比较 → 不支持，视为矛盾）
        CmpOp::Lt => {
            let Atom::Num(v) = a else { return Err(()) };
            if fc
                .lo
                .is_some_and(|l| if l.strict { l.v >= v } else { l.v > v })
            {
                return Err(());
            }
            fc.hi = Some(match fc.hi {
                Some(h) if h.v <= v && !(h.v == v && h.strict) => h,
                Some(h) if h.v <= v => h,
                _ => Bound { v, strict: true },
            });
            Ok(())
        }
        CmpOp::Le => {
            let Atom::Num(v) = a else { return Err(()) };
            if fc
                .lo
                .is_some_and(|l| if l.strict { l.v > v } else { l.v >= v })
            {
                return Err(());
            }
            fc.hi = Some(match fc.hi {
                Some(h) if h.v < v || (h.v == v && h.strict) => h,
                _ => Bound { v, strict: false },
            });
            Ok(())
        }
        CmpOp::Gt => {
            let Atom::Num(v) = a else { return Err(()) };
            if fc
                .hi
                .is_some_and(|h| if h.strict { h.v <= v } else { h.v < v })
            {
                return Err(());
            }
            fc.lo = Some(match fc.lo {
                Some(l) if l.v >= v => l,
                _ => Bound { v, strict: true },
            });
            Ok(())
        }
        CmpOp::Ge => {
            let Atom::Num(v) = a else { return Err(()) };
            if fc
                .hi
                .is_some_and(|h| if h.strict { h.v < v } else { h.v <= v })
            {
                return Err(());
            }
            fc.lo = Some(match fc.lo {
                Some(l) if l.v > v || (l.v == v && l.strict) => l,
                _ => Bound { v, strict: false },
            });
            Ok(())
        }
        CmpOp::Eq => {
            if let Atom::Num(v) = a {
                if !Atom::Num(v).num_ok(fc.lo, fc.hi) {
                    return Err(());
                }
            }
            if let Some(e) = &fc.eq {
                if *e != a {
                    return Err(());
                }
            }
            if fc.exclude.contains(&a) {
                return Err(());
            }
            fc.eq = Some(a);
            fc.lo = None;
            fc.hi = None;
            Ok(())
        }
        CmpOp::Ne => {
            if fc.eq.as_ref() == Some(&a) {
                return Err(());
            }
            if !fc.exclude.contains(&a) {
                fc.exclude.push(a);
            }
            Ok(())
        }
    }
}

/// 存储：field → FieldC
#[derive(Debug, Clone, Default, PartialEq)]
struct Store {
    fields: HashMap<String, FieldC>,
}

impl Store {
    fn apply(&mut self, atom: &AtomC) -> Result<(), ()> {
        let fc = self.fields.entry(atom.field.clone()).or_default();
        apply_atom(fc, atom.op, &atom.lit)
    }
}

// ─────────────────────────── 表达式归一化 ───────────────────────────

/// 归一化：唯一引擎的 AST 静态分析（`runtime_engine::and_atoms`）——
/// `&&` 链拆解为字段比较原子；不支持形态返回 None（保守可满足，零假阳性）。
fn normalize(expr: &str) -> Option<Vec<AtomC>> {
    let atoms = and_atoms(expr).ok()??;
    Some(
        atoms
            .into_iter()
            .map(|a| AtomC {
                field: a.field,
                op: a.op,
                lit: a.literal,
            })
            .collect(),
    )
}

/// 无字段引用常量表达式：静态真/假；含引用/解析失败 → None
fn constant_bool(expr: &str) -> Option<bool> {
    let script = expr.trim();
    if script.is_empty() {
        return None;
    }
    let eng = runtime_engine::RhaiExpressionEngine::new();
    // 空已知键集下严格校验通过 ⇒ 无任何上下文标识符引用（`ctx[...]` 恒可见，
    // 故显式排除：含 ctx 者保守放弃）；`validate_all` 在标识符层之上并拦未登记自由函数
    // （常量表达式里的 `median(...)` 运行期必失败 ⇒ 不应被当作可静态求值的常量）。
    if script.contains("ctx") || eng.validate_all(script, &[]).is_err() {
        return None;
    }
    eng.evaluate_bool(script, &HashMap::new()).ok()
}

// ─────────────────────────── 探测主流程 ───────────────────────────

/// 探测边（cond = 出边条件表达式文本；None = 无条件边）
#[derive(Debug, Clone)]
pub struct ProbeEdge {
    pub from: usize,
    pub to: usize,
    pub cond: Option<String>,
}

/// 探测发现
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProbeFinding {
    pub code: &'static str,
    /// true = error（阻断发布）；false = warning
    pub error: bool,
    pub node_ref: Option<String>,
    pub message: String,
}

fn finding(
    code: &'static str,
    error: bool,
    node_ref: Option<String>,
    msg: impl Into<String>,
) -> ProbeFinding {
    ProbeFinding {
        code,
        error,
        node_ref,
        message: msg.into(),
    }
}

const MAX_STATES_PER_NODE: usize = 24;
const MAX_TOTAL_STATES: usize = 4096;

/// 探测入口。nodes：图节点（含 id/type/label/next/dmn）；edges：envelope edges 数组
/// （可选）；ctx：样例上下文（仅用于 DMN 单元格求值）。
pub fn probe_graph(
    nodes: &[serde_json::Value],
    edges: Option<&[serde_json::Value]>,
    ctx: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<ProbeFinding> {
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let mut findings: Vec<ProbeFinding> = Vec::new();
    let keys: Vec<String> = nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| {
            nd.get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| i.to_string())
        })
        .collect();
    let labels: Vec<String> = nodes
        .iter()
        .map(|nd| {
            nd.get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();

    // ── 出边（含条件文本）装配：edges 数组优先，node.next 兜底（与 scan 同源）──
    let mut out_edges: Vec<Vec<ProbeEdge>> = vec![Vec::new(); n];
    let mut found_any = false;
    if let Some(edges) = edges {
        let idx = |k: &str| keys.iter().position(|x| x == k);
        for e in edges {
            let (Some(s), Some(t)) = (
                e.get("source").and_then(|v| v.as_str()),
                e.get("target").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let (Some(f), Some(ti)) = (idx(s), idx(t)) else {
                continue;
            };
            if f == ti {
                continue;
            }
            out_edges[f].push(ProbeEdge {
                from: f,
                to: ti,
                cond: e.get("cond").and_then(|v| v.as_str()).map(str::to_string),
            });
            found_any = true;
        }
    }
    if !found_any {
        for (i, nd) in nodes.iter().enumerate() {
            let Some(next) = nd.get("next").and_then(|v| v.as_array()) else {
                continue;
            };
            for nx in next {
                let Some(t) = nx.get("to").and_then(|v| v.as_i64()) else {
                    continue;
                };
                if t < 0 || t as usize >= n || t as usize == i {
                    continue;
                }
                out_edges[i].push(ProbeEdge {
                    from: i,
                    to: t as usize,
                    cond: nx.get("cond").and_then(|v| v.as_str()).map(str::to_string),
                });
                found_any = true;
            }
        }
    }
    if !found_any {
        return Vec::new();
    }

    // ── 不动点状态集传播 ──
    // states[u] = 到达 u 的抽象状态集（去重、上限截断）
    let mut states: Vec<Vec<Store>> = vec![Vec::new(); n];
    let mut queue: VecDeque<usize> = VecDeque::new();
    let starts: Vec<usize> = (0..n)
        .filter(|&i| !keys[i].is_empty() && type_of(nodes, i) == "start")
        .collect();
    for &s in &starts {
        let st = Store::default();
        states[s].push(st.clone());
        queue.push_back(s);
        let _ = st;
    }
    if starts.is_empty() {
        // 无 start：结构层已报 NO_START——探测无可为
        return Vec::new();
    }
    let mut total_states = starts.len();
    let mut budget_hit = false;
    while let Some(u) = queue.pop_front() {
        if states[u].len() > MAX_STATES_PER_NODE || total_states > MAX_TOTAL_STATES {
            budget_hit = true;
            continue;
        }
        let cur = states[u].clone();
        for edge in &out_edges[u] {
            for st in &cur {
                let mut next = st.clone();
                let ok = match &edge.cond {
                    None => Some(true),
                    Some(expr) => {
                        match constant_bool(expr) {
                            Some(true) => Some(true),
                            Some(false) => Some(false),
                            None => {
                                match normalize(expr) {
                                    None => Some(true), // 保守可满足
                                    Some(atoms) => {
                                        let mut ok2 = true;
                                        for a in &atoms {
                                            if next.apply(a).is_err() {
                                                ok2 = false;
                                                break;
                                            }
                                        }
                                        Some(ok2)
                                    }
                                }
                            }
                        }
                    }
                };
                if let Some(true) = ok {
                    let t = edge.to;
                    if !states[t].iter().any(|s| s == &next) {
                        states[t].push(next.clone());
                        total_states += 1;
                        if states[t].len() <= MAX_STATES_PER_NODE
                            && total_states <= MAX_TOTAL_STATES
                        {
                            queue.push_back(t);
                        } else {
                            budget_hit = true;
                        }
                    }
                }
            }
        }
    }
    if budget_hit {
        findings.push(finding(
            "BUDGET_EXCEEDED",
            false,
            None,
            "路径/状态预算超限——探测覆盖不完整，未证明项按可达保守处理（仅信息提示）",
        ));
    }

    // ── 终局判定 ──
    let reachable: HashSet<usize> = (0..n).filter(|&i| !states[i].is_empty()).collect();
    // NODE_UNREACHABLE（start/end 之外；end 不可达由 scan END_UNREACHABLE 覆盖）
    for i in 0..n {
        let ty = type_of(nodes, i);
        if ty == "start" || ty == "end" {
            continue;
        }
        if !reachable.contains(&i) {
            findings.push(finding(
                "NODE_UNREACHABLE",
                false,
                Some(keys[i].clone()),
                format!(
                    "节点 '{}' 在符号可达性下从 start 不可达（可能恒无执行路径）",
                    labels[i]
                ),
            ));
        }
    }
    // 出边可行性与 FANOUT_EMPTY_PROVEN
    for u in 0..n {
        if states[u].is_empty() || states[u].len() > MAX_STATES_PER_NODE {
            continue; // 不可达或状态集超限 → 不做证明
        }
        let ustates = states[u].clone();
        if out_edges[u].is_empty() {
            continue;
        }
        let mut all_infeasible = !out_edges[u].is_empty();
        for e in &out_edges[u] {
            let feasible = edge_feasible(&ustates, e);
            if !feasible {
                findings.push(finding(
                    "EDGE_INFEASIBLE",
                    false,
                    Some(keys[u].clone()),
                    format!(
                        "节点 '{}' 的出边条件 '{}' 在其全部可达状态下恒不可达",
                        labels[u],
                        e.cond.as_deref().unwrap_or("")
                    ),
                ));
            } else {
                all_infeasible = false;
            }
        }
        let has_default = out_edges[u].iter().any(|e| e.cond.is_none());
        if all_infeasible && !has_default {
            findings.push(finding(
                "FANOUT_EMPTY_PROVEN",
                true,
                Some(keys[u].clone()),
                format!(
                    "节点 '{}' 全部出边均被证明恒不可达且无兜底边——运行时必然空扇出静默停滞（阻断发布）",
                    labels[u]
                ),
            ));
        }
    }

    // ── DMN 规则可达性（决策节点）──
    for (i, nd) in nodes.iter().enumerate() {
        if type_of(nodes, i) != "decision" {
            continue;
        }
        let Some(dmn_val) = nd.get("dmn") else {
            continue;
        };
        let Some(table) = crate::dmn::parse_dmn(dmn_val) else {
            continue;
        };
        for (ri, rule) in table.rules.iter().enumerate() {
            if rule_dead(&rule.cells, ctx) {
                findings.push(finding(
                    "DMN_RULE_NEVER_HIT",
                    false,
                    Some(keys[i].clone()),
                    format!(
                        "决策节点 '{}' 第 {} 条规则（输出 {:?}）永不命中（单元格内部矛盾或与样例上下文冲突）",
                        labels[i],
                        ri + 1,
                        rule.output
                    ),
                ));
            }
        }
    }

    findings
}

fn type_of(nodes: &[serde_json::Value], i: usize) -> &str {
    nodes[i].get("type").and_then(|v| v.as_str()).unwrap_or("")
}

fn edge_feasible(ustates: &[Store], e: &ProbeEdge) -> bool {
    let Some(expr) = &e.cond else { return true };
    match constant_bool(expr) {
        Some(true) => return true,
        Some(false) => return false,
        None => {}
    }
    let Some(atoms) = normalize(expr) else {
        return true;
    };
    // 全部状态矛盾 → 不可达；任一状态可满足 → 可达
    for st in ustates {
        let mut s = st.clone();
        let mut ok = true;
        for a in &atoms {
            if s.apply(a).is_err() {
                ok = false;
                break;
            }
        }
        if ok {
            return true;
        }
    }
    false
}

/// DMN 规则行死亡判定：任一单元格与其它单元格矛盾（空 store 顺序并入）
/// 或 ctx 存在时任一非空单元格对 ctx 求值为 false。
fn rule_dead(
    cells: &[Option<String>],
    ctx: Option<&serde_json::Map<String, serde_json::Value>>,
) -> bool {
    // ctx 判定（覆盖"与样例上下文矛盾"）
    if let Some(ctx) = ctx {
        for c in cells.iter().flatten() {
            let c = c.trim();
            if c.is_empty() {
                continue;
            }
            match crate::advance::eval_flow_condition(c, ctx) {
                Ok(false) => return true,
                Ok(true) => {}
                Err(_) => return false, // 求值错误 → 不做死亡断言
            }
        }
    }
    // 内部矛盾：顺序并入空存储
    let mut store = Store::default();
    for c in cells.iter().flatten() {
        let c = c.trim();
        if c.is_empty() {
            continue;
        }
        match normalize(c) {
            None => return false, // 不支持形态 → 不做死亡断言
            Some(atoms) => {
                for a in atoms {
                    if store.apply(&a).is_err() {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, ty: &str, label: &str) -> serde_json::Value {
        json!({ "id": id, "type": ty, "label": label }) // id-json-ok
    }
    fn start() -> serde_json::Value {
        node("s", "start", "开始")
    }
    fn end(id: &str) -> serde_json::Value {
        node(id, "end", "结束")
    }
    fn codes(fs: &[ProbeFinding]) -> Vec<String> {
        fs.iter().map(|f| f.code.to_string()).collect()
    }

    #[test]
    fn constant_false_edge_infeasible() {
        // a 有恒假条件边 → e 与无条件兜底边 → e2；恒假边检出、兜底阻止 fanout
        let nodes = vec![start(), node("a", "approve", "审批"), end("e"), end("e2")];
        let edges = vec![
            json!({ "source": "s", "target": "a" }),
            json!({ "source": "a", "target": "e", "cond": "1 > 2" }),
            json!({ "source": "a", "target": "e2" }),
        ];
        let fs = probe_graph(&nodes, Some(&edges), None);
        assert!(
            codes(&fs).contains(&"EDGE_INFEASIBLE".to_string()),
            "{fs:?}"
        );
        assert!(!codes(&fs).contains(&"FANOUT_EMPTY_PROVEN".to_string()));
    }

    #[test]
    fn cross_edge_interval_proof() {
        // s →(amount<=500) c →(amount>1000) e：c 处恒矛盾
        let nodes = vec![start(), node("c", "condition", "条件"), end("e")];
        let edges = vec![
            json!({ "source": "s", "target": "c", "cond": "amount <= 500" }),
            json!({ "source": "c", "target": "e", "cond": "amount > 1000" }),
        ];
        let fs = probe_graph(&nodes, Some(&edges), None);
        assert!(
            codes(&fs).contains(&"EDGE_INFEASIBLE".to_string()),
            "c→e 在 amount<=500 下恒矛盾: {fs:?}"
        );
    }

    #[test]
    fn fanout_proven_error() {
        // 节点两条恒假条件边且无兜底 → FANOUT_EMPTY_PROVEN(error)
        let nodes = vec![start(), node("a", "approve", "审批"), end("e"), end("e2")];
        let edges = vec![
            json!({ "source": "s", "target": "a" }),
            json!({ "source": "a", "target": "e", "cond": "2 > 3" }),
            json!({ "source": "a", "target": "e2", "cond": "amount > 9999 && amount < 1" }),
        ];
        let fs = probe_graph(&nodes, Some(&edges), None);
        let fe = fs.iter().find(|f| f.code == "FANOUT_EMPTY_PROVEN");
        assert!(fe.is_some(), "{fs:?}");
        assert!(fe.unwrap().error, "空扇出证明应为 error");
    }

    #[test]
    fn satisfiable_edge_not_reported() {
        let nodes = vec![start(), node("a", "approve", "审批"), end("e"), end("e2")];
        let edges = vec![
            json!({ "source": "s", "target": "a" }),
            json!({ "source": "a", "target": "e", "cond": "amount > 1000" }),
            json!({ "source": "a", "target": "e2" }),
        ];
        let fs = probe_graph(&nodes, Some(&edges), None);
        assert!(
            !codes(&fs).contains(&"EDGE_INFEASIBLE".to_string()),
            "{fs:?}"
        );
        assert!(!codes(&fs).contains(&"FANOUT_EMPTY_PROVEN".to_string()));
    }

    #[test]
    fn unreachable_node_warned() {
        // x 孤立（有出边但无入边）
        let mut x = node("x", "approve", "孤岛审批");
        x["next"] = json!([{ "to": 3 }]);
        let nodes = vec![start(), node("a", "approve", "审批"), x, end("e")];
        let edges = vec![
            json!({ "source": "s", "target": "a" }),
            json!({ "source": "a", "target": "e" }),
        ];
        let fs = probe_graph(&nodes, Some(&edges), None);
        assert!(
            codes(&fs).contains(&"NODE_UNREACHABLE".to_string()),
            "{fs:?}"
        );
    }

    #[test]
    fn dmn_internal_contradiction_rule_dead() {
        let mut d = node("d", "decision", "决策");
        d["dmn"] = json!({
            "hitPolicy": "FIRST",
            "inputs": [{ "name": "amount" }, { "name": "region" }],
            "outputs": [{ "name": "route" }],
            "rules": [
                { "match": ["amount > 1000", "amount <= 500"], "output": "approve-route" },
                { "match": ["amount > 1000", "region == \"NW\""], "output": "review-route" }
            ]
        });
        let nodes = vec![start(), d, end("e")];
        let edges = vec![
            json!({ "source": "s", "target": "d" }),
            json!({ "source": "d", "target": "e" }),
        ];
        let fs = probe_graph(&nodes, Some(&edges), None);
        let dead: Vec<_> = fs
            .iter()
            .filter(|f| f.code == "DMN_RULE_NEVER_HIT")
            .collect();
        assert_eq!(dead.len(), 1, "仅第 1 行内部矛盾: {fs:?}");
    }

    #[test]
    fn dmn_ctx_contradiction_rule_dead() {
        let mut d = node("d", "decision", "决策");
        d["dmn"] = json!({
            "hitPolicy": "FIRST",
            "inputs": [{ "name": "region" }],
            "outputs": [{ "name": "route" }],
            "rules": [
                { "match": ["region == \"NW\""], "output": "nw-route" },
                { "match": [], "output": "default-route" }
            ]
        });
        let nodes = vec![start(), d, end("e")];
        let edges = vec![
            json!({ "source": "s", "target": "d" }),
            json!({ "source": "d", "target": "e" }),
        ];
        let ctx: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(r#"{"region":"SE"}"#).unwrap();
        let fs = probe_graph(&nodes, Some(&edges), Some(&ctx));
        let dead: Vec<_> = fs
            .iter()
            .filter(|f| f.code == "DMN_RULE_NEVER_HIT")
            .collect();
        assert_eq!(dead.len(), 1, "NW 行在 ctx region=SE 下死亡: {fs:?}");
    }

    #[test]
    fn no_semantic_no_findings() {
        // 纯结构图无 cond → 探测空
        let nodes = vec![start(), node("a", "approve", "审批"), end("e")];
        let edges = vec![
            json!({ "source": "s", "target": "a" }),
            json!({ "source": "a", "target": "e" }),
        ];
        let fs = probe_graph(&nodes, Some(&edges), None);
        assert!(fs.is_empty(), "{fs:?}");
    }
}
