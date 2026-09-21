//! 合同族**行级 NGAC 注册**（写件内单源）。
//!
//! 背景：NGAC 行级判定**无 collection fallback**——`isahl_auth.ngac_object_attribute` 缺行时
//! 连创建者本人也不可见。合同行的写入方有多条链：
//! ① 合同服务 create / 续约（`insert_contract_pair_tx` / `insert_mirror_of_contract_tx`）；
//! ② `transport-dispatch` 询价 / 报价 / 选商分单（同上两写件）；
//! ③ `consignment-writer` 下单合同对（同上）。
//! 此前**只有合同服务侧**在外部显式注册，②③ 链的合同行（主 + 镜像）无行级属性
//! ⇒ 行级判定对被授权者 403。
//!
//! 用户裁决（2026-09-14）：把注册**下沉本 crate**，令所有经本写件落库的合同行天然覆盖，
//! 消除「按调用方记得注册」的隐性契约。
//!
//! 幂等：`ON CONFLICT … DO NOTHING`；失败仅 `warn` 不阻断写链（但必须留痕——静默即行不可见）。

use sqlx::PgConnection;

use common::AliothError;

/// 合同行的 NGAC 资源类型（与服务侧行级判定一致）。
pub const CONTRACT_RESOURCE_TYPE: &str = "contracts";

/// 为合同族行注册行级 NGAC：对象属性（`ngac_object_attribute`）+ 属主关联（`ngac_association`）。
///
/// `item_id` = 合同行 id；`user_id` = 创建者（关联其全部 NGAC 用户属性，授予 read/write/delete/update/create）。
/// 与调用方事务同连接（`&mut PgConnection`），注册随业务写一同提交/回滚。
pub async fn register_contract_row_ngac_tx(
    conn: &mut PgConnection,
    item_id: i64,
    user_id: i64,
) -> Result<(), AliothError> {
    // 对象属性（无 policy class 行时不写：`fk_policy_class` 由子查询取首个）
    // 注：`id` 显式取 `isahl.gen_next_zuid()`——与同 crate 合约行/产品行插入同范式，
    // 且不依赖各环境 `isahl_auth` 表的 `id` DEFAULT（测试库曾实测缺省致 23502）。
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_object_attribute
           (id, o_name, fk_policy_class, resource_type, fk_resource, created_by_id)
           VALUES (isahl.gen_next_zuid(), $1, (SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1), $2, $3, $4)
           ON CONFLICT(resource_type, fk_resource) DO NOTHING"#,
    )
    .bind(format!("{CONTRACT_RESOURCE_TYPE}-{item_id}"))
    .bind(CONTRACT_RESOURCE_TYPE)
    .bind(item_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;

    // 属主关联：创建者的全部 NGAC 用户属性 → 本对象属性，授予全套权限（已存在则跳过）
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_association
           (id, fk_user_attribute, fk_object_attribute, ak_access_rights, fk_policy_class, created_at)
           SELECT isahl.gen_next_zuid(), rr.fk_user_attribute, oa.id,
                  ARRAY(SELECT id FROM isahl_auth.ngac_access_right
                        WHERE o_name IN ('read','write','delete','update','create')),
                  oa.fk_policy_class, NOW()
           FROM isahl_auth.ngac_user_rr_attribute rr
           JOIN isahl_auth.ngac_object_attribute oa
             ON oa.resource_type = $2 AND oa.fk_resource = $3 AND oa.deleted_at IS NULL
           WHERE rr.fk_user = $1 AND rr.deleted_at IS NULL
             AND NOT EXISTS (
                 SELECT 1 FROM isahl_auth.ngac_association a2
                 WHERE a2.fk_user_attribute = rr.fk_user_attribute
                   AND a2.fk_object_attribute = oa.id AND a2.deleted_at IS NULL)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(user_id)
    .bind(CONTRACT_RESOURCE_TYPE)
    .bind(item_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;

    Ok(())
}
