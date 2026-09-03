//! 状态模型（共享内核）——`isahl.zc_id_status`
//!
//! 字段并集吸收 WZ/Alioth 两 ns 契约：
//!
//! - WZ：notice/code/flag/enable（+ _refs 恒 None）
//! - Alioth：name（notice AS name 别名）/flag/comments
//!
//! SELECT_FIELDS 用 notice 原列（壳投影 name 别名时自行处理）。
//! 另有 isahl 全局事件实体：DamageReport（zc_id_appr-damage）/EventTracking（zc_id_even-tracking）/
//! EventAccident（zc_id_even-accident）——只读仓库，写操作由各 domain service 负责。

use chrono::{DateTime, Utc};
use crud::entity::{AliothDbEntity, Identifiable};
use crud::reference::{Card, HasReferenceJoins, JoinKind, ReferenceJoin};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

/// 状态实体 — 映射 `isahl.zc_id_status`
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Status {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub code: Option<String>,
    pub flag: Option<String>,
    pub enable: Option<bool>,
    pub comments: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Status {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Status {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_status\""
    }
    const SELECT_FIELDS: &'static str =
        "id, notice, code, flag::text, enable, comments, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "status";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

/// 创建请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateStatusRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub flag: Option<String>,
    pub enable: Option<bool>,
    pub comments: Option<String>,
}

/// 更新请求（PATCH 语义）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatusRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub flag: Option<String>,
    pub enable: Option<bool>,
    pub comments: Option<String>,
}

// ═══ DamageReport — isahl.zc_id_appr-damage ═════

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DamageReport {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_urgent: Option<i64>,
    pub timeline: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
}
impl Identifiable for DamageReport {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for DamageReport {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_appr-damage\""
    }
    const SELECT_FIELDS: &'static str = "id, notice, code, comments, qk_date, fk_place, fk_subject, lk_urgent, timeline, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "damage_report";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}
impl HasReferenceJoins for DamageReport {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_subject",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_subject",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_place",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_place",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_place""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "qk_date",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_date",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-date""#,
                display_fields: &["notice", "date"],
            },
            ReferenceJoin {
                name: "lk_urgent",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "lk_urgent",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_level""#,
                display_fields: &["notice"],
            },
        ]
    }
}

// ═══ EventTracking — isahl.zc_id_even-tracking ═══

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EventTracking {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    pub timeline: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
    #[sqlx(default)]
    pub _refs: Option<Value>,
}
impl Identifiable for EventTracking {
    fn id(&self) -> i64 {
        self.id
    }
}
impl AliothDbEntity for EventTracking {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_even-tracking\""
    }
    const SELECT_FIELDS: &'static str = "id, notice, code, comments, qk_date, fk_place, fk_subject, timeline, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "event_tracking";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}
impl HasReferenceJoins for EventTracking {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "fk_subject",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_subject",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "fk_place",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_place",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_place""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "qk_date",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "qk_date",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_scal-date""#,
                display_fields: &["notice", "date"],
            },
        ]
    }
}

// ── Request DTOs ─────────────────────────────────

// ═══ EventAccident — isahl.zc_id_even-accident ═══

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EventAccident {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub code: Option<String>,
    pub notice: Option<String>,
    pub comments: Option<String>,
    pub t_color_: Option<String>,
    /// 关联的事故主体
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    /// 关联的地点
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    /// 事故日期（unix ts）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    /// 事故时间线 (JSON)
    pub timeline: Option<Value>,
    /// 风险等级 (lk)
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_risk: Option<i64>,
    /// 严重程度 (lk)
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_severity: Option<i64>,
}

impl Identifiable for EventAccident {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for EventAccident {
    fn table_name() -> &'static str {
        "\"isahl\".\"zc_id_even-accident\""
    }
    const SELECT_FIELDS: &'static str = "id, code, notice, comments, t_color_, fk_subject, fk_place, qk_date, timeline, lk_risk, lk_severity, created_at, updated_at";
    const ENTITY_NAME: &'static str = "event_accident";
    const SOFT_DELETE: bool = true;
}

impl HasReferenceJoins for EventAccident {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "subject",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_subject",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_subjects""#,
                display_fields: &["notice", "code"],
            },
            ReferenceJoin {
                name: "place",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_place",
                    target_key: "id",
                },
                target_table: r#"isahl."zc_id_place""#,
                display_fields: &["notice", "code"],
            },
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDamageRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_urgent: Option<i64>,
    pub timeline: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDamageRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_urgent: Option<i64>,
    pub timeline: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEventRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    pub timeline: Option<Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateEventRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    pub timeline: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAccidentRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    pub timeline: Option<Value>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_risk: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_severity: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAccidentRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_subject: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_place: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub qk_date: Option<i64>,
    pub timeline: Option<Value>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_risk: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub lk_severity: Option<i64>,
}
