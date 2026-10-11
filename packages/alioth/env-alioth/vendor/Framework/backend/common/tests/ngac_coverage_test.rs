//! `common::ngac_grants::ensure_org_resource_coverage`（B-2 组织资源覆盖单入口）语义测试。
//!
//! 覆盖（cover-org-resources-ngac tasks 2.1/2.2/3.1/3.2/5.1/5.2 判据）：
//! - 首跑三面落库：集合 OA（`{rt}-collection`、`fk_resource=0`、default 类）+
//!   ownership 默认行（`enabled=false`、owner→operator、benefit/permit/access→employee）+
//!   operator UA × 集合 OA 的 read association；
//! - 幂等重跑：stats 全零、行数不变（advisory lock 串行化下重复调用安全）；
//! - 隔离：夹具清理只动 `z-test-cov-*` 资源类型。
//!
//! 运行：`bash scripts/cargo-test.sh -p common --test ngac_coverage_test -- --ignored --test-threads=1`
//!（需 `aliothstudio_test`：`bash scripts/db/reset-db.sh --test` 预置 operator/employee UA、
//! default 策略类与 read/write/delete 基础 right——Gateway seed 自愈链供给）。

use common::ngac_grants::ensure_org_resource_coverage;
use common::testing::connect_test_db;
use sqlx::PgPool;
use std::sync::LazyLock;
use tokio::sync::{Mutex, MutexGuard};

/// 文件级串行化（夹具共享同组资源类型，cleanup 全局清空）。
static SERIAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn lock_serial() -> MutexGuard<'static, ()> {
    SERIAL_LOCK.lock().await
}

const RTS: [&str; 2] = ["z-test-cov-alpha", "z-test-cov-beta"];

async fn cleanup(pool: &PgPool) {
    for rt in RTS {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type = $1",
        )
        .bind(rt)
        .fetch_all(pool)
        .await
        .unwrap();
        for id in ids {
            sqlx::query("DELETE FROM isahl_auth.ngac_association WHERE fk_object_attribute = $1")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
            sqlx::query("DELETE FROM isahl_auth.ngac_object_attribute WHERE id = $1")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
        }
        sqlx::query("DELETE FROM isahl_auth.ngac_ownership_policy WHERE resource_type = $1")
            .bind(rt)
            .execute(pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "needs test DB"]
async fn coverage_creates_rows_and_is_idempotent() {
    let _g = lock_serial().await;
    let pool = connect_test_db().await;
    cleanup(&pool).await;

    let types: Vec<String> = RTS.iter().map(|s| s.to_string()).collect();

    // ── 首跑：三面落库 ──
    let first = ensure_org_resource_coverage(&pool, &types)
        .await
        .expect("first ensure");
    assert_eq!(first.collection_oas_created, 2, "两个新类型各建一行集合 OA");
    assert_eq!(
        first.ownership_defaults_created, 2,
        "两行 ownership 默认（enabled=false）"
    );
    assert_eq!(
        first.operator_associations_written, 2,
        "operator × 两集合 OA read association"
    );

    for rt in RTS {
        // 集合 OA：default 类 + `-collection` 名制（§7.3 种子先例）
        let oa: Option<(String, String)> = sqlx::query_as(
            "SELECT oa.o_name, pc.o_name FROM isahl_auth.ngac_object_attribute oa \
             JOIN isahl_auth.ngac_policy_class pc ON pc.id = oa.fk_policy_class \
             WHERE oa.resource_type = $1 AND oa.fk_resource = 0 AND oa.deleted_at IS NULL",
        )
        .bind(rt)
        .fetch_optional(&pool)
        .await
        .unwrap();
        let (o_name, pc) = oa.expect("集合 OA 行存在");
        assert_eq!(o_name, format!("{rt}-collection"));
        assert_eq!(pc, "default", "绑定 default 策略类");

        // ownership 默认行：enabled=false + owner→operator + benefit→employee
        let own: Option<(bool, String, String)> = sqlx::query_as(
            "SELECT op.enabled, oua.o_name, mua.o_name \
             FROM isahl_auth.ngac_ownership_policy op \
             JOIN isahl_auth.ngac_user_attribute oua ON oua.id = op.owner_attr_id \
             JOIN isahl_auth.ngac_user_attribute mua ON mua.id = op.benefit_attr_id \
             WHERE op.resource_type = $1",
        )
        .bind(rt)
        .fetch_optional(&pool)
        .await
        .unwrap();
        let (enabled, owner, benefit) = own.expect("ownership 默认行存在");
        assert!(
            !enabled,
            "安全缺省：enabled=false——覆盖 ensure 不放行任何行级写"
        );
        assert_eq!(
            owner, "operator",
            "owner 槽 = operator（seed org 三集合先例同族）"
        );
        assert_eq!(benefit, "employee", "benefit 槽 = employee");

        // operator read association 落在集合 OA 上
        let ops_read: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM isahl_auth.ngac_association a \
             JOIN isahl_auth.ngac_user_attribute ua ON ua.id = a.fk_user_attribute \
             JOIN isahl_auth.ngac_object_attribute oa ON oa.id = a.fk_object_attribute \
             WHERE ua.o_name = 'operator' AND oa.resource_type = $1 AND oa.fk_resource = 0 \
               AND a.deleted_at IS NULL \
               AND EXISTS (SELECT 1 FROM isahl_auth.ngac_access_right r \
                            WHERE r.id = ANY(a.ak_access_rights) AND r.o_name = 'read')",
        )
        .bind(rt)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(ops_read, 1, "operator UA 对集合 OA 持 read");
    }

    // ── 幂等重跑：stats 全零、行数不变 ──
    let oa_count_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = ANY($1) AND deleted_at IS NULL",
    )
    .bind(&types)
    .fetch_one(&pool)
    .await
    .unwrap();

    let second = ensure_org_resource_coverage(&pool, &types)
        .await
        .expect("second ensure");
    assert_eq!(second.collection_oas_created, 0, "集合 OA 幂等");
    assert_eq!(
        second.ownership_defaults_created, 0,
        "ownership NOT EXISTS 防重"
    );
    assert_eq!(
        second.operator_associations_written, 0,
        "grant_rights 加性：已含 read ⇒ 零写入（不空转 bump 触发器）"
    );

    let oa_count_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM isahl_auth.ngac_object_attribute \
         WHERE resource_type = ANY($1) AND deleted_at IS NULL",
    )
    .bind(&types)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(oa_count_before, oa_count_after, "重跑不增行");

    cleanup(&pool).await;
}
