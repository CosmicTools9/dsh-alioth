use chrono::{DateTime, Utc};
use crud::entity::{AliothDbEntity, Identifiable};
use crud::reference::{Card, HasReferenceJoins, JoinKind, ReferenceJoin};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ── ApprovalFlow ──────────────────────────────────────────────
// 表: isahl."zc_id_proc-approve"（zc_id_process 审批族子类；create 落子类表，
// 读路径经基表 zc_id_process 继承并集——与工艺路线共享流程系统）

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ApprovalFlow {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    pub code: Option<String>,
    pub t_color_: Option<String>,
    /// 人类可读备注（comments-text-semantics：不承载结构）
    pub comments: Option<String>,
    /// 设计图 JSON 信封（serializeFlow 契约：version/nodes/meta；
    /// 存储于 meta jsonb 列——migrate-flow-design-storage-to-meta-mermaid）
    #[sqlx(default)]
    pub meta: Option<serde_json::Value>,
    /// 流程整体结构 mermaid 文本（保存时引擎自动生成，幂等）
    pub mermaid: Option<String>,
    /// 流程输入范畴（fk_context → zc_id_proc-context 族 scope-definition 行）
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_context: Option<i64>,
    /// 实际落位叶表（tableoid 派生，如 zc_id_proc-approve）
    pub branch: Option<String>,
    /// 输入范畴业务概念（proc-context 读聚合解析 notice）
    pub context_concept: Option<String>,
    /// 输入范畴落位叶表（fk_context 行 tableoid 派生——发起端点 entity_table 数据源）
    pub context_leaf: Option<String>,
    /// 生命周期主状态（zc_id_lifecycle_r_primary-status 桥 + zc_id_stus-process 字典派生；
    /// create/update RETURNING 不派生——新流程/更新无状态变化，回退 None）
    #[sqlx(default)]
    pub status: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for ApprovalFlow {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for ApprovalFlow {
    fn table_name() -> &'static str {
        "isahl.zc_id_process"
    }
    const SELECT_FIELDS: &'static str =
        "id, notice AS name, code, t_color_, comments, meta, mermaid, fk_context, \
         e.tableoid::regclass::text AS branch, \
         (SELECT c.notice FROM isahl.\"zc_id_proc-context\" c \
          WHERE c.id = e.fk_context AND c.deleted_at IS NULL) AS context_concept, \
         (SELECT replace(c.tableoid::regclass::text, '\"', '') FROM isahl.\"zc_id_proc-context\" c \
          WHERE c.id = e.fk_context AND c.deleted_at IS NULL) AS context_leaf, \
         (SELECT s.code FROM isahl.\"zc_id_lifecycle_r_primary-status\" ls \
          JOIN isahl.\"zc_id_stus-process\" s ON s.id = ls.ref_right \
          WHERE ls.ref_left = e.id AND ls.deleted_at IS NULL) AS status, \
         created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "approval-flow";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// ── FlowNode ──────────────────────────────────────────────────
// 保持不变（非本次校对范围）

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FlowNode {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub label: String,
    pub code: Option<String>,
    pub t_color_: Option<String>,
    pub comments: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for FlowNode {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for FlowNode {
    fn table_name() -> &'static str {
        "isahl.\"zc_id_even-approve\""
    }
    const SELECT_FIELDS: &'static str =
        "id, notice AS label, code, t_color_, comments, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "flow-node";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// ── ApprovalInstance ──────────────────────────────────────────
// 表: isahl.zc_id_oper-approve（发起审批事项）
// 实例↔审批事件经 zc_id_operation_rr_event 桥（ref_left=实例, ref_right=zc_id_even-approve 模板）

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ApprovalInstance {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub node_name: String, // notice = 当前节点名称
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_approve: Option<i64>, // → zc_id_even-approve
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_subject: Option<i64>, // 申请人
    pub comments: Option<String>, // 表单数据 JSON
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    /// 引用解析嵌入（事件 notice 经 rr_event 桥派生；fk_subject → auth_users 用户名）
    #[sqlx(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for ApprovalInstance {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for ApprovalInstance {
    fn table_name() -> &'static str {
        "isahl.\"zc_id_oper-approve\""
    }
    const SELECT_FIELDS: &'static str = "id, notice AS node_name, code, \
         (SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe \
          WHERE oe.ref_left = e.id AND oe.deleted_at IS NULL \
            AND EXISTS (SELECT 1 FROM isahl.zc_id_operation_rr_event oe2 \
                        JOIN isahl.zc_id_process_rr_operation rro2 \
                          ON rro2.ref_right = oe2.ref_left AND rro2.deleted_at IS NULL \
                        WHERE oe2.ref_right = oe.ref_right AND oe2.deleted_at IS NULL) \
          ORDER BY oe.created_at LIMIT 1) AS fk_approve, \
         fk_subject, comments, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "approval-instance";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// ── ApprovalAction ────────────────────────────────────────────
// 表: isahl.zc_id_deta-opinion（审批意见明细）

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ApprovalAction {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub summary: String,         // notice
    pub opinion: Option<String>, // comments
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_list: Option<i64>, // → zc_id_oper-approve 审批实例（fk_index 契约）
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_biller: Option<i64>, // 审批人（zc_id_deta-opinion 无 fk_subject 列，实际列为 fk_biller）
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_date: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    /// 引用解析嵌入（fk_biller → auth_users 用户名；fk_list → oper-approve notice；
    /// qk_date → zc_id_scal-date 日标量）
    #[sqlx(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for ApprovalAction {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for ApprovalAction {
    fn table_name() -> &'static str {
        "isahl.\"zc_id_deta-opinion\""
    }
    const SELECT_FIELDS: &'static str =
        "id, notice AS summary, opinion AS opinion, code, fk_list, fk_biller, qk_date, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "approval-action";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

// ── Request DTOs ──────────────────────────────────────────────

// ApprovalFlow requests
#[derive(Debug, Deserialize)]
pub struct ListApprovalFlowsQuery {
    #[serde(with = "common::serde_zuid::opt")]
    pub page: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub page_size: Option<i64>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateApprovalFlowRequest {
    pub name: String,
    pub code: Option<String>,
    /// 人类可读备注（comments-text-semantics：不承载结构）
    pub comments: Option<String>,
    /// 设计图 JSON 信封（serializeFlow 契约；写入 meta jsonb 列）
    pub meta: Option<serde_json::Value>,
    /// 流程叶表分支（zc_id_process 子表物理名；缺省 proc-approve 向后兼容）
    pub branch: Option<String>,
    /// 流程输入范畴（→ zc_id_proc-context 族 scope-definition 行 zuid；兼容旧契约）
    #[serde(default, with = "common::serde_zuid::opt")]
    pub context_id: Option<i64>,
    /// 流程输入上下文叶表（新契约：选定域叶表后在域父表创建流程专属
    /// 上下文范例行 `_t_='flow-context'`，fk_context → 范例行）
    pub context_table: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApprovalFlowRequest {
    pub name: Option<String>,
    // code 为引擎发布位/业务码（publish/unpublish 独占），客户端 update 禁写
    /// 人类可读备注（comments-text-semantics：不承载结构）
    pub comments: Option<String>,
    /// 设计图 JSON 信封（serializeFlow 契约；写入 meta jsonb 列）
    pub meta: Option<serde_json::Value>,
    /// 流程输入范畴重绑（分支不可改——行不可跨叶表迁移）
    #[serde(default, with = "common::serde_zuid::opt")]
    pub context_id: Option<i64>,
    /// 流程输入上下文叶表重绑（新契约：域父表建新范例行）
    pub context_table: Option<String>,
}

// FlowNode requests
#[derive(Debug, Deserialize)]
pub struct ListFlowNodesQuery {
    #[serde(with = "common::serde_zuid::opt")]
    pub page: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub page_size: Option<i64>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFlowNodeRequest {
    pub label: String,
    pub code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFlowNodeRequest {
    pub label: Option<String>,
    pub code: Option<String>,
}

// ApprovalInstance requests
#[derive(Debug, Deserialize)]
pub struct ListApprovalInstancesQuery {
    #[serde(with = "common::serde_zuid::opt")]
    pub page: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub page_size: Option<i64>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateApprovalInstanceRequest {
    pub node_name: String,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_approve: Option<i64>,
    pub comments: Option<String>, // 表单数据 JSON
}

#[derive(Debug, Deserialize)]
pub struct UpdateApprovalInstanceRequest {
    pub node_name: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>, // 表单数据 JSON（不可变）
}

// ApprovalAction requests
#[derive(Debug, Deserialize)]
pub struct ListApprovalActionsQuery {
    #[serde(with = "common::serde_zuid::opt")]
    pub page: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub page_size: Option<i64>,
    pub search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateApprovalActionRequest {
    pub summary: String,
    pub code: Option<String>,
    /// 关联审批实例（fk_index 契约：fk_list → zc_id_oper-approve.id）
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_list: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApprovalActionRequest {
    pub summary: Option<String>,
    pub code: Option<String>,
}

// ── DelegationRule ───────────────────────────────────────────
// 表: isahl.zc_id_operation（公式驱动委托规则）

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DelegationRule {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String, // notice
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_subject: Option<i64>, // 委托人（委托发起人）
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_operator: Option<i64>, // 受托人（被委托代审人）
    pub comments: Option<String>, // JSON 时间窗：{"validFrom","validUntil"}（RFC3339）
    /// 起止时间标量引用（zc_id_segm-date：date_st/date_ed/time_st/time_ed）
    #[serde(with = "common::serde_zuid::opt")]
    pub qk_period: Option<i64>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: Option<chrono::DateTime<Utc>>,
    pub deleted_at: Option<chrono::DateTime<Utc>>,
    /// 引用解析嵌入（qk_period → zc_id_segm-date 的 date_st/date_ed）
    #[sqlx(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for DelegationRule {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for DelegationRule {
    fn table_name() -> &'static str {
        "isahl.zc_id_operation"
    }
    const SELECT_FIELDS: &'static str =
        "id, notice AS name, code, fk_subject, fk_operator, comments, qk_period, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "delegation-rule";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
    /// 判别值过滤：zc_id_operation 为 operation 族共享根表（审批实例/计划执行等同族
    /// 子表行经继承均可见），list/get 必须限定 `_t_='delegation-rule'`——与 create
    /// 写入判别值、advance::apply_delegation 检索语义三方一致。
    const COORDINATE_FILTER: &'static str = "_t_ = 'delegation-rule'";
}

impl HasReferenceJoins for DelegationRule {
    /// 起止时间标量引用：qk_period → zc_id_segm-date（date_st/date_ed 为起止时间物理列）。
    /// 前端经 `_refs.qk_period.date_st/date_ed` 读取委托生效/失效时间。
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "qk_period",
            card: Card::ToOne,
            kind: JoinKind::Forward {
                local_fk: "qk_period",
                target_key: "id",
            },
            target_table: r#"isahl."zc_id_segm-date""#,
            display_fields: &["date_st", "date_ed"],
        }]
    }
}

impl HasReferenceJoins for ApprovalInstance {
    /// 审批实例引用：事件 notice（经 rr_event 桥派生）、fk_subject → auth_users（申请人）
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "event",
                card: Card::ToOne,
                kind: JoinKind::Junction {
                    junction_table: "isahl.zc_id_operation_rr_event",
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("created_at"),
                },
                target_table: r#"isahl."zc_id_even-approve""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "applicant",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_subject",
                    target_key: "id",
                },
                target_table: "isahl_auth.auth_users",
                display_fields: &["name", "username"],
            },
        ]
    }
}

impl HasReferenceJoins for ApprovalAction {
    /// 审批意见引用：fk_biller → auth_users（审批人）、fk_list → oper-approve（实例 notice）、
    /// qk_date → zc_id_scal-date（日标量）
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "biller",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_biller",
                    target_key: "id",
                },
                target_table: "isahl_auth.auth_users",
                display_fields: &["name", "username"],
            },
            ReferenceJoin {
                name: "instance",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_list",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_oper-approve""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "date",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_date",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-date""#,
                display_fields: &["date"],
            },
        ]
    }
}

// DelegationRule requests
#[derive(Debug, Deserialize)]
pub struct ListDelegationRulesQuery {
    #[serde(with = "common::serde_zuid::opt")]
    pub page: Option<i64>,
    #[serde(with = "common::serde_zuid::opt")]
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDelegationRuleRequest {
    pub name: String,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_subject: Option<i64>, // 委托人
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_operator: Option<i64>, // 受托人
    pub comments: Option<String>, // JSON 时间窗：{"validFrom","validUntil"}
    pub date_st: Option<chrono::DateTime<Utc>>, // 生效日期（RFC3339，nullable）
    pub date_ed: Option<chrono::DateTime<Utc>>, // 失效日期（RFC3339，nullable）
}

#[derive(Debug, Deserialize)]
pub struct UpdateDelegationRuleRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_subject: Option<i64>, // 委托人
    #[serde(with = "common::serde_zuid::opt")]
    pub fk_operator: Option<i64>, // 受托人
    pub comments: Option<String>, // JSON 时间窗：{"validFrom","validUntil"}
    pub date_st: Option<chrono::DateTime<Utc>>, // 生效日期（RFC3339，nullable）
    pub date_ed: Option<chrono::DateTime<Utc>>, // 失效日期（RFC3339，nullable）
}
