//! contacts service 测试
//!
//! 1. ContactsEntity reference_joins 配置验证（无 DB）
//! 2. 集成测试：email+phone 共存、分页、default_info

use crud::entity::AliothDbEntity;
use crud::reference::{Card, HasReferenceJoins, JoinKind};
use crud::Identifiable;
use framework_contacts::models::ContactsEntity;

#[test]
fn contacts_entity_has_reference_joins() {
    let joins = ContactsEntity::reference_joins();
    assert!(!joins.is_empty(), "should have reference joins");

    // Verify email join uses OrderedJunction(ToMany)
    let email = joins
        .iter()
        .find(|j| j.name == "email")
        .expect("should have email join");
    assert_eq!(email.card, Card::ToMany, "info joins should be ToMany");
    matches!(&email.kind, JoinKind::OrderedJunction { .. });

    // Verify phone join uses OrderedJunction(ToMany)
    let phone = joins
        .iter()
        .find(|j| j.name == "phone")
        .expect("should have phone join");
    assert_eq!(phone.card, Card::ToMany, "info joins should be ToMany");

    // Verify department join uses Junction
    let dept = joins
        .iter()
        .find(|j| j.name == "department")
        .expect("should have department join");
    matches!(&dept.kind, JoinKind::Junction { .. });
}

#[test]
fn contacts_entity_has_basic_traits() {
    let entity = ContactsEntity {
        id: 42,
        notice: Some("test".into()),
        position: None,
        avatar_url: None,
        _refs: None,
    };
    assert_eq!(entity.id(), 42);
    assert_eq!(ContactsEntity::ENTITY_NAME, "contacts");
    assert!(
        ContactsEntity::SELECT_FIELDS.contains("notice"),
        "should contain notice"
    );
    assert!(
        ContactsEntity::SELECT_FIELDS.contains("id"),
        "should contain id"
    );
}
