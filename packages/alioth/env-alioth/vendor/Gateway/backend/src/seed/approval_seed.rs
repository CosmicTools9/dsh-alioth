//! 审批域种子：基态 + 「实现·范例」流程模板（publish 物化同构）+ 一致性自愈
//!
//! 节点模型契约（refactor-flow-node-operation-model / advance.rs 消费面）：
//! 每节点 = 事件载体行（`zc_id_even-approve`，code=图内编号，实例挂载体、节点解析
//! 经接线桥反查）+ 节点主体行（approve→`zc_id_oper-approve`，start/end→
//! `zc_id_oper-gate`；`_f_='实现'`/`_t_='范例'`，publish.rs 物化同形态）+
//! cate 绑定（`ck_cate-proc_op` find-or-create）+ `zc_id_operation_rr_event`
//! 接线桥 + `zc_id_process_rr_operation` 归属桥（code=图内编号、comments=节点
//! label、`next-ops` 对象形态 `[{"id":N}]`）。流程行 `meta.nodes` 承载 start
//! 定位契约（initiate_flow 解析 type='start' 的图内编号）。
//!
//! 覆盖：
//! 1. `zc_id_stus-approve` 审批基态种子（pending/approved/rejected）——引擎
//!    终态判定（`zc_id_lifecycle_r_primary-status` 桥 st.code）硬依赖。
//! 2. 四个「实现·范例」流程模板（FLOW-USER-REGISTER / FLOW-USER-VERIFY /
//!    FLOW-AUTHORIZATION / FLOW-VERCTRL），start→approve→end 线性 DAG。
//! 3. 注册审批双表一致性自检（oper-approve ↔ even-approve 补链 + 断链告警）
//!    + 模板绑定回填（tpl_id → approve 节点主体行）。
//!
//! uid 链码纪律：全部 INSERT 省略 `id` 列——DDL default（gen_next_uid(N)/
//! gen_next_zuid()）是唯一链码真相源；禁止硬编码表码（模型重编号即漂移），
//! 禁止对非 lifecycle 桥表显式调用 `gen_next_zuid()`。

use sqlx::PgPool;

use super::SeedStats;

/// 注册审批实例/事件 code（register.rs 双写契约，不变）
pub const REGISTRATION_APPROVAL_CODE: &str = "user-register-approval";
/// 注册审批流程模板 code
pub const REGISTRATION_FLOW_CODE: &str = "FLOW-USER-REGISTER";
/// 用户实名审核流程模板 code（zc_id_appr-user_verify 叶表）
pub const VERIFY_FLOW_CODE: &str = "FLOW-USER-VERIFY";
/// 访问授权流程模板 code（zc_id_appr-authorization 叶表）
pub const AUTHORIZATION_FLOW_CODE: &str = "FLOW-AUTHORIZATION";
/// 版控固化审批流程模板 code（ct-git verctrl freeze 审批实例 fk_process 对齐，
/// add-ct-git-vc-interop）
pub const VERCTRL_FLOW_CODE: &str = "FLOW-VERCTRL";
/// 外部主体入驻流程模板 code（add-dual-register-channels：
/// /auth/register/external 通道专用，与内部 FLOW-AUTHORIZATION 分流）
pub const EXTERNAL_SUBJECT_FLOW_CODE: &str = "FLOW-EXTERNAL-SUBJECT";
/// 实名审核事件 code（identity.rs 写入契约）
pub const USER_VERIFY_CODE: &str = "user-verify";
/// 访问授权事件 code（register.rs 双写契约，不变）
pub const AUTHORIZATION_CODE: &str = "user-register-approval";
/// 外部主体入驻审批事件 code（register.rs 外部通道写契约）
pub const EXTERNAL_SUBJECT_APPROVAL_CODE: &str = "external-subject-register-approval";

/// 注册审批 SLA 时长维度（zc_id_scal-duration）：72 小时
/// ——add-register-approval-closure 缺口 2：SLA 超时自动驳回的前提。
const REGISTRATION_SLA_HOURS: &str = "72h";
const REGISTRATION_SLA_HOURS_VALUE: f64 = 72.0;

/// 审批基态种子：code → notice
const APPROVAL_STATUS_SEEDS: &[(&str, &str)] = &[
    ("pending", "待审批"),
    ("approved", "已通过"),
    ("rejected", "已驳回"),
];

/// 流程模板节点：图内编号（meta.nodes[].id 与 rro.code 双向定位键）
struct FlowNode {
    graph_id: &'static str,
    /// 节点类型（publish 白名单子集）：approve → oper-approve 主体，其余 → oper-gate
    kind: &'static str,
    label: &'static str,
}

/// 流程模板：code → (notice, 线性节点链 start→approve→end)
const FLOW_TEMPLATES: &[(&str, &str, &[FlowNode])] = &[
    (
        REGISTRATION_FLOW_CODE,
        "用户注册审批",
        &[
            FlowNode {
                graph_id: "N1",
                kind: "start",
                label: "注册发起",
            },
            FlowNode {
                graph_id: "N2",
                kind: "approve",
                label: "访问授权审批",
            },
            FlowNode {
                graph_id: "N3",
                kind: "end",
                label: "授权完成",
            },
        ],
    ),
    (
        VERIFY_FLOW_CODE,
        "用户实名审核",
        &[
            FlowNode {
                graph_id: "N1",
                kind: "start",
                label: "实名提交",
            },
            FlowNode {
                graph_id: "N2",
                kind: "approve",
                label: "实名审核",
            },
            FlowNode {
                graph_id: "N3",
                kind: "end",
                label: "审核完成",
            },
        ],
    ),
    (
        AUTHORIZATION_FLOW_CODE,
        "访问授权审批",
        &[
            FlowNode {
                graph_id: "N1",
                kind: "start",
                label: "授权申请",
            },
            FlowNode {
                graph_id: "N2",
                kind: "approve",
                label: "访问授权审批",
            },
            FlowNode {
                graph_id: "N3",
                kind: "end",
                label: "授权完成",
            },
        ],
    ),
    (
        VERCTRL_FLOW_CODE,
        "版控固化审批",
        &[
            FlowNode {
                graph_id: "N1",
                kind: "start",
                label: "固化发起",
            },
            FlowNode {
                graph_id: "N2",
                kind: "approve",
                label: "版控固化审批",
            },
            FlowNode {
                graph_id: "N3",
                kind: "end",
                label: "固化完成",
            },
        ],
    ),
    (
        EXTERNAL_SUBJECT_FLOW_CODE,
        "外部主体入驻审批",
        &[
            FlowNode {
                graph_id: "N1",
                kind: "start",
                label: "入驻申请",
            },
            FlowNode {
                graph_id: "N2",
                kind: "approve",
                label: "外部主体入驻审核",
            },
            FlowNode {
                graph_id: "N3",
                kind: "end",
                label: "入驻完成",
            },
        ],
    ),
];

/// 审批域自检入口：基态 + 模板 + 一致性。
pub async fn ensure(pool: &PgPool) -> SeedStats {
    let mut stats = SeedStats::default();

    let (existing, created) = ensure_approval_status_seeds(pool).await;
    stats.existing += existing;
    stats.created += created;

    // 四个「实现·范例」流程模板（注册/实名审核/访问授权/版控固化，
    // add-approval-leaf-template-seeds + add-ct-git-vc-interop）
    for tpl in FLOW_TEMPLATES {
        let (existing, created, healed) = ensure_flow_template(pool, tpl).await;
        stats.existing += existing;
        stats.created += created;
        stats.healed += healed;
    }

    // SLA 时长维度（注册审批超时自动驳回的前提，add-register-approval-closure）
    match ensure_sla_duration(pool).await {
        Ok(preexisted) => {
            if preexisted {
                stats.existing += 1;
            } else {
                stats.created += 1;
            }
        }
        Err(e) => common::telemetry::warn!("seed[approval]: SLA 时长维度自愈失败: {e}"),
    }

    // 审批事件一致性自检（注册/外部入驻/实名审核，add-approval-leaf-template-seeds）：
    // 内部注册与访问授权共用 code "user-register-approval"（register.rs 双写契约）；
    // 外部入驻审批（add-dual-register-channels）为独立 code，
    // 映射独立流程 FLOW-EXTERNAL-SUBJECT，故三 code 各自遍历。
    for code in [
        REGISTRATION_APPROVAL_CODE,
        EXTERNAL_SUBJECT_APPROVAL_CODE,
        USER_VERIFY_CODE,
    ] {
        let (existing, broken, backfilled, rebound) = self_check_approvals(pool, code).await;
        stats.existing += existing as usize;
        stats.healed += backfilled as usize + rebound as usize;
        if broken > 0 {
            common::telemetry::warn!(
                "seed[approval]: {} 个 {code} 审批实例断链（rr_event 桥缺失/悬空）——请人工核查",
                broken
            );
        }
    }

    stats
}

/// 1. 审批基态种子：缺则 INSERT（幂等）。
async fn ensure_approval_status_seeds(pool: &PgPool) -> (usize, usize) {
    let mut existing = 0usize;
    let mut created = 0usize;
    for (code, notice) in APPROVAL_STATUS_SEEDS {
        let found: Option<i64> = match sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_stus-approve"
               WHERE code = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(code)
        .fetch_optional(pool)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                common::telemetry::warn!("seed[approval]: 状态种子查询失败 code={code}: {e}");
                None
            }
        };
        match found {
            Some(_) => existing += 1,
            None => match sqlx::query(
                // id 省略——DDL default gen_next_uid(73) 为唯一链码真相源
                r#"INSERT INTO isahl."zc_id_stus-approve" (code, notice)
                   VALUES ($1, $2)"#,
            )
            .bind(code)
            .bind(notice)
            .execute(pool)
            .await
            {
                Ok(_) => created += 1,
                Err(e) => {
                    common::telemetry::warn!("seed[approval]: 状态种子创建失败 code={code}: {e}");
                }
            },
        }
    }
    (existing, created)
}

/// SLA 时长维度幂等确保（zc_id_scal-duration 72h 行，注册审批超时自动驳回前提）。
/// 返回 (是否存在, 本轮之前已存在)。
async fn ensure_sla_duration(pool: &PgPool) -> Result<bool, sqlx::Error> {
    if let Some(_id) = sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM isahl."zc_id_scal-duration"
           WHERE o_number = $1 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(REGISTRATION_SLA_HOURS)
    .fetch_optional(pool)
    .await?
    {
        return Ok(true);
    }

    sqlx::query(
        r#"INSERT INTO isahl."zc_id_scal-duration" (o_number, mark, notice, code)
           VALUES ($1, $2, $3, $4)"#,
    )
    .bind(REGISTRATION_SLA_HOURS)
    .bind(REGISTRATION_SLA_HOURS_VALUE)
    .bind("注册审批超时时限（72 小时）")
    .bind("SLA-REG-72H")
    .execute(pool)
    .await?;

    Ok(false)
}

/// 流程模板编排：流程行 → 节点物化（事件载体 + 主体行 + 接线桥 + 归属桥）→ next-ops。
/// publish.rs materialize_graph 物化同构（线性图两阶段：先节点后边）。
///
/// 返回 (已存在, 新增, 修复)。修复 = 流程行本轮之前已存在但结构缺失的补建数。
async fn ensure_flow_template(
    pool: &PgPool,
    tpl: &(&str, &str, &[FlowNode]),
) -> (usize, usize, usize) {
    let (flow_code, flow_notice, nodes) = tpl;
    let mut existing = 0usize;
    let mut created = 0usize;
    let mut healed = 0usize;

    // 2.1 流程行（幂等，lifecycle 链 → DDL default gen_next_zuid；meta.nodes 承载
    // start 定位契约——initiate_flow 解析 type='start' 节点的图内编号）
    let (flow_id, flow_preexisted) =
        match ensure_process_row(pool, flow_code, flow_notice, nodes).await {
            Ok(v) => v,
            Err(e) => {
                common::telemetry::warn!("seed[approval]: 流程模板行自愈失败 {flow_code}: {e}");
                return (existing, created, healed);
            }
        };
    if flow_preexisted {
        existing += 1;
    } else {
        created += 1;
    }

    // 2.2 节点物化（幂等键 = 归属桥 (ref_left, code=图内编号)；publish 物化同构）
    let mut op_ids: Vec<i64> = Vec::with_capacity(nodes.len());
    for node in nodes.iter() {
        match ensure_flow_node(pool, flow_id, node).await {
            Ok((op_id, preexisted)) => {
                if preexisted {
                    existing += 1;
                } else if flow_preexisted {
                    healed += 1;
                } else {
                    created += 1;
                }
                op_ids.push(op_id);
            }
            Err(e) => {
                common::telemetry::warn!(
                    "seed[approval]: 流程节点自愈失败 {flow_code}/{}: {e}",
                    node.graph_id
                );
                return (existing, created, healed);
            }
        }
    }

    // 2.3 next-ops 出边（对象形态 {"id":N}——parse_next_op_entries 契约；
    // 无条件重写，顺带修复历史断边/裸数值旧形态）
    for (idx, node) in nodes.iter().enumerate() {
        let next_ops: serde_json::Value = match op_ids.get(idx + 1) {
            Some(downstream) => serde_json::json!([{ "id": downstream }]), // id-json-ok（DB 内 JSONB：Rust as_i64 + jsonb @> containment 双契约要求数字形态）
            None => serde_json::json!([]),
        };
        if let Err(e) = sqlx::query(
            r#"UPDATE isahl.zc_id_process_rr_operation
               SET "next-ops" = $1
               WHERE ref_left = $2 AND ref_right = $3 AND deleted_at IS NULL"#,
        )
        .bind(&next_ops)
        .bind(flow_id)
        .bind(op_ids[idx])
        .execute(pool)
        .await
        {
            common::telemetry::warn!(
                "seed[approval]: next-ops 写入失败 {flow_code}/{}: {e}",
                node.graph_id
            );
            return (existing, created, healed);
        }
    }

    (existing, created, healed)
}

/// 流程行幂等确保（meta.nodes 图结构随行写入）。返回 (流程 id, 本轮之前已存在)。
async fn ensure_process_row(
    pool: &PgPool,
    flow_code: &str,
    flow_notice: &str,
    nodes: &[FlowNode],
) -> Result<(i64, bool), sqlx::Error> {
    if let Some(id) = sqlx::query_scalar(
        "SELECT id FROM isahl.zc_id_process WHERE code = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(flow_code)
    .fetch_optional(pool)
    .await?
    {
        return Ok((id, true));
    }

    // 图内编号 → 节点类型（initiate_flow start 定位契约：meta.nodes[].id + type）
    let meta = serde_json::json!({
        "nodes": nodes
            .iter()
            .map(|n| serde_json::json!({ "id": n.graph_id, "type": n.kind }))
            .collect::<Vec<_>>(),
    });
    let id: i64 = match sqlx::query_scalar::<_, i64>(
        // 审批流程定义落 zc_id_proc-approve 子类（flow-process-continuity 规约：
        // 基表 zc_id_process 读经继承并集兼容；子类缺失降级基表，防旧库自愈失败）
        r#"INSERT INTO isahl."zc_id_proc-approve" (code, notice, meta)
           VALUES ($1, $2, $3)
           RETURNING id"#,
    )
    .bind(flow_code)
    .bind(flow_notice)
    .bind(&meta)
    .fetch_one(pool)
    .await
    {
        Ok(id) => id,
        Err(e) if e.to_string().contains("42P01") => {
            sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO isahl.zc_id_process (code, notice, meta)
                   VALUES ($1, $2, $3)
                   RETURNING id"#,
            )
            .bind(flow_code)
            .bind(flow_notice)
            .bind(&meta)
            .fetch_one(pool)
            .await?
        }
        Err(e) => return Err(e),
    };

    Ok((id, false))
}

/// 单节点物化（publish.rs 物化同构，单事务原子）：
/// 事件载体行（`zc_id_even-approve`，code=图内编号）→ 节点主体行
/// （approve→`zc_id_oper-approve`，start/end→`zc_id_oper-gate`，范例标记
/// `_f_='实现'`/`_t_='范例'`）→ cate 绑定（approve）→ end outcome meta →
/// `zc_id_operation_rr_event` 接线桥 → `zc_id_process_rr_operation` 归属桥
/// （code=图内编号、comments=节点 label；next-ops 由编排阶段二统一补写）。
///
/// 幂等键：归属桥 (ref_left, code)。返回 (节点主体行 id, 本轮之前已存在)。
async fn ensure_flow_node(
    pool: &PgPool,
    flow_id: i64,
    node: &FlowNode,
) -> Result<(i64, bool), sqlx::Error> {
    let existing_op: Option<i64> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl.zc_id_process_rr_operation
           WHERE ref_left = $1 AND code = $2 AND deleted_at IS NULL LIMIT 1"#,
    )
    .bind(flow_id)
    .bind(node.graph_id)
    .fetch_optional(pool)
    .await?;
    if let Some(op_id) = existing_op {
        return Ok((op_id, true));
    }

    let mut tx = pool.begin().await?;

    // 事件载体行（节点↔载体 1:1；实例挂载体、advance 经接线桥反查节点）
    let carrier_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_even-approve" (notice, created_by_id, code)
           VALUES ($1, 1, $2)
           RETURNING id"#,
    )
    .bind(node.label)
    .bind(node.graph_id)
    .fetch_one(&mut *tx)
    .await?;

    // 节点主体行（类型→子类分派同 publish；id 省略走 DDL default 生命周期链）
    let op_id: i64 = if node.kind == "approve" {
        sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_oper-approve" (notice, code, created_by_id, _f_, _t_)
               VALUES ($1, $2, 1, '实现', '范例')
               RETURNING id"#,
        )
        .bind(node.label)
        .bind(node.graph_id)
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_oper-gate" (notice, code, created_by_id, _f_, _t_)
               VALUES ($1, $2, 1, '实现', '范例')
               RETURNING id"#,
        )
        .bind(node.label)
        .bind(node.graph_id)
        .fetch_one(&mut *tx)
        .await?
    };

    // cate 绑定（非 terminal 节点；find-or-create 同 publish——类型判定 cate 优先）
    if node.kind == "approve" {
        let cate_id: i64 =
            match sqlx::query_scalar(
                r#"SELECT id FROM isahl."zc_id_cate-proc_op"
               WHERE code = 'approve' AND deleted_at IS NULL LIMIT 1"#,
            )
            .fetch_optional(&mut *tx)
            .await?
            .flatten()
            {
                Some(id) => id,
                None => sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_cate-proc_op" (notice, code, enable, created_by_id)
                       VALUES ('approve', 'approve', TRUE, 1)
                       RETURNING id"#,
                )
                .fetch_one(&mut *tx)
                .await?,
            };
        sqlx::query(r#"UPDATE isahl.zc_id_operation SET "ck_cate-proc_op" = $1 WHERE id = $2"#)
            .bind(cate_id)
            .bind(op_id)
            .execute(&mut *tx)
            .await?;
    }

    // end 节点终局语义（publish 同构：meta.end_outcome，缺省 complete）
    if node.kind == "end" {
        sqlx::query(
            r#"UPDATE isahl.zc_id_operation
               SET meta = jsonb_set(COALESCE(meta, '{}'::jsonb), '{end_outcome}',
                     to_jsonb('complete'::text), true)
               WHERE id = $1"#,
        )
        .bind(op_id)
        .execute(&mut *tx)
        .await?;
    }

    // 接线桥（节点主体 ↔ 事件载体；实例挂载体行——create_approval_instances 契约）
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_operation_rr_event (ref_left, ref_right, created_by_id)
           VALUES ($1, $2, 1)"#,
    )
    .bind(op_id)
    .bind(carrier_id)
    .execute(&mut *tx)
    .await?;

    // 归属桥（code=图内编号、comments=节点 label——COALESCE(rro.comments, o.notice)
    // 节点 label 解析契约；next-ops 编排阶段二补写）
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_process_rr_operation
           (code, ref_left, ref_right, comments, created_by_id)
           VALUES ($1, $2, $3, $4, 1)"#,
    )
    .bind(node.graph_id)
    .bind(flow_id)
    .bind(op_id)
    .bind(node.label)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((op_id, false))
}

/// 3. 审批事件一致性自检 + 模板绑定回填（按事件 code 分组，add-approval-leaf-template-seeds 泛化）。
///
/// - oper-approve 侧：桥断链计数 → 返回 broken；断链实例按 oper 字段重建 even
///   事件并回填 rr_event 桥行（oper→even 自愈，fix-approval-event-adaptive-write）
/// - even-approve 侧：缺 oper-approve 实例 → 补建（backfilled）
/// - 模板绑定回填：event 有 code 但 tpl_id IS NULL 且模板存在 → 回填（rebound；
///   fk_process 已物理移除——事件↔流程归属经节点主体行桥链反查，无事件侧归属列）
///
/// 事件 code → 流程模板 code 映射（决定模板绑定回填目标；与写入契约一致）：
/// - user-register-approval（注册/访问授权共用，approvals/apply、register.rs 均绑）→ FLOW-AUTHORIZATION
/// - user-verify → FLOW-USER-VERIFY
///
/// 返回 (实例总数, 断链数, 补建数, 回填数)
async fn self_check_approvals(pool: &PgPool, event_code: &str) -> (i64, i64, i64, i64) {
    // 事件 code → 流程模板 code（回填目标；与写入契约一致：
    // - user-verify → FLOW-USER-VERIFY
    // - user-register-approval（注册/访问授权共用）→ FLOW-AUTHORIZATION（approvals/apply、
    //   register.rs 均绑此流程，fix-approval-event-adaptive-write 统一）
    let flow_code: &str = match event_code {
        USER_VERIFY_CODE => VERIFY_FLOW_CODE,
        EXTERNAL_SUBJECT_APPROVAL_CODE => EXTERNAL_SUBJECT_FLOW_CODE,
        _ => AUTHORIZATION_FLOW_CODE,
    };

    let instance_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve"
           WHERE code = $1 AND deleted_at IS NULL"#,
    )
    .bind(event_code)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let broken_count: i64 = sqlx::query_scalar(
        // fix-fk-approve-residual-consumers：断链判定改经 operation_rr_event 桥
        //（实例无指向活跃 even-approve 事件的桥行 = 断链）
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           WHERE oa.code = $1 AND oa.deleted_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                 JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
                 WHERE rr.ref_left = oa.id AND rr.deleted_at IS NULL
             )"#,
    )
    .bind(event_code)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    // oper→even 自愈（fix-approval-event-adaptive-write）：断链 oper 实例
    // （rr_event 桥缺失）按 oper 字段重建缺失 even 事件并回填桥行。
    // 事件写入目标表自适应——`zc_id_appr-authorization` 叶表存在写叶表（继承
    // even-approve，查询可见），否则写 even-approve 主表（与 approvals/apply、
    // register.rs 注册主链路同规则）。
    // to_regclass 检测失败（schema/权限错误）不静默当无叶表，也不中断整个 self_check——
    // 置 leaf_check_ok=false 跳过 oper→even 自愈（无法确定目标表），但尾部 even→oper
    // 补建/rebound/SLA/broken_after 照常执行（fix-approval-event-adaptive-write 契约）。
    let mut leaf_check_ok = true;
    let leaf_table_exists: bool = match sqlx::query_scalar(
        "SELECT to_regclass('\"isahl.zc_id_appr-authorization\"') IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            common::telemetry::warn!(
                "seed[approval]: 检测 authorization 叶表失败（跳过 oper→even 自愈，保持断链告警）: {e}"
            );
            leaf_check_ok = false;
            false
        }
    };
    let mut healed_to_event: i64 = 0;
    // 仅注册审批（user-register-approval）执行 oper→even 自愈：其事件契约绑
    // FLOW-AUTHORIZATION + 72h SLA（approvals/apply、register.rs 一致）。
    // user-verify（USER_VERIFY_CODE）走独立流程模板 FLOW-USER-VERIFY，无对应
    // 自愈重建逻辑——若误按 AUTHORIZATION_FLOW_CODE 重建会绑错流程，故不在此
    // 自愈，断链维持告警由人工核查（fix-approval-event-adaptive-write 契约）。
    // leaf_check_ok=false 时同样跳过（无法确定事件写入目标表）。
    if broken_count > 0 && event_code == REGISTRATION_APPROVAL_CODE && leaf_check_ok {
        // 模板绑定目标：user-register-approval 事件绑 FLOW-AUTHORIZATION 的 approve 节点
        // 模板（与 approvals/apply、register.rs 写入契约一致）
        let flow_binding: Option<(i64, Option<i64>)> = sqlx::query_as(
            r#"
            SELECT p.id,
                   (SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
                    JOIN isahl."zc_id_oper-approve" oa
                      ON oa.id = rro.ref_right AND oa.deleted_at IS NULL
                    WHERE rro.ref_left = p.id AND rro.deleted_at IS NULL LIMIT 1)
            FROM isahl.zc_id_process p
            WHERE p.code = $1 AND p.deleted_at IS NULL
            LIMIT 1
            "#,
        )
        .bind(AUTHORIZATION_FLOW_CODE)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let sla_duration_id: Option<i64> = sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_scal-duration"
               WHERE o_number = $1 AND deleted_at IS NULL LIMIT 1"#,
        )
        .bind(REGISTRATION_SLA_HOURS)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        // 断链 oper 实例清单——仅主体仍存在且活跃（isahl_auth.auth_users, is_active=TRUE）
        // 的 oper 进入自愈：主体已删除或封禁/停用（is_active=false）时无法确定有效
        // applicant，重建会污染事件且不可追溯（封禁用户授权链路已终止），保持断链告警
        // 由人工核查（fix-approval-event-adaptive-write 契约）。
        let broken_ops: Vec<(i64, String, Option<i64>, String)> = sqlx::query_as(
            r#"SELECT oa.id, oa.notice, oa.fk_subject, oa.code
               FROM isahl."zc_id_oper-approve" oa
               WHERE oa.code = $1 AND oa.deleted_at IS NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                     JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
                     WHERE rr.ref_left = oa.id AND rr.deleted_at IS NULL
                 )
                 AND oa.fk_subject IS NOT NULL
                 AND EXISTS (SELECT 1 FROM isahl_auth.auth_users u
                             WHERE u.id = oa.fk_subject AND u.is_active = TRUE)
               ORDER BY oa.id"#,
        )
        .bind(event_code)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        // 断链但主体非活跃的 oper（已删除/is_active=false，无法自愈）→ 计数告警由
        // broken_after 覆盖（人工核查）
        let broken_no_subject: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
               WHERE oa.code = $1 AND oa.deleted_at IS NULL
                 AND NOT EXISTS (
                     SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                     JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
                     WHERE rr.ref_left = oa.id AND rr.deleted_at IS NULL
                 )
                 AND (oa.fk_subject IS NULL OR NOT EXISTS (
                     SELECT 1 FROM isahl_auth.auth_users u
                     WHERE u.id = oa.fk_subject AND u.is_active = TRUE
                 ))"#,
        )
        .bind(event_code)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
        if broken_no_subject > 0 {
            common::telemetry::warn!(
                "seed[approval]: {event_code} {} 个断链实例主体已删除/停用（无法自愈，保持告警人工核查）",
                broken_no_subject
            );
        }
        for (oper_id, notice, applicant_id, code) in &broken_ops {
            // 防御：broken_ops 查询已保证 fk_subject 非空且主体存在，此处仅防御性跳过
            // （写 0 会污染事件 created_by/comments，主体缺失的断链由 broken_after 告警）
            let Some(applicant_id) = applicant_id else {
                common::telemetry::warn!(
                    "seed[approval]: {event_code} oper→even 跳过重建 oper={oper_id}（fk_subject 为空，无法确定 applicant）"
                );
                continue;
            };
            // applicant_name 从 notice 提取（"用户 <name> 访问授权审批"）
            let applicant_name = notice
                .strip_prefix("用户 ")
                .and_then(|s| s.strip_suffix(" 访问授权审批"))
                .unwrap_or("用户")
                .to_string();
            // comments 为纯文本语义（remove-comments-json-embedding）：人类可读申请人摘要
            let comments = format!("申请人：{applicant_name}（用户 id {applicant_id}）");

            // 事务包裹：事件 INSERT + rr_event 桥行回填原子（崩溃不产生孤儿事件/重复事件）
            let mut tx = match pool.begin().await {
                Ok(t) => t,
                Err(e) => {
                    common::telemetry::warn!(
                        "seed[approval]: {event_code} oper→even 事务开启失败 oper={oper_id}: {e}"
                    );
                    continue;
                }
            };
            let event_id: Result<i64, _> = if leaf_table_exists {
                sqlx::query_scalar(
                    r#"
                    INSERT INTO isahl."zc_id_appr-authorization" (
                        created_by_id, updated_by_id, notice, code, comments,
                        tpl_id, qk_sla, created_at, updated_at
                    ) VALUES ($1, $1, $2, $3, $4, $5, $6, NOW(), NOW())
                    RETURNING id
                    "#,
                )
                .bind(applicant_id)
                .bind(notice)
                .bind(code)
                .bind(&comments)
                .bind(flow_binding.as_ref().and_then(|(_, t)| *t))
                .bind(sla_duration_id)
                .fetch_one(&mut *tx)
                .await
            } else {
                sqlx::query_scalar(
                    r#"
                    INSERT INTO isahl."zc_id_even-approve" (
                        created_by_id, updated_by_id, notice, code, comments,
                        tpl_id, qk_sla, created_at, updated_at
                    ) VALUES ($1, $1, $2, $3, $4, $5, $6, NOW(), NOW())
                    RETURNING id
                    "#,
                )
                .bind(applicant_id)
                .bind(notice)
                .bind(code)
                .bind(&comments)
                .bind(flow_binding.as_ref().and_then(|(_, t)| *t))
                .bind(sla_duration_id)
                .fetch_one(&mut *tx)
                .await
            };

            match event_id {
                Ok(new_event_id) => {
                    // fk_process 列已物理移除（2026-08-30）：事件↔流程归属经桥链——
                    // 'register-context' 上下文 oper 行（每流程复用）+
                    // process_rr_operation 归属桥 + rr_event 模板桥
                    if let Some((flow_id, _)) = flow_binding {
                        let ctx_oper: Option<i64> = sqlx::query_scalar(
                            r#"SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
                               JOIN isahl."zc_id_oper-approve" oa ON oa.id = rro.ref_right
                                 AND oa.deleted_at IS NULL AND oa.notice = 'register-context'
                               WHERE rro.ref_left = $1 AND rro.deleted_at IS NULL LIMIT 1"#,
                        )
                        .bind(flow_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .unwrap_or(None);
                        let ctx_oper: i64 = match ctx_oper {
                            Some(v) => v,
                            None => {
                                match sqlx::query_scalar::<_, i64>(
                                    r#"INSERT INTO isahl."zc_id_oper-approve" (notice, created_by_id)
                                       VALUES ('register-context', 1) RETURNING id"#,
                                )
                                .fetch_one(&mut *tx)
                                .await
                                {
                                    Ok(new_id) => {
                                        if let Err(e) = sqlx::query(
                                            "INSERT INTO isahl.zc_id_process_rr_operation (ref_left, ref_right, created_by_id)
                                             VALUES ($1, $2, 1)",
                                        )
                                        .bind(flow_id)
                                        .bind(new_id)
                                        .execute(&mut *tx)
                                        .await
                                        {
                                            let _ = tx.rollback().await;
                                            common::telemetry::warn!(
                                                "seed[approval]: {event_code} 流程归属桥失败 oper={oper_id}: {e}"
                                            );
                                            continue;
                                        }
                                        new_id
                                    }
                                    Err(e) => {
                                        let _ = tx.rollback().await;
                                        common::telemetry::warn!(
                                            "seed[approval]: {event_code} register-context 创建失败 oper={oper_id}: {e}"
                                        );
                                        continue;
                                    }
                                }
                            }
                        };
                        if let Err(e) = sqlx::query(
                            "INSERT INTO isahl.zc_id_operation_rr_event (ref_left, ref_right, created_by_id)
                             VALUES ($1, $2, 1)",
                        )
                        .bind(ctx_oper)
                        .bind(new_event_id)
                        .execute(&mut *tx)
                        .await
                        {
                            let _ = tx.rollback().await;
                            common::telemetry::warn!(
                                "seed[approval]: {event_code} 事件模板桥失败 oper={oper_id}: {e}"
                            );
                            continue;
                        }
                    }

                    // 桥行回填（同事务）。校验 rows_affected==1：事件已插入但
                    // 回填 0 行（oper 并发删除/软删/桥已存在）→ 事务回滚，不计 healed，
                    // 避免孤儿事件 + 虚报自愈数。
                    match sqlx::query(
                        r#"INSERT INTO isahl.zc_id_operation_rr_event
                           (ref_left, ref_right, created_by_id)
                           SELECT $2, $1, 1
                           WHERE EXISTS (
                               SELECT 1 FROM isahl."zc_id_oper-approve" oa
                               WHERE oa.id = $2 AND oa.deleted_at IS NULL
                           )
                           AND NOT EXISTS (
                               SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                               WHERE rr.ref_left = $2 AND rr.ref_right = $1 AND rr.deleted_at IS NULL
                           )"#,
                    )
                    .bind(new_event_id)
                    .bind(oper_id)
                    .execute(&mut *tx)
                    .await
                    {
                        Ok(rows) if rows.rows_affected() == 1 => match tx.commit().await {
                            Ok(_) => healed_to_event += 1,
                            // commit 失败：事务已尝试提交并结束（tx 被消费），无法 rollback
                            Err(e) => common::telemetry::warn!(
                                "seed[approval]: {event_code} oper→even 事务提交失败 oper={oper_id}: {e}"
                            ),
                        },
                        Ok(rows) => {
                            // 回填 0 行：oper 已不存在/软删/桥已存在 → 回滚（不产生孤儿事件）
                            let _ = tx.rollback().await;
                            common::telemetry::warn!(
                                "seed[approval]: {event_code} oper→even 回填影响 {} 行（oper 可能已删或桥已存在），回滚 oper={oper_id}",
                                rows.rows_affected()
                            );
                        }
                        Err(e) => {
                            let _ = tx.rollback().await;
                            common::telemetry::warn!(
                                "seed[approval]: {event_code} oper→even 回填失败 oper={oper_id}: {e}"
                            );
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.rollback().await;
                    common::telemetry::warn!(
                        "seed[approval]: {event_code} oper→even 重建失败 oper={oper_id}: {e}"
                    );
                }
            }
        }
        if healed_to_event > 0 {
            common::telemetry::info!(
                "seed[approval]: 自愈 {} 个 {event_code} 断链实例（oper→even 重建并回填 rr_event 桥）",
                healed_to_event
            );
        }
    }

    // 补建：even-approve 有事件但无 oper-approve 实例 → 使其在审批工作区（dock）可见
    let admin_id = first_admin_id(pool).await;
    let backfilled: i64 = match sqlx::query(
        // fix-fk-approve-residual-consumers：fk_approve 列已移除——
        // 实例补建不带事件绑定，随后由 bridge_pairing 语句落 rr_event 桥行
        r#"
        INSERT INTO isahl."zc_id_oper-approve" (
            notice, code, fk_subject, fk_operator, created_by_id, created_at, updated_at
        )
        SELECT
            COALESCE(e.notice, '审批'), e.code,
            e.created_by_id,
            $2,
            e.created_by_id,
            NOW(), NOW()
        FROM isahl."zc_id_even-approve" e
        WHERE e.code = $1 AND e.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM isahl.zc_id_operation_rr_event rr
              JOIN isahl."zc_id_oper-approve" oa ON oa.id = rr.ref_left AND oa.deleted_at IS NULL
              WHERE rr.ref_right = e.id AND rr.deleted_at IS NULL
          )
        "#,
    )
    .bind(event_code)
    .bind(admin_id)
    .execute(pool)
    .await
    {
        Ok(r) => r.rows_affected() as i64,
        Err(e) => {
            common::telemetry::warn!("seed[approval]: {event_code} 审批实例补建失败: {e}");
            0
        }
    };
    if backfilled > 0 {
        common::telemetry::info!(
            "seed[approval]: 补建 {} 个 {event_code} 审批实例（even-approve → oper-approve，dock 可见）",
            backfilled
        );
    }

    // 桥配对：补建/历史实例（无任何 rr_event 桥）↔ 同 code 无桥事件，按创建序 1:1 配对
    //（幂等：已有桥行的事件/实例跳过；rows_affected 仅作观测不计入 stats）
    let _bridged: i64 = match sqlx::query(
        r#"
        INSERT INTO isahl.zc_id_operation_rr_event (ref_left, ref_right, created_by_id)
        SELECT pair.op_id, e.id, 1
        FROM isahl."zc_id_even-approve" e
        JOIN LATERAL (
            SELECT oa.id AS op_id
            FROM isahl."zc_id_oper-approve" oa
            WHERE oa.code = e.code AND oa.deleted_at IS NULL
              AND NOT EXISTS (
                  SELECT 1 FROM isahl.zc_id_operation_rr_event x
                  WHERE x.ref_left = oa.id AND x.deleted_at IS NULL
              )
            ORDER BY oa.created_at, oa.id
            LIMIT 1
        ) pair ON TRUE
        WHERE e.code = $1 AND e.deleted_at IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM isahl.zc_id_operation_rr_event rr
              WHERE rr.ref_right = e.id AND rr.deleted_at IS NULL
          )
        "#,
    )
    .bind(event_code)
    .execute(pool)
    .await
    {
        Ok(r) => r.rows_affected() as i64,
        Err(e) => {
            common::telemetry::warn!("seed[approval]: {event_code} 实例↔事件桥配对失败: {e}");
            0
        }
    };

    // 模板绑定回填：历史/模板缺失期的事件未绑节点模板 → 回填 tpl_id（approve
    // 节点主体行，与 approvals/apply 写入契约一致）。fk_process 已物理移除——
    // 事件↔流程归属经「节点主体行 ∈ process_rr_operation」桥链反查
    // （advance_flow 步骤 1），无事件侧归属列可回填。
    let rebound: i64 = match sqlx::query(
        r#"
        UPDATE isahl."zc_id_even-approve" e
        SET tpl_id = (SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
                      JOIN isahl."zc_id_oper-approve" oa
                        ON oa.id = rro.ref_right AND oa.deleted_at IS NULL
                      WHERE rro.ref_left = p.id AND rro.deleted_at IS NULL LIMIT 1)
        FROM isahl.zc_id_process p
        WHERE e.code = $1 AND e.deleted_at IS NULL
          AND e.tpl_id IS NULL
          AND p.code = $2 AND p.deleted_at IS NULL
        "#,
    )
    .bind(event_code)
    .bind(flow_code)
    .execute(pool)
    .await
    {
        Ok(r) => r.rows_affected() as i64,
        Err(e) => {
            common::telemetry::warn!("seed[approval]: {event_code} 事件模板绑定回填失败: {e}");
            0
        }
    };
    if rebound > 0 {
        common::telemetry::info!(
            "seed[approval]: 回填 {} 个 {event_code} 事件模板绑定（tpl_id → approve 节点主体行）",
            rebound
        );
    }

    // SLA 回填：审批事件未设 qk_sla 且 72h 时长维度存在 → 回填
    // （add-register-approval-closure：纳入超时自动驳回的前提）
    let sla_backfilled: i64 = match sqlx::query(
        r#"
        UPDATE isahl."zc_id_even-approve" e
        SET qk_sla = sd.id
        FROM isahl."zc_id_scal-duration" sd
        WHERE e.code = $1 AND e.deleted_at IS NULL
          AND e.qk_sla IS NULL
          AND sd.o_number = $2 AND sd.deleted_at IS NULL
        "#,
    )
    .bind(event_code)
    .bind(REGISTRATION_SLA_HOURS)
    .execute(pool)
    .await
    {
        Ok(r) => r.rows_affected() as i64,
        Err(e) => {
            common::telemetry::warn!("seed[approval]: {event_code} 事件 SLA 回填失败: {e}");
            0
        }
    };
    if sla_backfilled > 0 {
        common::telemetry::info!(
            "seed[approval]: 回填 {} 个 {event_code} 事件 SLA（qk_sla → 72h）",
            sla_backfilled
        );
    }

    // 自愈后重算断链（oper→even 自愈可能已落 rr_event 桥行）——返回自愈后真实断链数，
    // 使告警反映剩余未修复断链（fix-approval-event-adaptive-write：自愈成功不误报）。
    let broken_after: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_oper-approve" oa
           WHERE oa.code = $1 AND oa.deleted_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                 JOIN isahl."zc_id_even-approve" e ON e.id = rr.ref_right AND e.deleted_at IS NULL
                 WHERE rr.ref_left = oa.id AND rr.deleted_at IS NULL
             )"#,
    )
    .bind(event_code)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    (
        instance_count,
        broken_after,
        backfilled,
        rebound + sla_backfilled + healed_to_event,
    )
}

/// 首个 admin 用户（复用 register.rs 的解析 SQL）。
async fn first_admin_id(pool: &PgPool) -> Option<i64> {
    sqlx::query_scalar(
        r#"
        SELECT ur.fk_user FROM isahl_auth.ngac_user_rr_attribute ur
        JOIN isahl_auth.ngac_user_attribute ua ON ua.id = ur.fk_user_attribute
        WHERE ua.o_name = 'admin' AND ur.deleted_at IS NULL AND ua.deleted_at IS NULL
          AND (ur.expires_at IS NULL OR ur.expires_at > NOW())
        ORDER BY ur.id LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}
