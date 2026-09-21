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

/// 诉求叶表（列清单与 `cont-sales` 同构；镜像落**同表**、主体互换）。
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

/// 「我」槽位不变量（2026-09-14 裁决 + 2026-09-17 双账本细化）：
/// - 采购叶（我方采购账）「我」MUST 在甲（P1）；诉求叶（客户诉求账）「我」MUST 在乙（P2）；
/// - 销售叶双承载：我方销售「我」在乙（P2）、他方销售（采购对的同序镜像）「我」在甲（P1）
///   —— 故销售叶断言放宽为「我」MUST 在甲或乙（缺任一即违约）；
/// - 配对一律**不互换**主体序（2026-09-17 裁决「我方采购 配对 他方销售」同交易双账本，
///   取代 2026-09-14「甲/乙互换」）：镜像行 = 同一交易的对方账本，甲乙角色与正本一致。
fn ensure_actor_slot(input: &ContractRowInput<'_>) -> Result<(), AliothError> {
    let Some(actor) = input.actor.as_ref() else {
        return Ok(());
    };
    // 槽位序（0-based）：采购叶「我」在甲（P1）；诉求叶在乙（P2）；销售叶双承载（P1 或 P2）
    let match_slot = match input.leaf {
        ContractLeaf::Purchase => input
            .parties
            .first()
            .and_then(|p| p.subject_id)
            .map(|s| s == actor.subject_id)
            .unwrap_or(false),
        ContractLeaf::Sales => input
            .parties
            .iter()
            .take(2)
            .any(|p| p.subject_id == Some(actor.subject_id)),
        ContractLeaf::Request => input
            .parties
            .get(1)
            .and_then(|p| p.subject_id)
            .map(|s| s == actor.subject_id)
            .unwrap_or(false),
    };
    if !match_slot {
        return Err(AliothError::Validation {
            field: "parties".into(),
            message: format!(
                "ACTOR_SLOT_MISMATCH: {} 叶的「我」槽位不匹配（主体 {}，实际 {:?}）——采购叶须甲=我、诉求叶须乙=我、销售叶甲或乙=我",
                input.leaf.table(),
                actor.subject_id,
                input.parties.iter().take(2).map(|p| p.subject_id).collect::<Vec<_>>()
            ),
        });
    }
    if let Some(required) = input.require_view {
        if !actor.has_view(required) {
            return Err(AliothError::Validation {
                field: "actor".into(),
                message: format!(
                    "ACTOR_VIEW_MISSING: 主体 {} 缺岗位视角 {}（现有 {:?}）",
                    actor.subject_id, required, actor.view_tags
                ),
            });
        }
    }
    Ok(())
}

/// 单张合约行落库 + 合同方行（+ 可选草稿状态桥）+ **行级 NGAC 注册**。返回合约 id。
pub async fn insert_contract_row_tx(
    conn: &mut PgConnection,
    input: &ContractRowInput<'_>,
) -> Result<i64, AliothError> {
    ensure_actor_slot(input)?;
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

    // 行级 NGAC：本 crate 单源注册（所有经本写件的合同行——主/镜像、各 ns 各链——天然覆盖）。
    // 失败**不阻断**写链（对齐下沉前服务侧语义：`warn` 留痕 + 继续），避免为共享 crate 引入新的失败面。
    if let Err(e) = crate::ngac::register_contract_row_ngac_tx(&mut *conn, id, input.user_id).await
    {
        common::telemetry::warn!("合同行级 NGAC 注册失败（contract {id}）: {e}");
    }

    Ok(id)
}

/// 合同方名称解析（主体 id → `zc_id_subjects.notice`，单源）——批注 be104c08：
/// `zc_id_contract_rr_party.notice` 是**主体名快照**（读侧「甲方/乙方」列直取该列），
/// 角色语义只由 `code`（P1/P2）与 `ck_contract_role` 承载，禁把「甲方/乙方/采购方」等角色词当名字落库。
///
/// 单次批量查（`= ANY` IN 列表，保序）；主体缺行（悬空引用）→ `None`，调用方自行兜底。
pub async fn resolve_party_names(
    conn: &mut PgConnection,
    ids: &[i64],
) -> Result<Vec<Option<String>>, AliothError> {
    let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
        r#"SELECT id, notice FROM "isahl"."zc_id_subjects"
           WHERE id = ANY($1) AND deleted_at IS NULL"#,
    )
    .bind(ids)
    .fetch_all(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    let mut out = vec![None; ids.len()];
    for (id, name) in rows {
        if let Some(pos) = ids.iter().position(|x| *x == id) {
            out[pos] = name.filter(|n| !n.trim().is_empty());
        }
    }
    Ok(out)
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
///
/// 方向 = 叶子语义固化（用户裁决 2026-09-17，先语义判定再有位置判定）：需求性合约
/// （我方采购=采购叶、客户诉求=诉求叶）恒落 `ref_left`，供给性合约（我方销售=销售叶）
/// 恒落 `ref_right`，与创建顺序解耦；两条配对轴 = 采购↔销售、诉求↔销售（同日裁决
/// 「除了采购，客户诉求→我方销售」——诉求对镜像落销售叶，不再同表）。
async fn insert_symmetry_bridge_tx(
    conn: &mut PgConnection,
    main_id: i64,
    mirror_id: i64,
    main_leaf: ContractLeaf,
    user_id: i64,
) -> Result<(), AliothError> {
    let (left_id, right_id) = if main_leaf.is_supply_nature() {
        (mirror_id, main_id)
    } else {
        (main_id, mirror_id)
    };
    sqlx::query(
        r#"INSERT INTO "isahl"."zc_id_contract_rr_symmetry"
           (id, ref_left, ref_right, notice, code, comments, created_by_id)
           VALUES (isahl.gen_next_uid($1), $2, $3, '一式两份', $4, $5, $6)"#,
    )
    .bind(SYMMETRY_TABLE_CODE)
    .bind(left_id)
    .bind(right_id)
    .bind(format!("MIR-{main_id}-{mirror_id}"))
    .bind(format!(
        "对称合约：需求侧 {left_id} ↔ 供给侧 {right_id}（正本 {main_id}，镜像 {mirror_id}）"
    ))
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(AliothError::from_sqlx)?;
    Ok(())
}

/// 主 + 镜像成对落库（镜像叶 = `leaf.mirrored()`；`code = {主code}-R`；MIR 桥）。
/// **同交易双账本（2026-09-17 裁决）：合同方一律不互换**——镜像行 = 同一交易的对方账本
/// （我方采购↔他方销售 / 客户诉求↔我方销售 / 手建我方销售↔客户诉求行），甲乙角色与正本一致；
/// 取代 2026-09-14「甲/乙互换」口径。
/// 返回 `(主, 镜像)`。
pub async fn insert_contract_pair_tx(
    conn: &mut PgConnection,
    main: &ContractRowInput<'_>,
) -> Result<(i64, i64), AliothError> {
    let main_id = insert_contract_row_tx(conn, main).await?;
    let mirror_code = main.mirrored_code();
    let mirror_leaf = main.leaf.mirrored();
    let mirror_parties = main.parties.clone();
    let mirror_input = ContractRowInput {
        leaf: mirror_leaf,
        code: &mirror_code,
        notice: main.notice,
        comments: main.comments,
        parties: mirror_parties,
        actor: main.actor.clone(),
        require_view: main.require_view,
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
    insert_symmetry_bridge_tx(conn, main_id, mirror_id, main.leaf, main.user_id).await?;
    Ok((main_id, mirror_id))
}

/// 为既有主合约补建镜像（`mirror` 入参自带方向/编号/合同方顺序）+ MIR 桥。
///
/// 调用方负责「甲/乙互换 + `code = {主code}-R`」的入参构造——语义由调用点显式持有，
/// 便于续约/分单等非对称路径复用同一写件。
pub async fn insert_mirror_of_contract_tx(
    conn: &mut PgConnection,
    main_id: i64,
    mirror: &ContractRowInput<'_>,
) -> Result<i64, AliothError> {
    let mirror_id = insert_contract_row_tx(conn, mirror).await?;
    insert_symmetry_bridge_tx(
        conn,
        main_id,
        mirror_id,
        mirror.leaf.mirrored(),
        mirror.user_id,
    )
    .await?;
    Ok(mirror_id)
}
