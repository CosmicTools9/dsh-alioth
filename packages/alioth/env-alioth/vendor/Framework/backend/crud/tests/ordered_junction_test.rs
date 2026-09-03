use crud::{
    entity::AliothDbEntity,
    reference::{
        build_refs_select_suffix, Card, HasReferenceJoins, JoinKind, JunctionField, ReferenceJoin,
    },
    Identifiable,
};

#[derive(sqlx::FromRow, serde::Serialize)]
struct TestOrderedToOne {
    id: i64,
}
impl AliothDbEntity for TestOrderedToOne {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_contacts""#
    }
    const SELECT_FIELDS: &'static str = "e.id";
    const ENTITY_NAME: &'static str = "test1";
    const SOFT_DELETE: bool = false;
}
impl Identifiable for TestOrderedToOne {
    fn id(&self) -> i64 {
        self.id
    }
}
impl HasReferenceJoins for TestOrderedToOne {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "email",
            card: Card::ToOne,
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
        }]
    }
}

#[test]
fn ordered_to_one_with_sort_and_junction_fields() {
    let suffix = build_refs_select_suffix::<TestOrderedToOne>();
    assert!(!suffix.is_empty());
    assert!(suffix.contains("default_info"));
    assert!(suffix.contains("is_default"));
    assert!(suffix.contains("DESC"));
    assert!(suffix.contains("NULLS LAST"));
    assert!(suffix.contains("LIMIT 1"));
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct TestOrderedToMany {
    id: i64,
}
impl AliothDbEntity for TestOrderedToMany {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_contacts""#
    }
    const SELECT_FIELDS: &'static str = "e.id";
    const ENTITY_NAME: &'static str = "test2";
    const SOFT_DELETE: bool = false;
}
impl Identifiable for TestOrderedToMany {
    fn id(&self) -> i64 {
        self.id
    }
}
impl HasReferenceJoins for TestOrderedToMany {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "emails",
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
        }]
    }
}

#[test]
fn ordered_to_many_includes_coalesce_and_junction_fields() {
    let suffix = build_refs_select_suffix::<TestOrderedToMany>();
    assert!(!suffix.is_empty());
    assert!(
        suffix.contains("COALESCE"),
        "ToMany should wrap with COALESCE(..., '[]'::jsonb)"
    );
    assert!(suffix.contains("jsonb_agg"), "ToMany should use jsonb_agg");
    assert!(
        suffix.contains("is_default"),
        "should include junction field alias"
    );
    assert!(
        suffix.contains("ORDER BY"),
        "should have ORDER BY for aggregated sort"
    );
    assert!(!suffix.contains("LIMIT 1"), "ToMany must NOT have LIMIT 1");
    assert!(
        suffix.contains("'[]'::jsonb"),
        "should have empty array fallback"
    );
}

#[derive(sqlx::FromRow, serde::Serialize)]
struct TestOrderedToManyNoSort {
    id: i64,
}
impl AliothDbEntity for TestOrderedToManyNoSort {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_contacts""#
    }
    const SELECT_FIELDS: &'static str = "e.id";
    const ENTITY_NAME: &'static str = "test3";
    const SOFT_DELETE: bool = false;
}
impl Identifiable for TestOrderedToManyNoSort {
    fn id(&self) -> i64 {
        self.id
    }
}
impl HasReferenceJoins for TestOrderedToManyNoSort {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![ReferenceJoin {
            name: "emails",
            card: Card::ToMany,
            kind: JoinKind::OrderedJunction {
                junction_table: r#"isahl."zc_id_contacts_rr_infos""#,
                source_fk: "ref_left",
                target_fk: "ref_right",
                order_by: None,
                order_desc: false,
                nulls_last: false,
                junction_display_fields: &[],
            },
            target_table: r#"isahl."zc_id_info-email""#,
            display_fields: &["notice"],
        }]
    }
}

#[test]
fn ordered_to_many_without_sort_still_has_coalesce() {
    let suffix = build_refs_select_suffix::<TestOrderedToManyNoSort>();
    assert!(!suffix.is_empty());
    assert!(suffix.contains("COALESCE"), "always COALESCE for ToMany");
    assert!(suffix.contains("'[]'::jsonb"), "empty array fallback");
    assert!(!suffix.contains("ORDER BY"), "no ORDER BY without order_by");
    assert!(!suffix.contains("LIMIT 1"), "no LIMIT 1 for ToMany");
}
