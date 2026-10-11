//! 合同族**行级 NGAC 锚点注册**（写件内单源；derive-creator-grants-from-lifecycle 修正案）。
//!
//! 创建者授权已由判定时刻派生（`ngac_host::creator_derived_permits` / `CREATOR_DERIVED_EXISTS_OA`），
//! 本模块只落**锚点单行**（类型化枚举 join 键 + 管理员增量授权对象锚），不再扇出
//! `ngac_association` 创建者关联。
//!
//! 背景：NGAC 行级判定**无 collection fallback**——锚点缺行时清单不可见（判定仍由派生放行）。
//! 合同行的写入方有多条链：
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

/// 为合同族行注册行级 NGAC **锚点单行**（`ngac_object_attribute`；零授权语义）。
///
/// `item_id` = 合同行 id；`user_id` = 创建者（落 `created_by_id` 供派生/审计）。
/// 与调用方事务同连接（`&mut PgConnection`），注册随业务写一同提交/回滚。
pub async fn register_contract_row_ngac_tx(
    conn: &mut PgConnection,
    item_id: i64,
    user_id: i64,
) -> Result<(), AliothError> {
    // 授权写入单源 = common::ngac_grants（consolidate-ngac-grant-authoring）：
    // 事务内逐函数调用（`&mut *conn` 重借），与业务写同连接、同提交/回滚。
    let spec = common::ngac_grants::ResourceOaSpec {
        o_name: format!("{CONTRACT_RESOURCE_TYPE}-{item_id}"),
        resource_type: CONTRACT_RESOURCE_TYPE,
        resource_id: item_id,
        resource_identifier: None,
        created_by: Some(user_id),
        policy_class_name: None,
    };
    common::ngac_grants::ensure_resource_oa_tx(&mut *conn, &spec).await?;
    // 创建者授权：不物化（派生层唯一实现 = common::ngac_org；deny-overrides 同权）
    Ok(())
}
