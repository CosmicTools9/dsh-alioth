//! `JoinKind::Junction`（含 ToOne / ToMany）软删过滤契约测试
//!
//! 背景（实测缺陷）：桥接表 `zc_id_contacts_rr_infos` 在联系人保存时「软删旧行 + 插新行」，
//! 而聚合子查询原先不过滤 `deleted_at` → 同一联系方式在 `_refs` 中出现多份且随保存次数累积
//! （用户观感「修改变成新增」）。契约：桥接行与目标行都必须带 `deleted_at IS NULL`。

use crud::entity::AliothDbEntity;
use crud::reference::{build_refs_select_suffix, Card, HasReferenceJoins, JoinKind, ReferenceJoin};
use crud::Identifiable;

#[derive(sqlx::FromRow, serde::Serialize)]
struct JunctionToOne {
    id: i64,
}
impl AliothDbEntity for JunctionToOne {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_contacts""#
    }
    const SELECT_FIELDS: &'static str = "e.id";
    const ENTITY_NAME: &'static str = "junction_to_one_test";
    const SOFT_DELETE: bool = false;
}
impl Identifiable for JunctionToOne {
    fn id(&self) -> i64 {
        self.id
    }
}
impl HasReferenceJoins for JunctionToOne {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "department",
            card: Card::ToOne,
            kind: JoinKind::Junction {
                junction_table: r#"isahl."zc_id_entity_rr_contacts""#,
                source_fk: "ref_right",
                target_fk: "ref_left",
                order_by: None,
            },
            target_table: r#"isahl."zc_id_entity""#,
            display_fields: &["notice"],
        }]
    }
}

#[test]
fn junction_to_one_filters_soft_deleted_rows() {
    let suffix = build_refs_select_suffix::<JunctionToOne>();
    assert!(suffix.contains("LIMIT 1"), "ToOne junction must LIMIT 1");
    assert!(
        suffix.contains("jt0.\"deleted_at\" IS NULL"),
        "junction row must be filtered: {suffix}"
    );
    assert!(
        suffix.contains("t0.\"deleted_at\" IS NULL"),
        "target row must be filtered: {suffix}"
    );
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct JunctionToMany {
    id: i64,
}
impl AliothDbEntity for JunctionToMany {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_contacts""#
    }
    const SELECT_FIELDS: &'static str = "e.id";
    const ENTITY_NAME: &'static str = "junction_to_many_test";
    const SOFT_DELETE: bool = false;
}
impl Identifiable for JunctionToMany {
    fn id(&self) -> i64 {
        self.id
    }
}
impl HasReferenceJoins for JunctionToMany {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "tags",
            card: Card::ToMany,
            kind: JoinKind::Junction {
                junction_table: r#"isahl."zc_id_lifecycle_r_tags""#,
                source_fk: "ref_left",
                target_fk: "ref_right",
                order_by: None,
            },
            target_table: "isahl.zc_id_tags",
            display_fields: &["notice", "code"],
        }]
    }
}

#[test]
fn junction_to_many_filters_soft_deleted_rows() {
    let suffix = build_refs_select_suffix::<JunctionToMany>();
    assert!(
        suffix.contains("jsonb_agg"),
        "ToMany junction must aggregate"
    );
    assert!(
        !suffix.contains("LIMIT 1"),
        "ToMany junction must NOT limit rows"
    );
    assert!(
        suffix.contains("jt0.\"deleted_at\" IS NULL"),
        "junction row must be filtered: {suffix}"
    );
    assert!(
        suffix.contains("t0.\"deleted_at\" IS NULL"),
        "target row must be filtered: {suffix}"
    );
}
