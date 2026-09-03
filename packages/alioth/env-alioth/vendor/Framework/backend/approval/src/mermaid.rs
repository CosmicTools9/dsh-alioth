//! 设计图 JSON → mermaid flowchart 文本生成引擎
//! （migrate-flow-design-storage-to-meta-mermaid：流程整体结构存储于
//! `zc_id_process.mermaid`，流程保存时由本引擎自动生成，幂等重写）。
//!
//! 输入为设计器 serializeFlow 信封（`validate_graph` 同源契约）：
//! - 节点数组 `nodes: [{id, type, label, ...}]`
//! - 边两种形态兼容：顶层 `edges: [{from, to, label?, cond?}]`（下标），
//!   或节点内 `next: [{to, label?, cond?}]`（下标索引边）
//!
//! 输出为合法 mermaid `flowchart TD` 文本；空图（无节点）返回空字符串。

use serde_json::Value;

/// 节点类型 → mermaid 形状（前缀/后缀包装）
fn shape_of(node_type: &str) -> (&'static str, &'static str) {
    match node_type {
        "start" | "end" => ("([\"", "\"])"),
        "condition" => ("{\"", "\"}"),
        "cc" => (">\"", "\"]"),
        "parallel" => ("[/\"", "\"/]"),
        "branch" => ("[\"", "\"]"),
        "loop" => ("((\"", "\"))"),
        "subflow" => ("[[\"", "\"]]"),
        _ => ("[\"", "\"]"),
    }
}

/// label 转义：引号转义、控制字符剥离（mermaid 双引号字面量安全）
fn escape_label(s: &str) -> String {
    s.replace('"', "\\\"").replace(['\n', '\r', '\t'], " ")
}

/// 节点 mermaid id：复用设计图 id，非 [A-Za-z0-9_] 字符替换为 `_`；
/// 缺失时按序生成；重复追加后缀去重（防御）。
fn node_id(raw: Option<&str>, idx: usize, used: &mut std::collections::HashSet<String>) -> String {
    let base = raw
        .map(|s| {
            let cleaned: String = s
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            if cleaned.is_empty() {
                format!("n{idx}")
            } else {
                cleaned
            }
        })
        .unwrap_or_else(|| format!("n{idx}"));
    if used.insert(base.clone()) {
        base
    } else {
        let mut i = 1;
        loop {
            let cand = format!("{base}_{i}");
            if used.insert(cand.clone()) {
                return cand;
            }
            i += 1;
        }
    }
}

/// 设计图 JSON → mermaid flowchart TD 文本
pub fn graph_to_mermaid(parsed: &Value) -> String {
    let nodes = match parsed.get("nodes").and_then(Value::as_array) {
        Some(n) if !n.is_empty() => n,
        _ => return String::new(),
    };

    let mut used = std::collections::HashSet::new();
    let mut lines: Vec<String> = Vec::with_capacity(nodes.len() * 2 + 1);
    lines.push("flowchart TD".to_string());

    // 节点下标 → mermaid id（先统一解析，供边引用）
    let mut ids: Vec<String> = Vec::with_capacity(nodes.len());
    for (idx, node) in nodes.iter().enumerate() {
        let ntype = node.get("type").and_then(Value::as_str).unwrap_or("");
        let label = node
            .get("label")
            .and_then(Value::as_str)
            .map(escape_label)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| ntype.to_string());
        let (pre, post) = shape_of(ntype);
        let id = node_id(node.get("id").and_then(Value::as_str), idx, &mut used);
        lines.push(format!("  {id}{pre}{label}{post}"));
        ids.push(id);
    }

    // 边（顶层 edges 数组；from/to 为节点下标）
    let mut edge_lines: Vec<String> = Vec::new();
    let mut emit = |from: usize, to: usize, label: Option<&str>| {
        if from >= ids.len() || to >= ids.len() {
            return;
        }
        match label.map(escape_label) {
            Some(l) if !l.is_empty() => {
                edge_lines.push(format!("  {} -->|\"{}\"| {}", ids[from], l, ids[to]))
            }
            _ => edge_lines.push(format!("  {} --> {}", ids[from], ids[to])),
        }
    };
    if let Some(edges) = parsed.get("edges").and_then(Value::as_array) {
        for e in edges {
            let from = e.get("from").and_then(Value::as_i64);
            let to = e.get("to").and_then(Value::as_i64);
            let label = e
                .get("label")
                .and_then(Value::as_str)
                .or_else(|| e.get("cond").and_then(Value::as_str));
            if let (Some(f), Some(t)) = (from, to) {
                emit(f as usize, t as usize, label);
            }
        }
    }
    // 节点内 next 索引边（serializeFlow 形态）
    for (idx, node) in nodes.iter().enumerate() {
        if let Some(next) = node.get("next").and_then(Value::as_array) {
            for e in next {
                let to = e.get("to").and_then(Value::as_i64);
                let label = e
                    .get("label")
                    .and_then(Value::as_str)
                    .or_else(|| e.get("cond").and_then(Value::as_str));
                if let Some(t) = to {
                    emit(idx, t as usize, label);
                }
            }
        }
    }
    lines.extend(edge_lines);
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_graph_returns_empty() {
        assert_eq!(graph_to_mermaid(&json!({"nodes": []})), "");
        assert_eq!(graph_to_mermaid(&json!({"nodes": null})), "");
    }

    #[test]
    fn node_shapes_and_edges_array() {
        let g = json!({
            "version": 1,
            "nodes": [
                {"id": "s1", "type": "start", "label": "开始"},
                {"id": "a1", "type": "approval", "label": "部门经理审批"},
                {"id": "c1", "type": "condition", "label": "金额 > 5000"},
                {"id": "p1", "type": "parallel", "label": "双线并行"},
                {"id": "cc1", "type": "cc", "label": "抄送专员"},
                {"id": "e1", "type": "end", "label": "结束"}
            ],
            "edges": [
                {"from": 0, "to": 1},
                {"from": 1, "to": 2, "label": "金额 > 5000"},
                {"from": 1, "to": 3, "cond": "否则"}
            ]
        });
        let out = graph_to_mermaid(&g);
        assert!(out.starts_with("flowchart TD\n"));
        assert!(out.contains("s1([\"开始\"])"));
        assert!(out.contains("a1[\"部门经理审批\"]"));
        assert!(out.contains("c1{\"金额 > 5000\"}"));
        assert!(out.contains("p1[/\"双线并行\"/]"));
        assert!(out.contains("cc1>\"抄送专员\"]"));
        assert!(out.contains("e1([\"结束\"])"));
        assert!(out.contains("s1 --> a1"));
        assert!(out.contains("a1 -->|\"金额 > 5000\"| c1"));
        assert!(out.contains("a1 -->|\"否则\"| p1"));
    }

    #[test]
    fn next_index_edges_serialize_flow_shape() {
        let g = json!({
            "version": 1,
            "nodes": [
                {"id": "s1", "type": "start", "label": "开始", "next": [{"to": 1}]},
                {"id": "a1", "type": "approval", "label": "审批", "next": [{"to": 2, "cond": "同意"}]},
                {"id": "e1", "type": "end", "label": "结束"}
            ]
        });
        let out = graph_to_mermaid(&g);
        assert!(out.contains("s1 --> a1"));
        assert!(out.contains("a1 -->|\"同意\"| e1"));
    }

    #[test]
    fn label_escape_and_id_sanitize() {
        let g = json!({
            "nodes": [
                {"id": "n 1!@#", "type": "approval", "label": "他说\"同意\""},
                {"id": "n 1!@#", "type": "approval", "label": "重复 id"}
            ]
        });
        let out = graph_to_mermaid(&g);
        assert!(out.contains("n_1___[\"他说\\\"同意\\\"\"]"));
        assert!(out.contains("n_1____1[\"重复 id\"]"));
    }

    #[test]
    fn fallback_id_and_label() {
        let g = json!({
            "nodes": [
                {"type": "approval"},
                {"id": "", "type": "cc"}
            ]
        });
        let out = graph_to_mermaid(&g);
        assert!(out.contains("n0[\"approval\"]"));
        assert!(out.contains("n1>\"cc\"]"));
    }
}
