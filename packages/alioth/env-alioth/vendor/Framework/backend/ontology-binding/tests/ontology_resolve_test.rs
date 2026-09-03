//! ontology-binding 测试：DkBinding trait + code→ZUID 解析
//!
//! 单元：trait 绑定契约（无 DB）
//! 集成：resolve/resolve_conn 在测试库解析真实维度行（`#[ignore]`，需 DATABASE_URL）

use common::testing::connect_test_db;
use ontology_binding::{resolve, resolve_conn, Coords, DkBinding};

/// 测试实体：WZ 运输转换（对齐 transport-dispatch DkEntity::TspTransition）
struct TspTransition;
impl DkBinding for TspTransition {
    fn coords(&self) -> Coords {
        ("GC", "FJA", "↓_BE")
    }
}

#[test]
fn dk_binding_trait_returns_fixed_coords() {
    let e = TspTransition;
    assert_eq!(e.coords(), ("GC", "FJA", "↓_BE"), "实体绑定固定三元组");
}

#[test]
fn coords_type_is_static_str_tuple() {
    let c: Coords = ("XX", "FJA", "↑_GG");
    assert_eq!(c.0, "XX");
    assert_eq!(c.2, "↑_GG");
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn resolve_finds_dimension_rows_by_code() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(_) => connect_test_db().await,
        None => {
            eprintln!("SKIP: 无 DATABASE_URL");
            return;
        }
    };
    let (scene, factor, function) = resolve(&pool, TspTransition.coords())
        .await
        .expect("resolve GC/FJA/↓_BE");
    assert!(scene.is_some(), "scene GC 应存在（WZ 种子）");
    assert!(factor.is_some(), "factor FJA 应存在");
    assert!(function.is_some(), "function ↓_BE 应存在");
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn resolve_conn_works_inside_transaction() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(_) => connect_test_db().await,
        None => {
            eprintln!("SKIP: 无 DATABASE_URL");
            return;
        }
    };
    let mut tx = pool.begin().await.expect("begin");
    let (scene, _, _) = resolve_conn(&mut tx, TspTransition.coords())
        .await
        .expect("resolve_conn");
    assert!(scene.is_some());
    tx.commit().await.expect("commit");
}

#[tokio::test]
#[ignore = "需 DATABASE_URL 测试库"]
async fn resolve_unknown_code_returns_none() {
    let url = std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty());
    let pool = match url {
        Some(_) => connect_test_db().await,
        None => {
            eprintln!("SKIP: 无 DATABASE_URL");
            return;
        }
    };
    let (scene, factor, function) =
        resolve(&pool, ("NO-SUCH-SCENE", "NO-SUCH-FACTOR", "NO-SUCH-FN"))
            .await
            .expect("resolve unknown");
    assert!(
        scene.is_none() && factor.is_none() && function.is_none(),
        "未知 code → None（列可空语义）"
    );
}
