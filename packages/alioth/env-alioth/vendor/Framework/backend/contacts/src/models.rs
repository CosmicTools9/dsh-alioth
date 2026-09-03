//! 联系人公共类型

use serde::Serialize;
use serde_json::Value;

use crud::entity::AliothDbEntity;
use crud::reference::{Card, HasReferenceJoins, JoinKind, JunctionField, ReferenceJoin};
use crud::Identifiable;

/// 单条联系方式：类型 + 值 + 是否默认
#[derive(Debug, Clone, Serialize)]
pub struct ContactInfoValue {
    /// 类型标记（"email", "telephone", "im", "isahl", "postal", "zipcode"）
    pub kind: String,
    /// 实际联系值
    pub value: String,
    /// 是否默认联系方式
    pub is_default: bool,
}

/// 联系人信息（对齐 ContactsPanel.ContactInfo）
#[derive(Debug, Serialize)]
pub struct ContactInfo {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
    /// 便捷字段：默认邮箱（从 infos 中提取 kind=email 且 is_default 的首条）
    pub email: Option<String>,
    /// 便捷字段：默认电话
    pub phone: Option<String>,
    pub department: Option<String>,
    pub position: Option<String>,
    pub avatar_url: Option<String>,
    pub is_online: Option<bool>,
    /// 全部联系方式（含 6 种类型，支持重复记录 + default 标记）
    pub infos: Vec<ContactInfoValue>,
}

/// 单条联系方式输入
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ContactInfoInput {
    pub kind: String,
    pub value: String,
    #[serde(default)]
    pub is_default: bool,
}

/// 创建联系人请求
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateContactRequest {
    pub name: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub comments: Option<String>,
    #[serde(default)]
    pub infos: Vec<ContactInfoInput>,
}

/// 更新联系人请求
#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdateContactRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub comments: Option<String>,
    #[serde(default)]
    pub infos: Vec<ContactInfoInput>,
}

/// Contacts 实体，用于 `_refs` 关联解析
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ContactsEntity {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub notice: Option<String>,
    /// 岗位名称（通过 entity→empl-natural→post_rr_employee→subj-position 链读取）
    pub position: Option<String>,
    /// 头像 URL（dev 无文件头像链，恒为 NULL，保留字段以稳定 API 形状）
    #[sqlx(default)]
    pub avatar_url: Option<String>,
    #[sqlx(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _refs: Option<Value>,
}

impl Identifiable for ContactsEntity {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for ContactsEntity {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_contacts""#
    }
    const SELECT_FIELDS: &'static str = concat!(
        "e.id, e.notice,\n",
        "  (SELECT sp.notice FROM isahl.\"zc_id_entity_rr_contacts\" rr\n",
        "   JOIN isahl.\"zc_id_empl-natural\" en ON en.id = rr.ref_left AND en.deleted_at IS NULL\n",
        "   JOIN isahl.\"zc_id_subj-post_rr_employee\" spre ON spre.ref_right = en.id AND spre.deleted_at IS NULL\n",
        "   JOIN isahl.\"zc_id_subj-position\" sp ON sp.id = spre.ref_left AND sp.deleted_at IS NULL\n",
        "   WHERE rr.ref_right = e.id AND rr.deleted_at IS NULL LIMIT 1) AS position",
    );
    const ENTITY_NAME: &'static str = "contacts";
    const SOFT_DELETE: bool = true;
}

impl HasReferenceJoins for ContactsEntity {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            ReferenceJoin {
                name: "email",
                card: Card::ToMany,
                kind: JoinKind::OrderedJunction {
                    junction_table: r#"isahl."zc_id_contacts_rr_infos""#,
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("default_info"),
                    order_desc: true,
                    nulls_last: true,
                    junction_display_fields: &[JunctionField {
                        column: "default_info",
                        alias: "is_default",
                    }],
                },
                target_table: r#"isahl."zc_id_info-email""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "phone",
                card: Card::ToMany,
                kind: JoinKind::OrderedJunction {
                    junction_table: r#"isahl."zc_id_contacts_rr_infos""#,
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("default_info"),
                    order_desc: true,
                    nulls_last: true,
                    junction_display_fields: &[JunctionField {
                        column: "default_info",
                        alias: "is_default",
                    }],
                },
                target_table: r#"isahl."zc_id_info-telephone""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "im",
                card: Card::ToMany,
                kind: JoinKind::OrderedJunction {
                    junction_table: r#"isahl."zc_id_contacts_rr_infos""#,
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("default_info"),
                    order_desc: true,
                    nulls_last: true,
                    junction_display_fields: &[JunctionField {
                        column: "default_info",
                        alias: "is_default",
                    }],
                },
                target_table: r#"isahl."zc_id_info-im""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "department",
                card: Card::ToOne,
                kind: JoinKind::Junction {
                    junction_table: r#"isahl."zc_id_entity_rr_contacts""#,
                    source_fk: "ref_right",
                    target_fk: "ref_left",
                    order_by: None,
                },
                target_table: r#"isahl.zc_id_entity"#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "isahl",
                card: Card::ToMany,
                kind: JoinKind::OrderedJunction {
                    junction_table: r#"isahl."zc_id_contacts_rr_infos""#,
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("default_info"),
                    order_desc: true,
                    nulls_last: true,
                    junction_display_fields: &[JunctionField {
                        column: "default_info",
                        alias: "is_default",
                    }],
                },
                target_table: r#"isahl."zc_id_info-isahl""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "postal",
                card: Card::ToMany,
                kind: JoinKind::OrderedJunction {
                    junction_table: r#"isahl."zc_id_contacts_rr_infos""#,
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("default_info"),
                    order_desc: true,
                    nulls_last: true,
                    junction_display_fields: &[JunctionField {
                        column: "default_info",
                        alias: "is_default",
                    }],
                },
                target_table: r#"isahl."zc_id_info-postal""#,
                display_fields: &["notice"],
            },
            ReferenceJoin {
                name: "zipcode",
                card: Card::ToMany,
                kind: JoinKind::OrderedJunction {
                    junction_table: r#"isahl."zc_id_contacts_rr_infos""#,
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("default_info"),
                    order_desc: true,
                    nulls_last: true,
                    junction_display_fields: &[JunctionField {
                        column: "default_info",
                        alias: "is_default",
                    }],
                },
                target_table: r#"isahl."zc_id_info-zipcode""#,
                display_fields: &["notice"],
            },
        ]
    }
}
