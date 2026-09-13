//! 流程 ↔ 任务双向绑定（change add-avic-generic-task-execution tasks 5.1/5.2）
//!
//! 引擎侧物化 + 订阅（orchestration 侧发布端见该服务 `handlers/task.rs`
//! transition done 分支；装配见 Gateway main.rs 领域事件订阅区）。
//!
//! ## 流程 → 任务（物化）
//! 人工节点（approve/review/action/vote——任务驱动语义 = 等效审批通过，须有
//! 待办 oper-approve 实例承载链路/门控）的载体（even-approve `timeline`
//! jsonb，经 operation_rr_event 桥）可配 `taskTemplate`：
//! ```json
//! { "name": "设计任务", "taskType": "design", "assigneeId": 12345 }
//! ```
//! 引擎推进物化到该节点（advance.rs 四处人工物化分支）→
//! `maybe_create_node_task`：INSERT `zc_id_task-<taskType>` 叶表行——
//! notice=name、fk_place=实体上下文行 id（若有）、fk_subject=assigneeId，
//! timeline jsonb 落绑定 `{"flowExecution","flowNode","entityTable",
//! "entityId"}`（zuid 数值一律字符串化防 2^53 截断），并写 TASK-PENDING
//! 主状态桥（字典缺行 warn 跳过，不阻断流程）。
//! - 无 taskTemplate 的节点零变化（配置读取一次查询，缺失即返回）。
//! - 每「执行链根 × 节点」至多一张生成任务（同执行重物化幂等跳过——
//!   如驳回打回目标节点再次创建实例时不重复建任务）。
//! - 执行链根 = 实例 fk_previous 链回溯至 zc_id_proc-* 执行行
//!   （实现·实例，flow-lifecycle-split；无锚定执行（如裸 initiate_flow
//!   测试直调）→ 无法绑定，warn 跳过建任务）。
//!
//! ## 任务 → 流程（订阅）
//! orchestration 任务 transition done 成功分支发布 `TaskCompleted`
//! （payload：task_id + timeline 中的 flowExecution/flowNode；无绑定不发）。
//! 本模块订阅：按 flowExecution + flowNode（= 节点 operation 行 id，与
//! 实例 tpl_id 同指）找该执行链上的待办 oper-approve 实例（fk_previous
//! 链回溯确认执行归属；解析失败 warn 跳过），逐实例写 approved 生命周期
//! 桥后以「等效审批通过」推进（复用 advance_flow 原语，不新造状态机）——
//! sequential 增量创建场景循环推进直至节点清空。终态实例幂等放行
//! （重复事件/重复点击 → 无待办即空操作）。
//!
//! 注：任务行创建不发 EntityCreated（5.3 仅覆盖 orchestration CRUD 创建
//! 路径，已在 create_task 发布）——引擎物化路径若再发会引入任务域范畴
//! 流程自递归风险，任务可见性由任务中心按 dk 域直读承载。

use common::error::AliothError as ApiError;
use common::event_bus::{DomainEvent, DomainEventBus};
use common::SYSTEM_USER_ID;
use sqlx::PgPool;
use std::sync::Arc;

/// TaskCompleted 频道常量（与 orchestration handlers/task.rs 发布端对齐）
pub mod event_types {
    pub const TASK_COMPLETED: &str = "TaskCompleted";
    pub const SOURCE_ORCHESTRATION: &str = "orchestration";
}

/// taskType → 任务叶表名（静态白名单，对齐 orchestration models_task
/// TASK_TYPES；未知类型不入 SQL）
fn leaf_table(task_type: &str) -> Option<&'static str> {
    match task_type {
        "design" => Some(r#"isahl."zc_id_task-design""#),
        "develop" => Some(r#"isahl."zc_id_task-develop""#),
        "testing" => Some(r#"isahl."zc_id_task-testing""#),
        "commission" => Some(r#"isahl."zc_id_task-commission""#),
        "fix" => Some(r#"isahl."zc_id_task-fix""#),
        "storage" => Some(r#"isahl."zc_id_task-storage""#),
        _ => None,
    }
}

/// 任务模板配置（载体 even-approve.timeline.taskTemplate 反序列化）。
/// 字段宽容：assigneeId 兼容数字与字符串（zuid 字符串化防 2^53 截断）。
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskTemplateConfig {
    pub name: Option<String>,
    pub task_type: Option<String>,
    pub assignee_id: Option<i64>,
}

/// 从载体 timeline 解析数值字段（数字或字符串均兼容；非数值 → None）
fn json_i64(v: Option<&serde_json::Value>) -> Option<i64> {
    v.and_then(|x| {
        x.as_i64()
            .or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))
    })
}

/// 读取节点任务模板配置（缺失 → None；载体缺失（无 rr_event 桥节点）同义）
async fn read_task_template(
    pool: &PgPool,
    node_op_id: i64,
) -> Result<Option<TaskTemplateConfig>, ApiError> {
    let raw: Option<serde_json::Value> = sqlx::query_scalar(
        r#"SELECT ea.timeline->'taskTemplate' FROM isahl."zc_id_even-approve" ea
           JOIN isahl.zc_id_operation_rr_event oe ON oe.ref_right = ea.id AND oe.deleted_at IS NULL
           WHERE oe.ref_left = $1 AND ea.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(node_op_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    let Some(serde_json::Value::Object(obj)) = raw else {
        return Ok(None);
    };
    let cfg = TaskTemplateConfig {
        name: obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        task_type: obj
            .get("taskType")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        assignee_id: json_i64(obj.get("assigneeId")),
    };
    if cfg.name.is_none() && cfg.task_type.is_none() && cfg.assignee_id.is_none() {
        // 空壳 taskTemplate（{}）视同未配置，零变化
        return Ok(None);
    }
    Ok(Some(cfg))
}

/// fk_previous 链回溯（防环 depth≤100）：自实例向上走审批实例链，首个不在
/// oper-approve 的行即执行链根（zc_id_proc-* 实现·实例；无锚定 → None）
async fn execution_root(pool: &PgPool, instance_id: i64) -> Result<Option<i64>, ApiError> {
    let root: Option<i64> = sqlx::query_scalar(
        r#"WITH RECURSIVE walk(id, depth, path) AS (
             SELECT $1::bigint, 0, ARRAY[$1::bigint]
             UNION ALL
             SELECT oa.fk_previous, w.depth + 1, w.path || oa.fk_previous
             FROM isahl."zc_id_oper-approve" oa
             JOIN walk w ON oa.id = w.id
             WHERE oa.fk_previous IS NOT NULL
               AND w.depth < 100
               AND NOT (oa.fk_previous = ANY(w.path))
           )
           SELECT w.id FROM walk w
           WHERE NOT EXISTS (SELECT 1 FROM isahl."zc_id_oper-approve" oa WHERE oa.id = w.id)
           ORDER BY w.depth DESC LIMIT 1"#,
    )
    .bind(instance_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    Ok(root)
}

/// 实例是否属于指定执行链（fk_previous 链上溯命中执行行 id）
async fn instance_belongs_to_execution(
    pool: &PgPool,
    instance_id: i64,
    execution_id: i64,
) -> Result<bool, ApiError> {
    let hit: bool = sqlx::query_scalar(
        r#"WITH RECURSIVE walk(id, depth, path) AS (
             SELECT $1::bigint, 0, ARRAY[$1::bigint]
             UNION ALL
             SELECT oa.fk_previous, w.depth + 1, w.path || oa.fk_previous
             FROM isahl."zc_id_oper-approve" oa
             JOIN walk w ON oa.id = w.id
             WHERE oa.fk_previous IS NOT NULL
               AND w.depth < 100
               AND NOT (oa.fk_previous = ANY(w.path))
           )
           SELECT EXISTS (SELECT 1 FROM walk WHERE id = $2)"#,
    )
    .bind(instance_id)
    .bind(execution_id)
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    Ok(hit)
}

/// 物化接线：人工节点实例创建成功后调用——节点载体配了 taskTemplate 则
/// 生成任务行（绑定执行链根/节点/实体上下文）。advance.rs 四处人工物化
/// 分支（initiate_flow 首跳 / advance_flow / advance_fan_out /
/// process_node_advancement）各自加一行调用；自动节点分支不调用
/// （自动节点无待办等待语义，taskTemplate 对其无意义——设计器侧约束）。
pub(crate) async fn maybe_create_node_task(
    pool: &PgPool,
    node_op_id: i64,
    node_label: &str,
    entity: Option<&(String, i64)>,
    instance_ids: &[i64],
) -> Result<(), ApiError> {
    let Some(&first_instance) = instance_ids.first() else {
        return Ok(());
    };
    let Some(cfg) = read_task_template(pool, node_op_id).await? else {
        return Ok(());
    };

    // 1. 配置校验（缺失/非法 → warn 跳过，不阻断流程——节点回落纯人工审批）
    let Some(task_type) = cfg.task_type else {
        common::telemetry::warn!(
            "taskTemplate: node {} 缺 taskType——跳过任务生成（节点回落人工审批）",
            node_op_id
        );
        return Ok(());
    };
    let Some(leaf) = leaf_table(&task_type) else {
        common::telemetry::warn!(
            "taskTemplate: node {} taskType '{}' 不在白名单（design|develop|testing|commission|fix|storage）——跳过任务生成",
            node_op_id,
            task_type
        );
        return Ok(());
    };
    let name = cfg.name.unwrap_or_else(|| node_label.to_string());

    // 2. 执行链根（无锚定执行 → 无法绑定，warn 跳过）
    let Some(execution_id) = execution_root(pool, first_instance).await? else {
        common::telemetry::warn!(
            "taskTemplate: node {} 实例 {} 无执行链根（fk_previous 未锚定执行行）——跳过任务生成",
            node_op_id,
            first_instance
        );
        return Ok(());
    };

    // 3. 幂等：同执行同节点已生成任务（驳回打回重物化等）→ 跳过
    let binding_dup = serde_json::json!({
        "flowExecution": execution_id.to_string(),
        "flowNode": node_op_id.to_string(),
    });
    let dup: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM isahl.zc_id_task
           WHERE timeline @> $1::jsonb AND deleted_at IS NULL)"#,
    )
    .bind(binding_dup.to_string())
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?;
    if dup {
        common::telemetry::info!(
            "taskTemplate: node {} (exec {}) 已生成任务——重物化跳过",
            node_op_id,
            execution_id
        );
        return Ok(());
    }

    // 4. dk 坐标静态绑定（研发执行同域 JE/FMA/↓_CH，与 orchestration
    //    DkEntity::DkYaFmaCh 同源；维度缺行 → NULL，读侧 NULL 过滤可容）
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("JE", "FMA", "↓_CH"))
        .await
        .map_err(|e| ApiError::Database(format!("task dk resolve failed: {}", e)))?;

    // 5. 建行（落叶表；notice=name；fk_place=实体上下文行；fk_subject=
    //    assigneeId；timeline=绑定——zuid 一律字符串，防 2^53 截断）
    let timeline = serde_json::json!({
        "flowExecution": execution_id.to_string(),
        "flowNode": node_op_id.to_string(),
        "entityTable": entity.map(|(t, _)| t),
        "entityId": entity.map(|(_, id)| id.to_string()),
    });
    let insert_sql = format!(
        "INSERT INTO {leaf} (id, notice, code, comments, fk_place, fk_subject, timeline, \
         created_by_id, dk_scene, dk_factor, dk_function) \
         VALUES (isahl.gen_next_zuid(), $1, NULL, $2, $3, $4, $5, $6, $7, $8, $9) \
         RETURNING id"
    );
    // 静态白名单表名（match 常量）+ 参数化值；AssertSqlSafe 声明已审计
    let comments =
        format!("流程执行 {execution_id} 节点 {node_op_id} 物化生成（节点「{node_label}」）");
    let task_id: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(insert_sql.as_str()))
        .bind(&name)
        .bind(&comments)
        .bind(entity.map(|(_, id)| *id))
        .bind(cfg.assignee_id)
        .bind(timeline)
        .bind(SYSTEM_USER_ID)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(pool)
        .await
        .map_err(|e| ApiError::Database(format!("task insert into {} failed: {}", leaf, e)))?;

    // 6. 初始主状态桥 TASK-PENDING（与 orchestration create 对齐；字典缺行
    //    属环境问题——warn 跳过不阻断，任务中心无桥行按初始态展示）
    let pending_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_stus-task" WHERE code = 'TASK-PENDING' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))?
    .flatten();
    match pending_id {
        Some(pid) => {
            sqlx::query(
                r#"INSERT INTO isahl."zc_id_lifecycle_r_primary-status" (id, ref_left, ref_right, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
            )
            .bind(task_id)
            .bind(pid)
            .bind(SYSTEM_USER_ID)
            .execute(pool)
            .await
            .map_err(|e| ApiError::Database(e.to_string()))?;
        }
        None => {
            common::telemetry::warn!(
                "taskTemplate: 状态字典缺 code=TASK-PENDING（zc_id_stus-task 未播种）——任务 {} 未写状态桥",
                task_id
            );
        }
    }

    common::telemetry::info!(
        "taskTemplate: node {} (exec {}) → 任务行 {}（{}/{}/{}）",
        node_op_id,
        execution_id,
        task_id,
        leaf,
        name,
        entity
            .map(|(t, id)| format!("{}#{}", t, id))
            .unwrap_or_else(|| "无实体上下文".to_string())
    );
    Ok(())
}

/// 节点待办实例（oper-approve，tpl_id = 节点 operation 行；未终态）
async fn pending_instances_at_node(pool: &PgPool, node_op_id: i64) -> Result<Vec<i64>, ApiError> {
    sqlx::query_scalar(
        r#"SELECT oa.id FROM isahl."zc_id_oper-approve" oa
           WHERE oa.tpl_id = $1 AND oa.deleted_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl."zc_id_lifecycle_r_primary-status" ls
                 JOIN isahl."zc_id_stus-approve" s ON s.id = ls.ref_right
                 WHERE ls.ref_left = oa.id AND ls.deleted_at IS NULL
                   AND s.code IN ('approved','rejected','withdrawn','cancelled','abstained')
             )
           ORDER BY oa.id"#,
    )
    .bind(node_op_id)
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::Database(e.to_string()))
}

/// TaskCompleted 事件负载（orchestration 发布端契约：task_id + timeline
/// 绑定 flowExecution/flowNode，zuid 字符串化）
#[derive(Debug, serde::Deserialize)]
pub struct TaskCompletedPayload {
    pub task_id: String,
    pub flow_execution: String,
    pub flow_node: String,
}

/// 订阅 TaskCompleted（Gateway 装配；与 cc_notify/auto_initiate 同构）：
/// spawn 后台循环消费，单事件失败仅记录不终止订阅。
pub fn subscribe_task_completed(bus: Arc<dyn DomainEventBus>, pool: PgPool) {
    actix_web::rt::spawn(async move {
        let mut subscriber = match bus.subscribe(event_types::TASK_COMPLETED).await {
            Ok(s) => s,
            Err(e) => {
                common::telemetry::error!("task-completed: subscribe TaskCompleted failed: {}", e);
                return;
            }
        };
        loop {
            match subscriber.recv().await {
                Ok(evt) => {
                    if let Err(e) = handle_task_completed(&pool, Some(&bus), evt).await {
                        common::telemetry::error!("task-completed: handle failed: {}", e);
                    }
                }
                Err(_) => continue,
            }
        }
    });
}

/// 任务完成处理：绑定节点待办实例 → 等效审批通过 → 推进流程
///
/// - 事件非 TaskCompleted / payload 缺绑定字段 / 执行链上无待办实例 →
///   空操作返回（幂等；重复事件/无绑定任务不触发）。
/// - 推进复用 advance_flow（含节点签署模式门控）；sequential 增量创建
///   场景循环「审批 → 推进」直至节点无待办（上限 25 防意外死循环）。
/// - 终态后发布 ApprovalCompleted（等效人工审批通过，订阅者联动不变）。
pub async fn handle_task_completed(
    pool: &PgPool,
    bus: Option<&Arc<dyn DomainEventBus>>,
    event: DomainEvent,
) -> Result<(), ApiError> {
    if event.event_type != event_types::TASK_COMPLETED {
        return Ok(());
    }
    let Ok(payload) = serde_json::from_value::<TaskCompletedPayload>(event.payload.clone()) else {
        common::telemetry::warn!(
            "TaskCompleted payload 解析失败（缺 task_id/flow_execution/flow_node 绑定字段？）: {:?}——跳过",
            event.payload
        );
        return Ok(());
    };
    let (Ok(execution_id), Ok(node_op_id)) = (
        payload.flow_execution.trim().parse::<i64>(),
        payload.flow_node.trim().parse::<i64>(),
    ) else {
        common::telemetry::warn!(
            "TaskCompleted 绑定字段非法（exec='{}' node='{}'）——跳过",
            payload.flow_execution,
            payload.flow_node
        );
        return Ok(());
    };

    let mut advanced_last: Option<i64> = None;
    for _ in 0..25 {
        // 1. 节点待办候选 → 执行链归属过滤（同模板多执行并发互不误配）
        let pending = pending_instances_at_node(pool, node_op_id).await?;
        let mut targets = Vec::new();
        for pid in &pending {
            if instance_belongs_to_execution(pool, *pid, execution_id).await? {
                targets.push(*pid);
            }
        }
        if targets.is_empty() {
            // 无待办：未物化节点/已推进完成/其他执行——空操作
            if advanced_last.is_none() {
                common::telemetry::info!(
                    "TaskCompleted: exec {} node {} 无待办实例——跳过（无绑定或已推进）",
                    execution_id,
                    node_op_id
                );
            }
            break;
        }
        // 2. 等效审批通过（全部在途实例置 approved——任务完成 = 节点整体通过）
        for pid in &targets {
            crate::handlers::approve_reject::update_lifecycle_status(
                pool,
                *pid,
                "approved",
                "任务完成：等效审批通过",
                SYSTEM_USER_ID,
            )
            .await?;
        }
        let last = *targets.last().expect("targets 非空");
        // 3. 推进（复用审批通过原语；sequential 未走完 → 返回时节点仍有新待办，
        //    下一轮循环继续，直至扇出到下游节点或节点清空）
        crate::advance::advance_flow(pool, last, SYSTEM_USER_ID, bus).await?;
        advanced_last = Some(last);
        common::telemetry::info!(
            "TaskCompleted: exec {} node {} 待办实例 {:?} 等效审批通过并推进",
            execution_id,
            node_op_id,
            targets
        );
    }

    // 4. 审批完成事件（等效人工通过；与 approve_inner 对齐，bus 缺失跳过）
    if let (Some(bus), Some(last)) = (bus, advanced_last) {
        crate::handlers::approve_reject::publish_approval_completed(
            bus,
            pool,
            last,
            "approved",
            Some("任务完成：等效审批通过"),
        )
        .await;
    }
    Ok(())
}
