//! DMN 决策表（fix-flow-designer-editing-gaps B3 + extend-dmn-decision-table-full）
//!
//! 与前端 `Framework/frontend/components/.../flow/dmn.ts` 同构（双端语义镜像，
//! 集成测试锚定）：
//! - 单元格为空串/null = 通配（该列恒命中）；非空 = 表达式（统一 expr 文法，
//!   FEEL 子集：between/区间/null 判空）；
//! - 多输出列（可选）：outputs 列定义与规则 output 等长；首列 = 路由列；
//!   无 outputs = 存量单列路由（output 为字符串，parse 归一为单元素数组）。
//! - hitPolicy：
//!   - FIRST：按行序首个命中；UNIQUE：恰好一行（多命中违例）；
//!   - ANY：多行命中须输出一致；PRIORITY：取 priority 最小（缺省=行序）；
//!   - COLLECT：聚合全部命中（aggregation list/count/sum/min/max，缺省 list）；
//! - 无命中/违例/求值错误均返回 Violation（fail-closed，不静默走默认边）。

/// 决策表运行时视图（发布物化于节点 timeline.dmn）
pub struct DmnTable {
    pub hit_policy: String,
    pub inputs: Vec<String>,
    /// 输出列定义（空 = 存量单列路由；非空时规则 output 与之等长）
    pub outputs: Vec<String>,
    /// COLLECT 聚合方式（仅 hit_policy=COLLECT；None/缺省 = list）
    pub aggregation: Option<String>,
    pub rules: Vec<DmnRule>,
}

pub struct DmnRule {
    /// 与 inputs 等长；None/空串 = 通配
    pub cells: Vec<Option<String>>,
    /// 输出值（parse 归一：字符串 → 单元素数组；数组原样）。首元素 = 路由列值
    pub output: Vec<String>,
    /// PRIORITY 优先级（正整数，越小越优先；None = 按行序）
    pub priority: Option<i64>,
}

/// timeline.dmn JSON → 运行时视图；结构非法返回 None（调用方按 legacy 容错）
pub fn parse_dmn(value: &serde_json::Value) -> Option<DmnTable> {
    let hit_policy = value.get("hitPolicy")?.as_str()?.to_string();
    let inputs: Vec<String> = value
        .get("inputs")?
        .as_array()?
        .iter()
        .map(|i| {
            i.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let outputs: Vec<String> = value
        .get("outputs")
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .map(|i| {
                    i.get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or_default()
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    let aggregation = value
        .get("aggregation")
        .and_then(|a| a.as_str())
        .map(str::to_string);
    let rules: Vec<DmnRule> = value
        .get("rules")?
        .as_array()?
        .iter()
        .map(|r| DmnRule {
            cells: r
                .get("match")
                .and_then(|m| m.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|c| {
                            let s = c.as_str().unwrap_or_default();
                            if s.trim().is_empty() {
                                None
                            } else {
                                Some(s.to_string())
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
            output: match r.get("output") {
                Some(serde_json::Value::Array(arr)) => arr
                    .iter()
                    .map(|o| o.as_str().unwrap_or_default().to_string())
                    .collect(),
                Some(v) => vec![v.as_str().unwrap_or_default().to_string()],
                None => Vec::new(),
            },
            priority: r.get("priority").and_then(|p| p.as_i64()),
        })
        .collect();
    Some(DmnTable {
        hit_policy,
        inputs,
        outputs,
        aggregation,
        rules,
    })
}

/// 决策求值结果
#[derive(Debug, Clone, PartialEq)]
pub enum DmnDecision {
    /// (路由输出值, 命中行数)——单输出（FIRST/UNIQUE/ANY/PRIORITY/COLLECT 数值聚合）
    Output(String, usize),
    /// (输出去重列表, 命中行数)——COLLECT-list 多输出并行扇出
    Outputs(Vec<String>, usize),
    /// fail-closed 违例描述（含定位信息）
    Violation(String),
}

/// 求值决策表。`eval` 为统一表达式求值器（advance::eval_flow_condition），
/// 返回 Err 时视为该 cell 求值失败（fail-closed Violation）。
pub fn evaluate_dmn<F>(
    table: &DmnTable,
    ctx: &serde_json::Map<String, serde_json::Value>,
    eval: F,
) -> DmnDecision
where
    F: Fn(&str, &serde_json::Map<String, serde_json::Value>) -> Result<bool, String>,
{
    struct Hit {
        index: usize,
        priority: i64,
        route: String,
    }
    let mut hits: Vec<Hit> = Vec::new();
    for (ri, rule) in table.rules.iter().enumerate() {
        let mut hit = true;
        for (ci, cell) in rule.cells.iter().enumerate() {
            let Some(expr) = cell else { continue };
            match eval(expr, ctx) {
                Ok(true) => {}
                Ok(false) => {
                    hit = false;
                    break;
                }
                Err(err) => {
                    let col = table.inputs.get(ci).map(String::as_str).unwrap_or("");
                    return DmnDecision::Violation(format!(
                        "规则 {} 列「{}」表达式「{}」求值失败：{}",
                        ri + 1,
                        col,
                        expr,
                        err
                    ));
                }
            }
        }
        if hit {
            hits.push(Hit {
                index: ri,
                priority: rule.priority.unwrap_or(ri as i64 + 1),
                route: rule.output.first().cloned().unwrap_or_default(),
            });
        }
    }
    if hits.is_empty() {
        return DmnDecision::Violation(
            "决策表无规则命中（fail-closed，不静默走默认边）".to_string(),
        );
    }

    let distinct_of = |items: &[&String]| -> Vec<String> {
        let mut d: Vec<String> = Vec::new();
        for m in items {
            if !d.contains(m) {
                d.push((*m).clone());
            }
        }
        d
    };

    match table.hit_policy.as_str() {
        "FIRST" => DmnDecision::Output(hits[0].route.clone(), 1),
        "UNIQUE" => {
            if hits.len() > 1 {
                let set =
                    distinct_of(&hits.iter().map(|h| &h.route).collect::<Vec<_>>()).join(" / ");
                DmnDecision::Violation(format!(
                    "UNIQUE 策略下 {} 条规则命中（输出集：{}）",
                    hits.len(),
                    set
                ))
            } else {
                DmnDecision::Output(hits[0].route.clone(), 1)
            }
        }
        "PRIORITY" => {
            // 命中集按 priority 升序取最小者；同 priority 多命中异输出 → 违例
            let min_p = hits.iter().map(|h| h.priority).min().unwrap_or(0);
            let top: Vec<&Hit> = hits.iter().filter(|h| h.priority == min_p).collect();
            if top.len() > 1 {
                let distinct = distinct_of(&top.iter().map(|h| &h.route).collect::<Vec<_>>());
                if distinct.len() > 1 {
                    return DmnDecision::Violation(format!(
                        "PRIORITY 策略下优先级 {min_p} 有 {} 条命中且输出不同（{}）",
                        top.len(),
                        distinct.join(" / ")
                    ));
                }
                return DmnDecision::Output(distinct[0].clone(), top.len());
            }
            DmnDecision::Output(top[0].route.clone(), 1)
        }
        "COLLECT" => {
            let agg = table.aggregation.as_deref().unwrap_or("list");
            let outputs: Vec<&String> = hits.iter().map(|h| &h.route).collect();
            match agg {
                "list" => DmnDecision::Outputs(distinct_of(&outputs), hits.len()),
                "count" => DmnDecision::Output(hits.len().to_string(), hits.len()),
                "sum" | "min" | "max" => {
                    let mut nums: Vec<f64> = Vec::with_capacity(outputs.len());
                    for (i, out) in outputs.iter().enumerate() {
                        match out.parse::<f64>() {
                            Ok(n) => nums.push(n),
                            Err(_) => {
                                return DmnDecision::Violation(format!(
                                    "COLLECT aggregation={agg} 规则 {} 输出「{}」非数值",
                                    hits[i].index + 1,
                                    out
                                ))
                            }
                        }
                    }
                    let r = match agg {
                        "sum" => nums.iter().sum::<f64>(),
                        "min" => nums.iter().cloned().fold(f64::INFINITY, f64::min),
                        _ => nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                    };
                    // 整数结果不带小数尾（10.0 → "10"）
                    let out = if r.fract() == 0.0 {
                        format!("{}", r as i64)
                    } else {
                        format!("{r}")
                    };
                    DmnDecision::Output(out, hits.len())
                }
                other => DmnDecision::Violation(format!(
                    "COLLECT 非法聚合方式「{other}」（须 list/count/sum/min/max）"
                )),
            }
        }
        "ANY" => {
            let distinct = distinct_of(&hits.iter().map(|h| &h.route).collect::<Vec<_>>());
            if distinct.len() > 1 {
                let set = distinct.join(" / ");
                DmnDecision::Violation(format!("ANY 策略下命中规则输出不一致（{set}）"))
            } else {
                DmnDecision::Output(distinct[0].clone(), hits.len())
            }
        }
        other => DmnDecision::Violation(format!(
            "非法命中策略「{other}」（须 UNIQUE/FIRST/ANY/PRIORITY/COLLECT）"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试桩求值器：支持 `key == 'v'` 字面匹配与 `key >= N` 数值比较
    /// （命中策略逻辑与引擎解耦；amount 型测试用数值比较）
    fn stub_eval(
        expr: &str,
        ctx: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<bool, String> {
        // "amount >= 10000" → 数值比较（ctx 取 amount）
        if let Some((k, v)) = expr.split_once(">=") {
            let k = k.trim();
            let threshold: f64 = v.trim().parse().map_err(|_| "stub: bad threshold")?;
            let val = ctx
                .get(k)
                .and_then(|x| x.as_f64())
                .ok_or_else(|| format!("stub: missing numeric {k}"))?;
            return Ok(val >= threshold);
        }
        // "code == 'C1'" → (key=code, v=C1)；ctx 取 key 与 v 比较
        if let Some((k, v)) = expr.split_once("==") {
            let k = k.trim();
            let v = v.trim().trim_matches('\'').trim_matches('"');
            let actual = ctx.get(k).and_then(|x| x.as_str()).unwrap_or("");
            return Ok(actual == v);
        }
        Err(format!("stub: unsupported expr {expr}"))
    }

    fn table(policy: &str, rules: Vec<(&[&str], &str)>) -> DmnTable {
        DmnTable {
            hit_policy: policy.to_string(),
            inputs: vec!["code".to_string()],
            outputs: Vec::new(),
            aggregation: None,
            rules: rules
                .into_iter()
                .map(|(cells, output)| DmnRule {
                    cells: cells
                        .iter()
                        .map(|c| {
                            if c.is_empty() {
                                None
                            } else {
                                Some(c.to_string())
                            }
                        })
                        .collect(),
                    output: vec![output.to_string()],
                    priority: None,
                })
                .collect(),
        }
    }

    fn empty_ctx() -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }

    /// ctx 含 code=C1（stub 字面比较命中用）
    fn c1_ctx() -> serde_json::Map<String, serde_json::Value> {
        let mut m = serde_json::Map::new();
        m.insert("code".into(), serde_json::json!("C1"));
        m
    }

    #[test]
    fn first_policy_takes_ordered_match_with_wildcard_fallback() {
        let t = table("FIRST", vec![(&["code == 'C1'"], "go-a"), (&[""], "go-b")]);
        // 具体命中（stub 字面 == 与 ctx 值比较）
        let c1_ctx = {
            let mut m = serde_json::Map::new();
            m.insert("code".into(), serde_json::json!("C1"));
            m
        };
        assert_eq!(
            evaluate_dmn(&t, &c1_ctx, stub_eval),
            DmnDecision::Output("go-a".into(), 1)
        );
        // 通配兜底行：无具体命中时取行序首个通配
        let no_match_ctx = {
            let mut m = serde_json::Map::new();
            m.insert("code".into(), serde_json::json!("NOPE"));
            m
        };
        assert_eq!(
            evaluate_dmn(&t, &no_match_ctx, stub_eval),
            DmnDecision::Output("go-b".into(), 1)
        );
    }

    #[test]
    fn unique_multi_match_is_violation_with_details() {
        let t = table("UNIQUE", vec![(&["code == 'C1'"], "a"), (&[""], "b")]);
        assert!(matches!(
            evaluate_dmn(&t, &c1_ctx(), stub_eval),
            DmnDecision::Violation(m) if m.contains("UNIQUE") && m.contains("2 条")
        ));
    }

    #[test]
    fn any_policy_requires_consistent_output() {
        let consistent = table("ANY", vec![(&["code == 'C1'"], "x"), (&[""], "x")]);
        assert_eq!(
            evaluate_dmn(&consistent, &c1_ctx(), stub_eval),
            DmnDecision::Output("x".into(), 2)
        );
        let inconsistent = table("ANY", vec![(&["code == 'C1'"], "x"), (&[""], "y")]);
        assert!(matches!(
            evaluate_dmn(&inconsistent, &c1_ctx(), stub_eval),
            DmnDecision::Violation(m) if m.contains("不一致")
        ));
    }

    #[test]
    fn no_match_and_eval_error_are_violations() {
        let t = table("FIRST", vec![(&["code == 'NOPE'"], "x")]);
        assert!(matches!(
            evaluate_dmn(&t, &empty_ctx(), stub_eval),
            DmnDecision::Violation(m) if m.contains("无规则命中")
        ));
        let bad = table("FIRST", vec![(&["bad syntax"], "x")]);
        assert!(matches!(
            evaluate_dmn(&bad, &empty_ctx(), stub_eval),
            DmnDecision::Violation(m) if m.contains("求值失败")
        ));
    }

    // ── 扩展策略（extend-dmn-decision-table-full）──
    #[test]
    fn priority_takes_lowest_priority_hit_and_defaults_to_row_order() {
        // 显式 priority 反行序胜出
        let t = DmnTable {
            hit_policy: "PRIORITY".into(),
            inputs: vec!["amount".into()],
            outputs: Vec::new(),
            aggregation: None,
            rules: vec![
                DmnRule {
                    cells: vec![Some("amount >= 10000".into())],
                    output: vec!["big".into()],
                    priority: Some(9),
                },
                DmnRule {
                    cells: vec![None],
                    output: vec!["small".into()],
                    priority: Some(1),
                },
            ],
        };
        let ctx = {
            let mut m = serde_json::Map::new();
            m.insert("amount".into(), serde_json::json!(50000));
            m
        };
        assert_eq!(
            evaluate_dmn(&t, &ctx, stub_eval),
            DmnDecision::Output("small".into(), 1)
        );
        // 缺省 priority（None）= 行序（等价 FIRST）
        let row = DmnTable {
            hit_policy: "PRIORITY".into(),
            inputs: vec!["amount".into()],
            outputs: Vec::new(),
            aggregation: None,
            rules: vec![
                DmnRule {
                    cells: vec![Some("amount >= 10000".into())],
                    output: vec!["big".into()],
                    priority: None,
                },
                DmnRule {
                    cells: vec![None],
                    output: vec!["small".into()],
                    priority: None,
                },
            ],
        };
        assert_eq!(
            evaluate_dmn(&row, &ctx, stub_eval),
            DmnDecision::Output("big".into(), 1)
        );
    }

    #[test]
    fn priority_same_priority_multi_hit_diff_output_violation() {
        let t = DmnTable {
            hit_policy: "PRIORITY".into(),
            inputs: vec!["amount".into()],
            outputs: Vec::new(),
            aggregation: None,
            rules: vec![
                DmnRule {
                    cells: vec![Some("amount >= 10000".into())],
                    output: vec!["big".into()],
                    priority: Some(5),
                },
                DmnRule {
                    cells: vec![None],
                    output: vec!["small".into()],
                    priority: Some(5),
                },
            ],
        };
        let ctx = {
            let mut m = serde_json::Map::new();
            m.insert("amount".into(), serde_json::json!(50000));
            m
        };
        assert!(matches!(
            evaluate_dmn(&t, &ctx, stub_eval),
            DmnDecision::Violation(m) if m.contains("PRIORITY") && m.contains("优先级 5")
        ));
    }

    #[test]
    fn collect_list_dedup_multi_output() {
        let t = DmnTable {
            hit_policy: "COLLECT".into(),
            inputs: vec!["flag".into()],
            outputs: Vec::new(),
            aggregation: Some("list".into()),
            rules: vec![
                DmnRule {
                    cells: vec![None],
                    output: vec!["go-a".into()],
                    priority: None,
                },
                DmnRule {
                    cells: vec![None],
                    output: vec!["go-b".into()],
                    priority: None,
                },
                DmnRule {
                    cells: vec![None],
                    output: vec!["go-a".into()],
                    priority: None,
                },
            ],
        };
        assert_eq!(
            evaluate_dmn(&t, &empty_ctx(), stub_eval),
            DmnDecision::Outputs(vec!["go-a".into(), "go-b".into()], 3)
        );
    }

    #[test]
    fn collect_numeric_aggregation_and_non_numeric_violation() {
        let mk = |agg: &str| DmnTable {
            hit_policy: "COLLECT".into(),
            inputs: vec!["flag".into()],
            outputs: Vec::new(),
            aggregation: Some(agg.into()),
            rules: vec![
                DmnRule {
                    cells: vec![None],
                    output: vec!["10".into()],
                    priority: None,
                },
                DmnRule {
                    cells: vec![None],
                    output: vec!["30".into()],
                    priority: None,
                },
                DmnRule {
                    cells: vec![None],
                    output: vec!["20".into()],
                    priority: None,
                },
            ],
        };
        assert_eq!(
            evaluate_dmn(&mk("count"), &empty_ctx(), stub_eval),
            DmnDecision::Output("3".into(), 3)
        );
        assert_eq!(
            evaluate_dmn(&mk("sum"), &empty_ctx(), stub_eval),
            DmnDecision::Output("60".into(), 3)
        );
        assert_eq!(
            evaluate_dmn(&mk("min"), &empty_ctx(), stub_eval),
            DmnDecision::Output("10".into(), 3)
        );
        assert_eq!(
            evaluate_dmn(&mk("max"), &empty_ctx(), stub_eval),
            DmnDecision::Output("30".into(), 3)
        );

        let bad = DmnTable {
            hit_policy: "COLLECT".into(),
            inputs: vec!["flag".into()],
            outputs: Vec::new(),
            aggregation: Some("sum".into()),
            rules: vec![
                DmnRule {
                    cells: vec![None],
                    output: vec!["10".into()],
                    priority: None,
                },
                DmnRule {
                    cells: vec![None],
                    output: vec!["abc".into()],
                    priority: None,
                },
            ],
        };
        assert!(matches!(
            evaluate_dmn(&bad, &empty_ctx(), stub_eval),
            DmnDecision::Violation(m) if m.contains("非数值")
        ));
    }

    #[test]
    fn multi_output_column_routes_first_col() {
        let t = DmnTable {
            hit_policy: "FIRST".into(),
            inputs: vec!["amount".into()],
            outputs: vec!["route".into(), "band".into()],
            aggregation: None,
            rules: vec![DmnRule {
                cells: vec![Some("amount >= 1000".into())],
                output: vec!["go-a".into(), "big".into()],
                priority: None,
            }],
        };
        let ctx = {
            let mut m = serde_json::Map::new();
            m.insert("amount".into(), serde_json::json!(5000));
            m
        };
        assert_eq!(
            evaluate_dmn(&t, &ctx, stub_eval),
            DmnDecision::Output("go-a".into(), 1)
        );
    }

    #[test]
    fn parse_dmn_normalizes_wildcards() {
        let v: serde_json::Value = serde_json::json!({
            "hitPolicy": "FIRST",
            "inputs": [{"name": "amount"}],
            "rules": [{"match": ["", null, "amount >= 1"], "output": "ok"}]
        });
        let t = parse_dmn(&v).expect("valid dmn json");
        assert_eq!(t.hit_policy, "FIRST");
        assert_eq!(t.rules.len(), 1);
        assert_eq!(
            t.rules[0].cells,
            vec![None, None, Some("amount >= 1".to_string())]
        );
        assert_eq!(t.rules[0].output, vec!["ok".to_string()]);
    }

    #[test]
    fn parse_dmn_multi_output_and_new_fields() {
        let v: serde_json::Value = serde_json::json!({
            "hitPolicy": "COLLECT",
            "inputs": [{"name": "amount"}],
            "outputs": [{"name": "route"}, {"name": "band"}],
            "aggregation": "sum",
            "rules": [
                {"match": ["amount >= 1"], "output": ["go-a", "big"], "priority": 3}
            ]
        });
        let t = parse_dmn(&v).expect("valid dmn json");
        assert_eq!(t.hit_policy, "COLLECT");
        assert_eq!(t.outputs, vec!["route".to_string(), "band".to_string()]);
        assert_eq!(t.aggregation.as_deref(), Some("sum"));
        assert_eq!(
            t.rules[0].output,
            vec!["go-a".to_string(), "big".to_string()]
        );
        assert_eq!(t.rules[0].priority, Some(3));
    }
}
