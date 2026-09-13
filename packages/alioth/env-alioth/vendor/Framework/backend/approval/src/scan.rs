//! 流程图静态缺陷扫描（add-approval-flow-static-scan）
//!
//! 目的：把引擎 fail-closed 静默停滞形态中**静态可判**的四类在保存/发布前显式检出：
//! ① 无可达 end 收尾（error）；② 仅自动节点成环无出口（error，有界 loop 豁免）；
//! ③ 节点出边全带条件无兜底 → 运行时扇出可空（warning）；④ 人工节点审批人
//! 配置缺失或解析后空集（warning，DB 实查）。
//!
//! 语义对齐：边推导与 `handlers/publish.rs` `materialize_graph` 同一算法
//! （图级 id/下标键 + edges 数组优先、node.next 兜底；跳过自环）——一致性由
//! 集成测试「物化边 == 扫描边集」锚定。不做条件表达式/DMN 真值评估（无上下文）。
//!
//! validate 与 publish 同判据：validate（只读连接）与 publish（事务连接）调用
//! 同一 `scan_graph`，仅 ④ 在无连接时降级为配置存在性检查。

use serde::Serialize;
use serde_json::Value;

/// 发现严重级：error = 结构性硬错（阻断发布）；warning = 疑似形态（放行）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// 单条扫描发现（node_ref = 图级节点 id/下标，可定位）
#[derive(Debug, Clone, Serialize)]
pub struct ScanFinding {
    pub code: &'static str,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_ref: Option<String>,
    pub message: String,
}

impl ScanFinding {
    fn error(code: &'static str, node_ref: Option<String>, message: String) -> Self {
        Self {
            code,
            severity: Severity::Error,
            node_ref,
            message,
        }
    }
    fn warning(code: &'static str, node_ref: Option<String>, message: String) -> Self {
        Self {
            code,
            severity: Severity::Warning,
            node_ref,
            message,
        }
    }
}

/// 扫描报告：errors 非空 → 发布阻断；仅 warnings → 放行
#[derive(Debug, Default, Serialize)]
pub struct ScanReport {
    pub errors: Vec<ScanFinding>,
    pub warnings: Vec<ScanFinding>,
}

impl ScanReport {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
    fn push(&mut self, f: ScanFinding) {
        match f.severity {
            Severity::Error => self.errors.push(f),
            Severity::Warning => self.warnings.push(f),
        }
    }
}

/// 节点角色分类（与发布物化/运行时推进词汇一致）：
/// 人工 = approve/review/action/vote 族（含历史词汇 approval/oper-approve）；
/// 终端 = start/end；其余 = 自动（cc/branch/condition/decision/gate/loop/parallel/subflow）
fn is_action(t: &str) -> bool {
    matches!(
        t,
        "approve" | "approval" | "oper-approve" | "review" | "action" | "vote"
    )
}

/// simulate walker 消费：动作节点（人工裁决）判定
pub(crate) fn node_kind_is_action(t: &str) -> bool {
    is_action(t)
}
/// simulate walker 消费：condition 节点
pub(crate) fn node_kind_is_condition(t: &str) -> bool {
    t == "condition"
}
/// simulate walker 消费：decision（DMN）节点
pub(crate) fn node_kind_is_decision(t: &str) -> bool {
    t == "decision"
}

/// 出边（from/to 均为 nodes 下标；has_cond = 边携带 cond）
struct Edge {
    from: usize,
    to: usize,
    has_cond: bool,
}

/// 边推导（镜像 materialize_graph：图级键优先 edges 数组；空则 node.next 兜底；
/// 自环跳过——发布侧同样丢弃，引擎永不见自环）
fn derive_edges(nodes: &[Value], edges: Option<&[Value]>) -> Vec<Edge> {
    let keys: Vec<String> = nodes
        .iter()
        .enumerate()
        .map(|(idx, n)| {
            n.get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| idx.to_string())
        })
        .collect();
    let idx_of = |key: &str| keys.iter().position(|k| k == key);

    let mut out: Vec<Edge> = Vec::new();
    if let Some(edges) = edges {
        for e in edges {
            let (Some(s), Some(t)) = (
                e.get("source").and_then(|v| v.as_str()),
                e.get("target").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let (Some(f), Some(ti)) = (idx_of(s), idx_of(t)) else {
                continue;
            };
            if f == ti {
                continue;
            }
            out.push(Edge {
                from: f,
                to: ti,
                has_cond: e.get("cond").and_then(|v| v.as_str()).is_some(),
            });
        }
    }
    if !out.is_empty() {
        return out;
    }
    // node.next 兜底（{to, cond, label} 对象形态；发布同源）
    for (idx, node) in nodes.iter().enumerate() {
        let Some(next_arr) = node.get("next").and_then(|v| v.as_array()) else {
            continue;
        };
        for nxt in next_arr {
            let Some(t) = nxt.get("to").and_then(|v| v.as_i64()) else {
                continue;
            };
            if t < 0 || (t as usize) >= nodes.len() || t as usize == idx {
                continue;
            }
            out.push(Edge {
                from: idx,
                to: t as usize,
                has_cond: nxt.get("cond").and_then(|v| v.as_str()).is_some(),
            });
        }
    }
    out
}

/// 节点 → 有向邻接（用于 BFS 与 SCC）
fn adjacency(n: usize, edges: &[Edge]) -> Vec<Vec<usize>> {
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in edges {
        adj[e.from].push(e.to);
    }
    adj
}

/// 迭代 Tarjan SCC（无新依赖；返回强连通分量，每项为节点下标集合）
fn tarjan_scc(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = adj.len();
    let mut index = 0usize;
    let mut idx = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut comps: Vec<Vec<usize>> = Vec::new();

    // 显式栈 DFS：frame = (node, next_child_idx)
    for seed in 0..n {
        if idx[seed] != usize::MAX {
            continue;
        }
        let mut frames = vec![(seed, 0usize)];
        idx[seed] = index;
        low[seed] = index;
        index += 1;
        stack.push(seed);
        on_stack[seed] = true;
        while let Some(&mut (u, ref mut ci)) = frames.last_mut() {
            if *ci < adj[u].len() {
                let v = adj[u][*ci];
                *ci += 1;
                if idx[v] == usize::MAX {
                    idx[v] = index;
                    low[v] = index;
                    index += 1;
                    stack.push(v);
                    on_stack[v] = true;
                    frames.push((v, 0usize));
                } else if on_stack[v] {
                    low[u] = low[u].min(idx[v]);
                }
            } else {
                frames.pop();
                if let Some(&(p, _)) = frames.last() {
                    low[p] = low[p].min(low[u]);
                }
                if low[u] == idx[u] {
                    let mut comp = Vec::new();
                    loop {
                        let w = stack.pop().expect("scc stack non-empty");
                        on_stack[w] = false;
                        comp.push(w);
                        if w == u {
                            break;
                        }
                    }
                    comps.push(comp);
                }
            }
        }
    }
    comps
}

/// 有界 loop 判定：publish 物化 node.maxIter → timeline.loopMaxIter；
/// 缺省/≤0 视为无界（运行时 cursor < maxIter 永不满足退出条件）
fn loop_bounded(node: &Value) -> bool {
    node.get("type").and_then(|v| v.as_str()) == Some("loop")
        && node
            .get("maxIter")
            .and_then(|v| v.as_i64())
            .map(|m| m > 0)
            .unwrap_or(false)
}

/// 审批人四类键归一（镜像 publish approver_sel_of + escalateTo 读兼容）
/// 返回 (规范键是否全部缺省, 各键 sel 是否已填充（结构级）)
/// 结构级填充判定不依赖 DB——DB 空集判定由调用方在有连接时执行。
fn approver_config_present(node: &Value) -> bool {
    let legacy_role_kind = node.get("roleKind").and_then(|v| v.as_str());
    let direct =
        crate::handlers::publish::approver_sel_of(node, "direct", Some("role"), legacy_role_kind);
    if !direct.pos.is_empty() || direct.user.is_some() {
        return true;
    }
    for (key, legacy) in [
        ("deputy", "roleDeputy"),
        ("escalate", "roleEscalate"),
        ("backup", "roleBackup"),
    ] {
        let sel = crate::handlers::publish::approver_sel_of(node, key, Some(legacy), None);
        if !sel.pos.is_empty() || sel.user.is_some() {
            return true;
        }
    }
    // legacy escalateTo（岗位名直写，无对象键）
    if node
        .get("escalateTo")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    false
}

/// 结构检查（纯同步；①-③ + ④ 配置存在性）
fn structural_scan(nodes: &[Value], edges: &[Edge]) -> ScanReport {
    let mut report = ScanReport::default();
    let n = nodes.len();
    let types: Vec<String> = nodes
        .iter()
        .map(|nd| {
            nd.get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string()
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

    // 出边按源分组（供 ①-③）
    let mut outs: Vec<Vec<(usize, bool)>> = vec![Vec::new(); n];
    for e in edges {
        outs[e.from].push((e.to, e.has_cond));
    }

    // ── ① start 可达 + 可达 end ─────────────────────────────────
    let starts: Vec<usize> = (0..n).filter(|&i| types[i] == "start").collect();
    if starts.is_empty() {
        report.push(ScanFinding::error(
            "NO_START",
            None,
            "设计图缺少 start 节点——流程无法发起".into(),
        ));
    }
    let adj = adjacency(n, edges);
    let mut reach = vec![false; n];
    let mut stack: Vec<usize> = starts.clone();
    while let Some(u) = stack.pop() {
        if reach[u] {
            continue;
        }
        reach[u] = true;
        stack.extend(adj[u].iter().copied());
    }
    let ends: Vec<usize> = (0..n).filter(|&i| types[i] == "end").collect();
    if ends.is_empty() {
        report.push(ScanFinding::error(
            "NO_END",
            None,
            "设计图无 end 收尾节点——流程到达终点时不会物化结论/收尾".into(),
        ));
    } else {
        let reachable_ends: Vec<usize> = ends.iter().copied().filter(|&i| reach[i]).collect();
        if reachable_ends.is_empty() {
            let sample: Vec<String> = ends
                .iter()
                .take(3)
                .map(|&i| format!("'{}'", labels[i]))
                .collect();
            report.push(ScanFinding::error(
                "END_UNREACHABLE",
                None,
                format!(
                    "end 节点（{}）均无法从 start 到达——流程执行路径缺失收尾",
                    sample.join("、")
                ),
            ));
        }
    }

    // ── ② 仅自动节点成环无出口 ─────────────────────────────────
    let auto_of = |i: usize| !is_action(&types[i]) && types[i] != "start" && types[i] != "end";
    let auto_adj: Vec<Vec<usize>> = (0..n)
        .map(|i| {
            if auto_of(i) {
                adj[i].iter().copied().filter(|&t| auto_of(t)).collect()
            } else {
                Vec::new()
            }
        })
        .collect();
    for comp in tarjan_scc(&auto_adj) {
        if comp.len() < 2 {
            continue;
        }
        // 出口判定：分量内节点存在指向分量外（任意节点）的边 = 可从环离开
        let in_comp = |v: usize| comp.contains(&v);
        let has_exit = comp.iter().any(|&u| adj[u].iter().any(|&t| !in_comp(t)));
        let has_bounded_loop = comp.iter().any(|&u| loop_bounded(&nodes[u]));
        if !has_exit && !has_bounded_loop {
            let sample: Vec<String> = comp
                .iter()
                .take(3)
                .map(|&u| format!("'{}'({})", labels[u], keys[u]))
                .collect();
            report.push(ScanFinding::error(
                "AUTO_CYCLE",
                comp.first().map(|&u| keys[u].clone()),
                format!(
                    "自动节点成环且无出口（{}…）——推进将无限循环至深度截断",
                    sample.join("、")
                ),
            ));
        }
    }

    // ── ③ 全条件出边无兜底（防空扇出静默停滞）────────────────
    for i in 0..n {
        if !is_action(&types[i]) && types[i] != "condition" {
            continue;
        }
        if outs[i].is_empty() {
            continue;
        }
        if outs[i].iter().all(|&(_, has_cond)| has_cond) {
            report.push(ScanFinding::warning(
                "EMPTY_FANOUT",
                Some(keys[i].clone()),
                format!(
                    "节点 '{}' 全部出边均带条件且无兜底边——运行时条件全不命中将空扇出、流程静默停滞（建议配置无条件兜底边）",
                    labels[i]
                ),
            ));
        }
    }

    // ── ④ 人工节点审批人配置存在性（DB 空集实查在 scan_graph）──
    for i in 0..n {
        if !is_action(&types[i]) {
            continue;
        }
        if types[i] == "vote" {
            if node_has_vote_sources(&nodes[i]) == Some(false) {
                report.push(ScanFinding::warning(
                    "VOTE_NO_SOURCE",
                    Some(keys[i].clone()),
                    format!(
                        "投票节点 '{}' 未配置 voteSources——运行时零投票人，quorum 永不达标",
                        labels[i]
                    ),
                ));
            }
            continue;
        }
        if !approver_config_present(&nodes[i]) {
            report.push(ScanFinding::warning(
                "APPROVER_UNCONFIGURED",
                Some(keys[i].clone()),
                format!(
                    "审批节点 '{}' 未配置审批人（direct/role/岗位）——运行时解析零审批人（仅 admin 兜底可见）",
                    labels[i]
                ),
            ));
        }
    }

    report
}

/// voteSources 存在性：None = 字段缺失（=false 判据），Some(true) = 非空数组
fn node_has_vote_sources(node: &Value) -> Option<bool> {
    match node.get("voteSources") {
        None => Some(false),
        Some(Value::Array(a)) => Some(!a.is_empty()),
        Some(_) => Some(true),
    }
}

/// 完整扫描入口：结构检查 + ④ DB 实查（conn 提供时）。
/// conn 为 None → ④ 仅配置存在性（validate 无 DB 时降级，结构检查不受影响）。
/// 与 materialize_graph 前置 validate_graph 配合：nodes/edges 已过白名单与终端配置校验。
pub async fn scan_graph(
    nodes: &[Value],
    edges_opt: Option<&[Value]>,
    conn: Option<&mut sqlx::PgConnection>,
) -> ScanReport {
    let edges = derive_edges(nodes, edges_opt);
    let mut report = structural_scan(nodes, &edges);
    // 路径约束探测（add-approval-flow-simulation I3）：符号可达性/恒不可达边/
    // 可证明空扇出（error）/DMN 永命规则；无上下文亦可运行。结果并入分级报告。
    for pf in crate::probe::probe_graph(nodes, edges_opt, None) {
        report.push(ScanFinding {
            code: pf.code,
            severity: if pf.error {
                Severity::Error
            } else {
                Severity::Warning
            },
            node_ref: pf.node_ref,
            message: pf.message,
        });
    }
    if conn.is_none() {
        return report;
    }
    let conn = conn.expect("conn checked");
    for node in nodes {
        let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if !is_action(node_type) || node_type == "vote" {
            continue;
        }
        if !approver_config_present(node) {
            continue; // 已在结构层报 APPROVER_UNCONFIGURED
        }
        // 有效配置但 DB 解析为空集 → 运行时零审批人
        let sel = effective_approver_sels(node);
        let mut resolved: Vec<i64> = Vec::new();
        let mut resolve_err: Option<String> = None;
        for s in &sel {
            match crate::handlers::publish::resolve_approver_sel(conn, s).await {
                Ok(rows) => resolved.extend(rows),
                Err(e) => {
                    resolve_err = Some(e.to_string());
                    break;
                }
            }
        }
        if resolve_err.is_some() {
            report.push(ScanFinding::warning(
                "APPROVER_UNRESOLVED",
                node.get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                format!(
                    "审批节点 '{}' 审批人解析失败（{}）——无法确认真空，发布后可能零审批人",
                    node.get("label").and_then(|v| v.as_str()).unwrap_or(""),
                    resolve_err.unwrap_or_default()
                ),
            ));
        } else if resolved.is_empty() {
            report.push(ScanFinding::warning(
                "APPROVER_EMPTY",
                node.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                format!(
                    "审批节点 '{}' 配置的岗位/员工解析后为空集（岗位无活跃任职或员工不存在）——运行时零审批人（仅 admin 兜底可见）",
                    node.get("label").and_then(|v| v.as_str()).unwrap_or("")
                ),
            ));
        }
    }
    report
}

/// 审批节点有效审批人选择集（镜像 publish 桥插入语义）：
/// approve 族 = direct ∪ (direct 空时 deputy) ∪ escalate ∪ backup；
/// review/action = direct
fn effective_approver_sels(node: &Value) -> Vec<crate::handlers::publish::ApproverSel> {
    let role_kind = node.get("roleKind").and_then(|v| v.as_str());
    let mut sels = Vec::new();
    let direct = crate::handlers::publish::approver_sel_of(node, "direct", Some("role"), role_kind);
    let node_type = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let approve_family = matches!(node_type, "approve" | "approval" | "oper-approve");
    if approve_family {
        sels.push(direct);
        let deputy =
            crate::handlers::publish::approver_sel_of(node, "deputy", Some("roleDeputy"), None);
        if !deputy.pos.is_empty() || deputy.user.is_some() {
            sels.push(deputy);
        }
        let mut escalate =
            crate::handlers::publish::approver_sel_of(node, "escalate", Some("roleEscalate"), None);
        if escalate.pos.is_empty() && escalate.user.is_none() {
            if let Some(to) = node.get("escalateTo").and_then(|v: &Value| v.as_str()) {
                if !to.trim().is_empty() {
                    escalate.pos = to.trim().to_string();
                }
            }
        }
        if !escalate.pos.is_empty() || escalate.user.is_some() {
            sels.push(escalate);
        }
        let backup =
            crate::handlers::publish::approver_sel_of(node, "backup", Some("roleBackup"), None);
        if !backup.pos.is_empty() || backup.user.is_some() {
            sels.push(backup);
        }
    } else if !direct.pos.is_empty() || direct.user.is_some() {
        sels.push(direct);
    }
    sels
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, ty: &str, label: &str) -> Value {
        json!({ "id": id, "type": ty, "label": label }) // id-json-ok
    }
    fn edge(s: &str, t: &str) -> Value {
        json!({ "source": s, "target": t })
    }
    fn edge_cond(s: &str, t: &str) -> Value {
        json!({ "source": s, "target": t, "cond": "amount > 100" })
    }

    fn nodes_vec(arr: Vec<Value>) -> Vec<Value> {
        arr
    }
    fn edges_arr(edges: Vec<Value>) -> Option<Vec<Value>> {
        Some(edges)
    }
    fn run(nodes: Vec<Value>, edges: Option<Vec<Value>>) -> ScanReport {
        let nodes_s = nodes_vec(nodes);
        let edges_s = edges.as_deref();
        // structural_scan 直接调用（无 DB）
        let es = derive_edges(&nodes_s, edges_s);
        structural_scan(&nodes_s, &es)
    }

    #[test]
    fn healthy_linear_flow_clean() {
        let mut a1 = node("a1", "approve", "部门审批");
        a1["direct"] = json!({ "pos": "部门经理" });
        let r = run(
            vec![node("s1", "start", "发起"), a1, node("e1", "end", "结束")],
            edges_arr(vec![edge("s1", "a1"), edge("a1", "e1")]),
        );
        assert!(r.errors.is_empty(), "errors: {:?}", r.errors);
        assert!(r.warnings.is_empty(), "warnings: {:?}", r.warnings);
    }

    #[test]
    fn missing_end_is_error() {
        let r = run(
            vec![
                node("s1", "start", "发起"),
                node("a1", "approve", "部门审批"),
            ],
            edges_arr(vec![edge("s1", "a1")]),
        );
        assert!(
            r.errors.iter().any(|f| f.code == "NO_END"),
            "{:?}",
            r.errors
        );
    }

    #[test]
    fn unreachable_end_is_error() {
        let r = run(
            vec![
                node("s1", "start", "发起"),
                node("a1", "approve", "审批"),
                node("e1", "end", "结束"),
                node("x1", "approve", "孤立节点"),
            ],
            edges_arr(vec![edge("s1", "a1"), edge("a1", "e1")]),
        );
        assert!(!r.errors.iter().any(|f| f.code == "END_UNREACHABLE"));
        // end 可达 → 无 END_UNREACHABLE；孤立 x1 不影响 end 判据（分支合法）
    }

    #[test]
    fn all_ends_unreachable_is_error() {
        let r = run(
            vec![
                node("s1", "start", "发起"),
                node("a1", "approve", "审批"),
                node("e1", "end", "结束"),
            ],
            edges_arr(vec![edge("s1", "a1")]), // e1 无入边
        );
        assert!(
            r.errors.iter().any(|f| f.code == "END_UNREACHABLE"),
            "{:?}",
            r.errors
        );
    }

    #[test]
    fn auto_cycle_without_exit_is_error() {
        let r = run(
            vec![
                node("s1", "start", "发起"),
                node("c1", "condition", "条件A"),
                node("c2", "condition", "条件B"),
                node("e1", "end", "结束"),
            ],
            edges_arr(vec![
                edge("s1", "c1"),
                edge("c1", "c2"),
                edge("c2", "c1"), // 自动环
            ]),
        );
        assert!(
            r.errors.iter().any(|f| f.code == "AUTO_CYCLE"),
            "{:?}",
            r.errors
        );
    }

    #[test]
    fn auto_cycle_with_human_exit_allowed() {
        // condition 环但 c1 有指向人工节点 a1 的出口 → 引擎可离开环
        let r = run(
            vec![
                node("s1", "start", "发起"),
                node("c1", "condition", "条件A"),
                node("c2", "condition", "条件B"),
                node("a1", "approve", "人工审批"),
                node("e1", "end", "结束"),
            ],
            edges_arr(vec![
                edge("s1", "c1"),
                edge("c1", "c2"),
                edge("c2", "c1"),
                edge("c1", "a1"),
                edge("a1", "e1"),
            ]),
        );
        assert!(
            !r.errors.iter().any(|f| f.code == "AUTO_CYCLE"),
            "{:?}",
            r.errors
        );
    }

    #[test]
    fn bounded_loop_cycle_exempt() {
        let mut l = node("l1", "loop", "循环");
        l["maxIter"] = json!(10);
        let r = run(
            vec![
                node("s1", "start", "发起"),
                node("c1", "condition", "条件"),
                l,
                node("e1", "end", "结束"),
            ],
            edges_arr(vec![
                edge("s1", "c1"),
                edge("c1", "l1"),
                edge("l1", "c1"),
                edge("c1", "e1"),
            ]),
        );
        assert!(
            !r.errors.iter().any(|f| f.code == "AUTO_CYCLE"),
            "{:?}",
            r.errors
        );
    }

    #[test]
    fn no_fallback_edges_warn() {
        let r = run(
            vec![
                node("s1", "start", "发起"),
                node("a1", "approve", "审批"),
                node("e1", "end", "结束"),
                node("e2", "end", "拒绝结束"),
            ],
            edges_arr(vec![
                edge("s1", "a1"),
                edge_cond("a1", "e1"),
                edge_cond("a1", "e2"), // 全 cond 无兜底
            ]),
        );
        assert!(
            r.warnings.iter().any(|f| f.code == "EMPTY_FANOUT"),
            "{:?}",
            r.warnings
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    #[test]
    fn fallback_edge_clean() {
        let r = run(
            vec![
                node("s1", "start", "发起"),
                node("a1", "approve", "审批"),
                node("e1", "end", "通过结束"),
                node("e2", "end", "拒绝结束"),
            ],
            edges_arr(vec![
                edge("s1", "a1"),
                edge_cond("a1", "e1"),
                edge("a1", "e2"), // 无条件兜底
            ]),
        );
        assert!(
            !r.warnings.iter().any(|f| f.code == "EMPTY_FANOUT"),
            "{:?}",
            r.warnings
        );
    }

    #[test]
    fn no_approver_config_warns() {
        let r = run(
            vec![
                node("s1", "start", "发起"),
                node("a1", "approve", "审批"),
                node("e1", "end", "结束"),
            ],
            edges_arr(vec![edge("s1", "a1"), edge("a1", "e1")]),
        );
        assert!(
            r.warnings.iter().any(|f| f.code == "APPROVER_UNCONFIGURED"),
            "{:?}",
            r.warnings
        );
    }

    #[test]
    fn configured_approver_no_warn() {
        let mut a = node("a1", "approve", "审批");
        a["direct"] = json!({ "pos": "部门经理" });
        let r = run(
            vec![node("s1", "start", "发起"), a, node("e1", "end", "结束")],
            edges_arr(vec![edge("s1", "a1"), edge("a1", "e1")]),
        );
        assert!(
            !r.warnings.iter().any(|f| f.code == "APPROVER_UNCONFIGURED"),
            "{:?}",
            r.warnings
        );
    }

    #[test]
    fn missing_start_is_error() {
        let r = run(
            vec![node("a1", "approve", "审批"), node("e1", "end", "结束")],
            edges_arr(vec![edge("a1", "e1")]),
        );
        assert!(
            r.errors.iter().any(|f| f.code == "NO_START"),
            "{:?}",
            r.errors
        );
    }

    #[test]
    fn next_array_fallback_derives_edges() {
        // 无 edges 数组（数组 envelope 形态）→ node.next 兜底
        let mut s = node("s1", "start", "发起");
        s["next"] = json!([{ "to": 1 }]);
        let mut a = node("a1", "approve", "审批");
        a["next"] = json!([{ "to": 2 }]);
        a["direct"] = json!({ "pos": "部门经理" });
        let nodes = vec![s, a, node("e1", "end", "结束")];
        let es = derive_edges(&nodes, None);
        assert_eq!(es.len(), 2, "s1→a1, a1→e1");
        let r = structural_scan(&nodes, &es);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    #[test]
    fn self_edges_skipped() {
        // 自环在发布侧被丢弃——扫描不得误报
        let nodes = vec![node("c1", "condition", "条件")];
        let es = derive_edges(&nodes, Some(&[json!({ "source": "c1", "target": "c1" })]));
        assert!(es.is_empty());
    }
}
