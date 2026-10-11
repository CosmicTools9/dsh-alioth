//! 镜像行**行级权属**（NGAC）回归（change: `fix-mirror-ownership-fields`）
//!
//! 被测单元 = `consignment_writer::insert_order_mirror_tx`——委托链（`writer.rs`）与
//! 派车运单链（`waybill-writer/src/dispatch.rs`）**共用单一实现**。
//!
//! 判据（正本 `docs/specs/WZ_EXTERNAL_CHAIN_SPEC.md` §3.1「镜像行字段口径」）：
//! 镜像行 `ak_permit_user` / `ak_access_user` MUST 与主行同源 = 入参 `user_id`（真实操作者）——
//! MUST NOT 硬编码 `1`（system 占位）、MUST NOT 留空。依据：主行一律
//! `ak_permit_user = ak_access_user = ARRAY[actor]`，且 `common/src/permissions.rs` 的读径
//! 分别消费两列（`ak_permit_user @> ARRAY[uid]` / `ak_access_user @> ARRAY[uid]`）⇒ 留空即
//! 镜像行对操作者与其组织不可见（门户/平台读径两侧失真，编译与既有测试均不报错）。
//!
//! 覆盖缺陷：修复前镜像 INSERT 无 `ak_access_user` 列（该断言读回 NULL 即失败），
//! 且派车链入参硬编码 `user_id: 1`（本用例的 `OPERATOR ≠ 1` 使常量落库同样被断言拦下）。
//!
//! 数据纪律：行码带 `MIR-OWN-TEST-{suffix}` 后缀；用例全程在**单事务内写入并回滚**
//! （零残留，MUST NOT 触碰既有夹具数据）。

use common::testing::connect_test_db;
use consignment_writer::{insert_order_mirror_tx, OrderMirrorInput};
use sqlx::{PgPool, Row};

/// 操作者 uid：故意 ≠ 1（system 占位）且 ≠ 常见夹具值——证明落库值来自入参而非常量。
const OPERATOR: i64 = 9_999_700_000_123;

fn suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}{}", nanos % 0xF_FFFF_FFFF, std::process::id() % 100)
}

/// 主单据行（`zc_id_orde-land`，坐标 TX/FJA/↓_EV = `WriterDk::OrderDocument`）；
/// 不写状态桥 ⇒ 镜像状态同步为 no-op（与「新建即镜像」实况一致）。
async fn insert_main_order(tx: &mut sqlx::PgConnection, code: &str, actor: i64) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_orde-land"
             (id, code, notice, comments, fk_subject, fk_object, created_by_id,
              ak_permit_user, ak_access_user, dk_scene, dk_factor, dk_function)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, NULL, NULL, $4,
                   ARRAY[$4]::bigint[], ARRAY[$4]::bigint[],
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'TX'  AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'FJA' AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↓_EV' AND deleted_at IS NULL))
           RETURNING id"#,
    )
    .bind(code)
    .bind("镜像权属回归主行")
    .bind("主行")
    .bind(actor)
    .fetch_one(&mut *tx)
    .await
    .expect("insert main order")
}

#[tokio::test]
async fn mirror_row_ownership_columns_carry_real_operator() {
    let pool: PgPool = connect_test_db().await;
    let main_code = format!("MIR-OWN-TEST-{}", suffix());
    let mirror_code = format!("{main_code}-R");

    let mut tx = pool.begin().await.expect("begin tx");
    let main_id = insert_main_order(&mut tx, &main_code, OPERATOR).await;

    let mirror_id = insert_order_mirror_tx(
        &mut tx,
        main_id,
        &OrderMirrorInput {
            code: &mirror_code,
            notice: "镜像权属回归（镜像）",
            comments: "",
            // 甲/乙互换由调用方给定；本用例只验证行级权属列，故两侧留空
            subject: None,
            object: None,
            qk_date: None,
            fk_contract: None,
            fn_code: "↓_BE",
            kind_label: "订单",
            user_id: OPERATOR,
        },
    )
    .await
    .expect("insert mirror order");

    let row = sqlx::query(
        r#"SELECT code, ak_permit_user, ak_access_user FROM isahl."zc_id_orde-land" WHERE id = $1"#,
    )
    .bind(mirror_id)
    .fetch_one(&mut *tx)
    .await
    .expect("read mirror row");

    let code: String = row.get("code");
    let permit: Option<Vec<i64>> = row.get("ak_permit_user");
    let access: Option<Vec<i64>> = row.get("ak_access_user");

    assert_eq!(code, mirror_code, "镜像行编号 = 主行 code + '-R'");
    assert_eq!(
        permit,
        Some(vec![OPERATOR]),
        "镜像行 ak_permit_user MUST = 真实操作者 uid（MUST NOT 硬编码 1=system）"
    );
    assert_eq!(
        access,
        Some(vec![OPERATOR]),
        "镜像行 ak_access_user MUST = 真实操作者 uid（MUST NOT 留空——读径行级过滤消费该列）"
    );

    // 互链桥（唯一合法识别判据 = 叶子桥 `zc_id_lifecycle_rr_form` 的 `MIR-%` 行）
    let linked: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM isahl."zc_id_lifecycle_rr_form"
           WHERE ref_left = $1 AND ref_right = $2 AND code LIKE 'MIR-%' AND deleted_at IS NULL"#,
    )
    .bind(main_id)
    .bind(mirror_id)
    .fetch_one(&mut *tx)
    .await
    .expect("probe mirror bridge");
    assert_eq!(linked, 1, "镜像互链桥 MUST 由同一实现写入（MIR-%）");

    tx.rollback().await.expect("rollback（零残留）");
}
