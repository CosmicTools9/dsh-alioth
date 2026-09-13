//! 模拟走查（add-approval-flow-simulation I2）
//!
//! 无状态逐步 replay walker：`POST /approval-flows/{id}/simulate` 以样例上下文 +
//! 步进意图序列驱动图走查——condition/decision/dmn 用样例 ctx 求值路由，人工节点
//! 按意图选边。dry-run：零 DB 写、零实例物化。
//!
//! walker 语义（design.md D2）：人工节点处需要步骤（目标出边序号/标签），无步骤 →
//! 停在人工节点（供前端渐进 replay 追加）；求值错误/空扇出/无命中 → 终止并标注
//! 发现（fail-closed，与运行时一致）；parallel 全出边入队；loop 按有界公式/退出一
//! 边近似（Non-Goal：完整运行时签署语义不建模）。

use actix_web::{web, HttpResponse};
use common::error::AliothError;
use common::ApiResponse;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::scan::{node_kind_is_action, node_kind_is_condition, node_kind_is_decision};

// ───────────────────────────── walker ─────────────────────────────

/// 走查单步记录
#[derive(Debug, Clone, Serialize)]
pub struct SimStep {
    pub node_id: String,
    pub label: String,
    pub node_type: String,
    /// 本步去向说明（auto 求值结果 / 人工意图 / 终止原因）
    pub decision: String,
}

/// 走查结果
#[derive(Debug, Clone, Serialize)]
pub struct SimResult {
    pub path: Vec<SimStep>,
    /// 发现的缺陷（code + 描述；error 级语义同 scan）
    pub findings: Vec<serde_json::Value>,
    pub reached_end: bool,
    /// None = 正常走完/停在人工节点；Some = 终止原因描述
    pub terminated: Option<String>,
    /// 停在人工节点等待输入（渐进 replay 信号）
    pub awaiting_human: Option<String>,
}

/// 意图步骤：node 为图级 id；edge = 目标出边序号（0-based，按边装配顺序）或
/// 省略（pass → 首条无条件/首条边）
#[derive(Debug, Clone, Deserialize)]
pub struct SimIntent {
    pub node: String,
    #[serde(default)]
    pub edge: Option<usize>,
}

/// 装配走查边（nodes + next/edges 同 scan 来源；带 label/cond）
#[derive(Debug, Clone)]
struct WalkEdge {
    to: usize,
    cond: Option<String>,
    label: Option<String>,
}

fn walk_edges(nodes: &[Value], edges_opt: Option<&[Value]>) -> Vec<Vec<WalkEdge>> {
    let n = nodes.len();
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
    let mut out: Vec<Vec<WalkEdge>> = vec![Vec::new(); n];
    let mut filled = false;
    if let Some(edges) = edges_opt {
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
            out[f].push(WalkEdge {
                to: ti,
                cond: e.get("cond").and_then(|v| v.as_str()).map(str::to_string),
                label: e.get("label").and_then(|v| v.as_str()).map(str::to_string),
            });
            filled = true;
        }
    }
    if !filled {
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
                out[i].push(WalkEdge {
                    to: t as usize,
                    cond: nx.get("cond").and_then(|v| v.as_str()).map(str::to_string),
                    label: nx.get("label").and_then(|v| v.as_str()).map(str::to_string),
                });
            }
        }
    }
    out
}

fn node_type(nodes: &[Value], i: usize) -> &str {
    nodes[i].get("type").and_then(|v| v.as_str()).unwrap_or("")
}
fn node_label(nodes: &[Value], i: usize) -> &str {
    nodes[i].get("label").and_then(|v| v.as_str()).unwrap_or("")
}
fn node_key(nodes: &[Value], i: usize) -> String {
    nodes[i]
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| i.to_string())
}

const MAX_WALK_STEPS: usize = 200;

/// 纯走查（供单测与 handler 复用）
pub fn run_walk(
    nodes: &[Value],
    edges: Option<&[Value]>,
    ctx: &serde_json::Map<String, Value>,
    intents: &[SimIntent],
) -> SimResult {
    let n = nodes.len();
    let out = walk_edges(nodes, edges);
    let starts: Vec<usize> = (0..n).filter(|&i| node_type(nodes, i) == "start").collect();
    let mut path: Vec<SimStep> = Vec::new();
    let mut findings: Vec<Value> = Vec::new();
    let mut intents_iter = intents.iter();
    let mut queue: Vec<usize> = starts.clone();
    let mut seen: usize = 0;
    let mut awaiting_human: Option<String> = None;
    let mut terminated: Option<String> = None;
    let mut reached_end = false;

    while let Some(u) = queue.pop() {
        seen += 1;
        if seen > MAX_WALK_STEPS {
            terminated = Some(format!(
                "走查超过 {} 步上限——疑似循环，已截断",
                MAX_WALK_STEPS
            ));
            break;
        }
        let ty = node_type(nodes, u).to_string();
        if ty == "end" {
            reached_end = true;
            path.push(SimStep {
                node_id: node_key(nodes, u),
                label: node_label(nodes, u).to_string(),
                node_type: ty.clone(),
                decision: "到达 end".into(),
            });
            continue;
        }
        let edges_u = &out[u];
        // 人工节点：等待意图步骤
        if node_kind_is_action(&ty) {
            let intent = intents_iter.next();
            let Some(intent) = intent else {
                awaiting_human = Some(node_key(nodes, u));
                path.push(SimStep {
                    node_id: node_key(nodes, u),
                    label: node_label(nodes, u).to_string(),
                    node_type: ty.clone(),
                    decision: "等待审批裁决".into(),
                });
                break;
            };
            if intent.node != node_key(nodes, u) {
                terminated = Some(format!(
                    "意图节点 '{}' 与当前人工节点 '{}' 不匹配——replay 序列错误",
                    intent.node,
                    node_key(nodes, u)
                ));
                break;
            }
            let pick = intent
                .edge
                .unwrap_or_else(|| edges_u.iter().position(|e| e.cond.is_none()).unwrap_or(0));
            let Some(e) = edges_u.get(pick) else {
                terminated = Some(format!(
                    "节点 '{}' 出边序号 {} 越界（共 {} 条）",
                    node_key(nodes, u),
                    pick,
                    edges_u.len()
                ));
                break;
            };
            path.push(SimStep {
                node_id: node_key(nodes, u),
                label: node_label(nodes, u).to_string(),
                node_type: ty.clone(),
                decision: format!(
                    "人工裁决 → {}",
                    e.label.clone().unwrap_or_else(|| format!("#{}", pick))
                ),
            });
            queue.push(e.to);
            continue;
        }
        // condition/decision：ctx 求值选边
        if node_kind_is_condition(&ty) || node_kind_is_decision(&ty) {
            let mut targets: Vec<usize> = Vec::new();
            let mut stopped: Option<String> = None;
            if node_kind_is_decision(&ty) {
                let dmn_val = nodes[u].get("dmn").cloned();
                match dmn_val.and_then(|d| crate::dmn::parse_dmn(&d)) {
                    Some(table) => {
                        let eval = |expr: &str, c: &serde_json::Map<String, Value>| {
                            crate::advance::eval_flow_condition(expr, c).map_err(|e| e.to_string())
                        };
                        match crate::dmn::evaluate_dmn(&table, ctx, eval) {
                            crate::dmn::DmnDecision::Output(route, _) => {
                                targets = edges_u
                                    .iter()
                                    .filter(|e| e.label.as_deref() == Some(route.as_str()))
                                    .map(|e| e.to)
                                    .collect();
                                if targets.is_empty() {
                                    stopped = Some(format!(
                                        "决策输出 '{}' 无匹配出边（label 失配）",
                                        route
                                    ));
                                }
                            }
                            crate::dmn::DmnDecision::Outputs(routes, _) => {
                                for route in &routes {
                                    for e in edges_u
                                        .iter()
                                        .filter(|e| e.label.as_deref() == Some(route.as_str()))
                                    {
                                        targets.push(e.to);
                                    }
                                }
                                if targets.is_empty() {
                                    stopped = Some("决策多输出均无匹配出边".to_string());
                                }
                            }
                            crate::dmn::DmnDecision::Violation(v) => {
                                stopped = Some(format!("决策违例（fail-closed）：{}", v));
                            }
                        }
                    }
                    None => {
                        stopped = Some("decision 节点缺 timeline.dmn 结构".to_string());
                    }
                }
            } else {
                // condition：对全部出边按 ctx 求值（inclusive；routing exclusive 语义近似取首中）
                let routing = nodes[u]
                    .get("routing")
                    .and_then(|v| v.as_str())
                    .unwrap_or("inclusive");
                let mut any_eval_err = None;
                for (ei, e) in edges_u.iter().enumerate() {
                    let Some(cond) = &e.cond else {
                        targets.push(e.to);
                        continue;
                    };
                    match crate::advance::eval_flow_condition(cond, ctx) {
                        Ok(true) => {
                            targets.push(e.to);
                            if routing == "exclusive" {
                                break;
                            }
                        }
                        Ok(false) => {}
                        Err(err) => {
                            any_eval_err = Some((ei, err));
                            break;
                        }
                    }
                }
                if let Some((ei, err)) = any_eval_err {
                    stopped = Some(format!(
                        "条件 '{}' 求值失败（fail-closed 阻断）：{}",
                        edges_u[ei].cond.as_deref().unwrap_or(""),
                        err
                    ));
                }
            }
            path.push(SimStep {
                node_id: node_key(nodes, u),
                label: node_label(nodes, u).to_string(),
                node_type: ty.clone(),
                decision: match &stopped {
                    Some(s) => s.clone(),
                    None => format!("自动路由 → {} 条边", targets.len()),
                },
            });
            if let Some(s) = stopped {
                terminated = Some(s);
                break;
            }
            if targets.is_empty() {
                // 无命中且无兜底（无条件边已在上面加入 targets）
                terminated = Some(format!(
                    "节点 '{}' 全部出边条件在样例上下文下均不命中——空扇出，未达 end",
                    node_label(nodes, u)
                ));
                findings.push(serde_json::json!({
                    "code": "SIM_EMPTY_FANOUT",
                    "node": node_key(nodes, u),
                    "message": "空扇出（样例上下文下无可行出边）"
                }));
                break;
            }
            for t in targets {
                queue.push(t);
            }
            continue;
        }
        // 其余（cc/parallel/gate/subflow/loop/branch 近似）：无条件优先的全出边入队
        // 先推无条件/无 label 边；parallel/cc 全推；loop 近似取首条非回边
        path.push(SimStep {
            node_id: node_key(nodes, u),
            label: node_label(nodes, u).to_string(),
            node_type: ty.clone(),
            decision: "通过（自动节点近似推进）".into(),
        });
        for e in edges_u {
            queue.push(e.to);
        }
    }
    SimResult {
        path,
        findings,
        reached_end,
        terminated,
        awaiting_human,
    }
}

// ───────────────────────────── endpoint ─────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SimulateRequest {
    /// 样例实体上下文（字段 → JSON 值）
    #[serde(default)]
    pub context: serde_json::Map<String, Value>,
    /// 渐进 replay 意图序列（已执行部分；无 = 从 start 开始）
    #[serde(default)]
    pub steps: Vec<SimIntent>,
}

/// `POST /approval-flows/{id}/simulate`（字面路径，须先于 CRUD scope 注册）
pub async fn simulate(
    pool: web::Data<sqlx::PgPool>,
    path: web::Path<i64>,
    body: web::Json<SimulateRequest>,
) -> Result<HttpResponse, AliothError> {
    let flow_id = path.into_inner();
    let meta: Option<Value> = sqlx::query_scalar(
        r#"SELECT meta FROM isahl.zc_id_process WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(flow_id)
    .fetch_optional(pool.get_ref())
    .await
    .map_err(|e| AliothError::Database(e.to_string()))?
    .flatten();
    let parsed =
        meta.ok_or_else(|| AliothError::NotFound(format!("ApprovalFlow {} not found", flow_id)))?;
    let nodes = match &parsed {
        Value::Object(map) => map.get("nodes").and_then(|v| v.as_array()).cloned(),
        Value::Array(arr) => Some(arr.clone()),
        _ => None,
    }
    .ok_or_else(|| AliothError::Validation {
        field: "meta".into(),
        message: "flow meta 缺 nodes——无法模拟（未保存设计图？）".into(),
    })?;
    let edges = match &parsed {
        Value::Object(map) => map.get("edges").and_then(|v| v.as_array()).cloned(),
        _ => None,
    };
    let result = run_walk(&nodes, edges.as_deref(), &body.context, &body.steps);
    Ok(HttpResponse::Ok().json(ApiResponse::success(result)))
}

pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.route("/approval-flows/{id}/simulate", web::post().to(simulate));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn start() -> Value {
        json!({ "id": "s", "type": "start", "label": "开始", "next": [{ "to": 1 }] })
    }
    fn approve(id: &str, label: &str, next: Value) -> Value {
        json!({ "id": id, "type": "approve", "label": label, "next": next }) // id-json-ok
    }
    fn end() -> Value {
        json!({ "id": "e", "type": "end", "label": "结束" })
    }

    #[test]
    fn walk_with_intents_reaches_end() {
        let nodes = vec![
            start(),
            approve("a1", "部门审批", json!([{ "to": 2 }])),
            end(),
        ];
        let intents = vec![SimIntent {
            node: "a1".into(),
            edge: Some(0),
        }];
        let ctx: serde_json::Map<String, Value> = Default::default();
        let r = run_walk(&nodes, None, &ctx, &intents);
        assert!(r.reached_end, "{r:?}");
        assert!(r.awaiting_human.is_none());
        assert!(r.terminated.is_none());
    }

    #[test]
    fn walk_waits_for_human_without_intent() {
        let nodes = vec![
            start(),
            approve("a1", "部门审批", json!([{ "to": 2 }])),
            end(),
        ];
        let ctx: serde_json::Map<String, Value> = Default::default();
        let r = run_walk(&nodes, None, &ctx, &[]);
        assert_eq!(r.awaiting_human.as_deref(), Some("a1"));
        assert!(!r.reached_end);
    }

    #[test]
    fn condition_ctx_routes_and_missing_field_stops() {
        let cond = json!({
            "id": "c1", "type": "condition", "label": "金额判断",
            "routing": "exclusive",
            "next": [
                { "to": 3, "cond": "amount > 1000", "label": "big" },
                { "to": 4, "cond": "amount <= 1000", "label": "small" }
            ]
        });
        let nodes = vec![
            start(),
            approve("a1", "部门审批", json!([{ "to": 2 }])),
            cond,
            end(),
            end(),
        ];
        let mut nodes = nodes;
        let _ = &mut nodes;
        // 重排：s(0) a1(1) c1(2) e_big(3) e_small(4)
        let mut ctx: serde_json::Map<String, Value> = Default::default();
        ctx.insert("amount".into(), json!(2000));
        let intents = vec![SimIntent {
            node: "a1".into(),
            edge: Some(0),
        }];
        let r = run_walk(&nodes, None, &ctx, &intents);
        // BFS 顺序到达 c1 → amount=2000 → big 边 → end(3)
        assert!(r.reached_end, "{r:?}");
        assert!(r.terminated.is_none(), "{r:?}");
        // 缺字段 fail-closed
        let ctx2: serde_json::Map<String, Value> = Default::default();
        let r2 = run_walk(&nodes, None, &ctx2, &intents);
        assert!(r2.terminated.is_some(), "{r2:?}");
    }

    #[test]
    fn empty_fanout_recorded() {
        // condition 两条条件边在 ctx 下均 false（无兜底）→ 空扇出终止并记录
        let cond = json!({
            "id": "c1", "type": "condition", "label": "判断",
            "next": [
                { "to": 2, "cond": "amount > 1000", "label": "big" },
                { "to": 3, "cond": "amount < 100", "label": "small" }
            ]
        });
        let nodes = vec![
            start(),
            approve("a1", "部门审批", json!([{ "to": 2 }])),
            cond,
            end(),
            end(),
        ];
        let mut ctx: serde_json::Map<String, Value> = Default::default();
        ctx.insert("amount".into(), json!(500));
        let intents = vec![SimIntent {
            node: "a1".into(),
            edge: Some(0),
        }];
        let r = run_walk(&nodes, None, &ctx, &intents);
        assert!(r.terminated.is_some(), "{r:?}");
        assert!(
            r.findings.iter().any(|f| f["code"] == "SIM_EMPTY_FANOUT"),
            "{r:?}"
        );
        assert!(!r.reached_end);
    }
}
