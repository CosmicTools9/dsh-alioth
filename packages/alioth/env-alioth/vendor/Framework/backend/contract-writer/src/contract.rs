//! 合约行与一式两份镜像写件。
//!
//! 叶表路由：销售向 `zc_id_cont-transport-sales` / 采购向 `zc_id_cont-transport-purchase` / 诉求 `zc_id_cont-request`
//! （表继承 `zc_id_contract`，编号查重与软删识别一律走继承根表）。

use sqlx::PgConnection;

use common::AliothError;
use trigger_registry::lifecycle::derive_form_type;

use crate::models::{ContractLeaf, ContractParty, ContractRowInput};

/// 合同方关系表编号（`isahl.gen_next_uid` 表码；非 lifecycle 族）
const PARTY_TABLE_CODE: i64 = 671;
/// 一式两份互链桥编号
const SYMMETRY_TABLE_CODE: i64 = 231;

const SALES_INSERT: &str = r#"INSERT INTO "isahl"."zc_id_cont-transport-sales"
   (id, code, notice, comments, o_number, projection, qk_date, "qk_valid-segm", tpl_id, lk_health,
    dk_scene, dk_factor, dk_function, "_f_", "_t_", created_by_id, updated_by_id,
    ak_permit_user, ak_access_user)
   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9,
           (SELECT id FROM "isahl"."zc_id_scene" WHERE code = $10 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_factor" WHERE code = $11 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_function" WHERE code = $12 LIMIT 1),
           $13, $14, $15, $15, ARRAY[$16]::bigint[], ARRAY[$16]::bigint[])
   RETURNING id"#;

const PURCHASE_INSERT: &str = r#"INSERT INTO "isahl"."zc_id_cont-transport-purchase"
   (id, code, notice, comments, o_number, projection, qk_date, "qk_valid-segm", tpl_id, lk_health,
    dk_scene, dk_factor, dk_function, "_f_", "_t_", created_by_id, updated_by_id,
    ak_permit_user, ak_access_user)
   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9,
           (SELECT id FROM "isahl"."zc_id_scene" WHERE code = $10 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_factor" WHERE code = $11 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_function" WHERE code = $12 LIMIT 1),
           $13, $14, $15, $15, ARRAY[$16]::bigint[], ARRAY[$16]::bigint[])
   RETURNING id"#;

/// 诉求叶表（列清单与 `cont-sales` 同构；镜像落**同表**，合同方与主行相同）。
const REQUEST_INSERT: &str = r#"INSERT INTO "isahl"."zc_id_cont-request"
   (id, code, notice, comments, o_number, projection, qk_date, "qk_valid-segm", tpl_id, lk_health,
    dk_scene, dk_factor, dk_function, "_f_", "_t_", created_by_id, updated_by_id)
   VALUES (isahl.gen_next_zuid(), $1, $2, $3, $4, $5, $6, $7, $8, $9,
           (SELECT id FROM "isahl"."zc_id_scene" WHERE code = $10 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_factor" WHERE code = $11 LIMIT 1),
           (SELECT id FROM "isahl"."zc_id_function" WHERE code = $12 LIMIT 1),
           $13, $14, $15, $15)
   RETURNING id"#;

/// 形态派生（单一源）：职能码 → (`_f_`, `_t_`)。
///
/// DB 无 `tf_bf_lifecycle__f__type` 触发器（dev/wz/test 三库实证）→ 由本 crate 派生后参数绑定；
/// 公开 API 只收职能码，调用方无法手写字面量对（`ALIOTH_ONTOLOGY_SPEC.md` §4.3.3 形态 1）。
fn derive_form(fn_code: &str) -> Result<(&'static str, &'static str), AliothError> {
    derive_form_type(fn_code).ok_or_else(|| AliothError::Validation {
        field: "fnCode".into(),
        message: format!("职能码 {fn_code} 无法派生 _f_/_t_（须为 !./!_/↑./↑_/↓./↓_ 六前缀之一）"),
    })
}

/// 合同编号唯一性（软删不计；继承根表查重自动覆盖叶表行）。
async fn ensure_contract_code_unique(
    conn: &mut PgConnection,
    code: &str,
) -> Result<(), AliothError> {
    let taken: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (SELECT 1 FROM "isahl"."zc_id_contract" WHERE code = $1 AND deleted_at IS NULL)"#,
    )
    .bind(code)
    .fetch_one(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    if taken {
        return Err(AliothError::Conflict(format!("合同编号 {code} 已存在")));
    }
    Ok(())
}

/// 单张合约行落库 + 合同方行（+ 可选草稿状态桥）。返回合约 id。
pub async fn insert_contract_row_tx(
    conn: &mut PgConnection,
    input: &ContractRowInput<'_>,
) -> Result<i64, AliothError> {
    let (form, tier) = derive_form(input.fn_code)?;
    ensure_contract_code_unique(conn, input.code).await?;

    let sql = match input.leaf {
        ContractLeaf::Sales => SALES_INSERT,
        ContractLeaf::Purchase => PURCHASE_INSERT,
        ContractLeaf::Request => REQUEST_INSERT,
    };
    let id: i64 = sqlx::query_scalar(sql)
        .bind(input.code)
        .bind(input.notice)
        .bind(input.comments)
        .bind(input.o_number.unwrap_or(""))
        .bind(input.projection.unwrap_or(""))
        .bind(input.qk_date_id)
        .bind(input.qk_valid_segm_id)
        .bind(input.tpl_id)
        .bind(input.lk_health)
        .bind(input.scene_code)
        .bind(input.factor_code)
        .bind(input.fn_code)
        .bind(form)
        .bind(tier)
        .bind(input.user_id)
        // 行级权属（D5）：合同方/合同行同落当前操作者 uid
        .bind(input.user_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?;

    insert_parties_tx(conn, id, &input.parties, input.user_id).await?;

    if let Some(draft_id) = input.draft_status_id {
        // 模型约束 `UNIQUE (ref_left)` 覆盖含软删行：不能只在「无未删行」时插入，
        // 否则软删残留会撞唯一键（镜像/续约路径会先软删再写）。改为按 ref_left 插入或复活。
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_lifecycle_r_primary-status"
               (ref_left, ref_right, notice, created_by_id)
               VALUES ($1, $2, '初始状态', $3)
               ON CONFLICT (ref_left) DO UPDATE SET ref_right = EXCLUDED.ref_right,
                 updated_at = NOW(), deleted_at = NULL, deleted_by_id = NULL"#,
        )
        .bind(id)
        .bind(draft_id)
        .bind(input.user_id)
        .execute(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?;
    }

    Ok(id)
}

/// 合同方行（`code` = `P{序号}`；主体缺省仅登记名称）。
pub async fn insert_parties_tx(
    conn: &mut PgConnection,
    contract_id: i64,
    parties: &[ContractParty],
    user_id: i64,
) -> Result<(), AliothError> {
    for (idx, party) in parties.iter().enumerate() {
        let role_code = ["PARTY-A", "PARTY-B", "PARTY-C"]
            .get(idx)
            .copied()
            .unwrap_or("PARTY-C");
        sqlx::query(
            r#"INSERT INTO "isahl"."zc_id_contract_rr_party"
               (id, ref_left, notice, code, ref_right, ck_contract_role, qk_period, created_by_id)
               VALUES (isahl.gen_next_uid($1), $2, $3, $4, $5,
                       (SELECT id FROM "isahl"."zc_id_cate-contact_role"
                        WHERE code = $6 AND deleted_at IS NULL LIMIT 1),
                       $7, $8)"#,
        )
        .bind(PARTY_TABLE_CODE)
        .bind(contract_id)
        .bind(&party.name)
        .bind(format!("P{}", idx + 1))
        .bind(party.subject_id)
        .bind(role_code)
        .bind(party.period_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await
        .map_err(AliothError::from_sqlx)?;
    }
    Ok(())
}

/// 一式两份互链桥（叶桥 `zc_id_contract_rr_symmetry`；逐行业务编号，禁类型常量比较）。
async fn insert_symmetry_bridge_tx(
    conn: &mut PgConnection,
    main_id: i64,
    mirror_id: i64,
    user_id: i64,
) -> Result<(), AliothError> {
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_contract_rr_symmetry"
           (id, ref_left, ref_right, notice, code, comments, created_by_id)
           VALUES (isahl.gen_next_uid($1), $2, $3, '一式两份', $4, $5, $6)"#,
    )
    .bind(SYMMETRY_TABLE_CODE)
    .bind(main_id)
    .bind(mirror_id)
    .bind(format!("MIR-{main_id}-{mirror_id}"))
    .bind(format!("镜像单据：主合同 {main_id} ↔ 反向合同 {mirror_id}"))
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    Ok(())
}

/// 主 + 镜像成对落库（镜像叶 = `leaf.mirrored()`；合同方与主合同**逐字段相同**；`code = {主code}-R`；MIR 桥）。
/// 返回 `(主, 镜像)`。
///
/// 甲/乙方**不互换**（用户裁决 2026-09-13）：镜像 = 同一单据的对方账副本（同主体/同角色/同结算方），
/// 双边性由镜像行自身的**属权**表达（行级 NGAC 注册；见调用方），不靠主体对调。
pub async fn insert_contract_pair_tx(
    conn: &mut PgConnection,
    main: &ContractRowInput<'_>,
) -> Result<(i64, i64), AliothError> {
    let main_id = insert_contract_row_tx(conn, main).await?;
    let mirror_code = main.mirrored_code();
    let mirror_input = ContractRowInput {
        leaf: main.leaf.mirrored(),
        code: &mirror_code,
        notice: main.notice,
        comments: main.comments,
        parties: main.parties.clone(),
        fn_code: main.fn_code,
        scene_code: main.scene_code,
        factor_code: main.factor_code,
        qk_date_id: main.qk_date_id,
        qk_valid_segm_id: main.qk_valid_segm_id,
        o_number: main.o_number,
        projection: main.projection,
        tpl_id: main.tpl_id,
        lk_health: main.lk_health,
        draft_status_id: main.draft_status_id,
        user_id: main.user_id,
    };
    let mirror_id = insert_contract_row_tx(conn, &mirror_input).await?;
    insert_symmetry_bridge_tx(conn, main_id, mirror_id, main.user_id).await?;
    Ok((main_id, mirror_id))
}

/// 为既有主合约补建镜像（`mirror` 入参自带目标叶/编号/合同方）+ MIR 桥。
///
/// 合同方 MUST 与主合同**逐字段相同**（甲/乙不互换、`ref_right`/角色码/结算方一致）——
/// 入参构造由调用方持有（本函数只落库，不改变语义），便于续约/分单等非对称路径复用同一写件。
/// 双边性由镜像行属权（行级 NGAC 注册）表达，调用方负责注册。
pub async fn insert_mirror_of_contract_tx(
    conn: &mut PgConnection,
    main_id: i64,
    mirror: &ContractRowInput<'_>,
) -> Result<i64, AliothError> {
    let mirror_id = insert_contract_row_tx(conn, mirror).await?;
    insert_symmetry_bridge_tx(conn, main_id, mirror_id, mirror.user_id).await?;
    Ok(mirror_id)
}
