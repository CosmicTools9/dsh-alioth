//! contract-writer 集成测试（真库：`common::testing::connect_test_db` → `*_test` 库）。
//!
//! 覆盖：主+镜像成对落库（相反叶表 / 甲乙互换 / `-R` 编号 / MIR 桥）、形态派生、
//! 编号冲突拒绝、非法职能码拒绝、产品行双族、镜像级联软删。

use sqlx::PgPool;

use common::testing::connect_test_db;
use contract_writer::{
    create_contract_transport_product_tx, insert_contract_pair_tx, insert_contract_row_tx,
    insert_mirror_of_contract_tx, insert_product_pair_tx, insert_product_row_tx,
    resolve_mirror_ids_tx, soft_delete_contract_tx, soft_delete_mirror_tx, ContractLeaf,
    ContractParty, ContractProductInput, ContractRowInput, ProductPairInput, ProductRowInput,
};

const SCENE: &str = "TD";
const FACTOR: &str = "FJA";

fn unique_code(tag: &str) -> String {
    format!("T-CW-{tag}-{}", chrono_like_millis())
}

fn chrono_like_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_millis() as i64
}

/// 测试主体（幂等：按 code 判重）。
async fn ensure_subject(pool: &PgPool, code: &str) -> i64 {
    let id: Option<i64> = sqlx::query_scalar(
        // 类契约（ALIOTH_ONTOLOGY_SPEC §4.3.3 形态 1）：提供 dk_function 派生源，夹具不手写 _f_/_t_；
        // '↓.GG'（实现·范例）为本 crate 测试链同款坐标，dev/test 库均已种子。
        r#"INSERT INTO isahl."zc_id_orga-non-banking-legal" (notice, code, created_by_id, updated_by_id, dk_scene, dk_factor, dk_function)
           SELECT $1, $2, 1, 1,
                  (SELECT id FROM isahl."zc_id_scene"    WHERE code = 'TX'   AND deleted_at IS NULL LIMIT 1),
                  (SELECT id FROM isahl."zc_id_factor"   WHERE code = 'FJA'  AND deleted_at IS NULL LIMIT 1),
                  (SELECT id FROM isahl."zc_id_function" WHERE code = '↓_GG' AND deleted_at IS NULL LIMIT 1)
           WHERE NOT EXISTS (SELECT 1 FROM isahl."zc_id_orga-non-banking-legal" WHERE code = $2 AND deleted_at IS NULL)
           RETURNING id"#,
    )
    .bind(format!("合约写件测试主体-{code}"))
    .bind(code)
    .fetch_optional(pool)
    .await
    .expect("insert subject");
    match id {
        Some(id) => id,
        None => sqlx::query_scalar(
            r#"SELECT id FROM isahl."zc_id_subj-org" WHERE code = $1 AND deleted_at IS NULL"#,
        )
        .bind(code)
        .fetch_one(pool)
        .await
        .expect("fetch subject"),
    }
}

async fn draft_status_id(pool: &PgPool) -> Option<i64> {
    sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_stus-contract" WHERE code = 'draft' AND deleted_at IS NULL"#,
    )
    .fetch_optional(pool)
    .await
    .expect("draft status")
}

fn row_input<'a>(
    code: &'a str,
    leaf: ContractLeaf,
    parties: Vec<ContractParty>,
    draft: Option<i64>,
) -> ContractRowInput<'a> {
    ContractRowInput {
        leaf,
        code,
        notice: "集成测试合约",
        comments: "",
        parties,
        // 系统内部写（夹具未绑主体）：跳过「我」槽位与视角断言
        actor: None,
        require_view: None,
        fn_code: "↓.GG",
        scene_code: SCENE,
        factor_code: FACTOR,
        qk_date_id: None,
        qk_valid_segm_id: None,
        o_number: None,
        projection: None,
        tpl_id: None,
        lk_health: None,
        draft_status_id: draft,
        user_id: 1,
    }
}

fn product_input<'a>(
    code: &'a str,
    is_sales: bool,
    buyer: i64,
    seller: i64,
) -> ProductRowInput<'a> {
    ProductRowInput {
        is_sales,
        code,
        notice: "集成测试产品",
        comments: "",
        demand_subject: buyer,
        provider_subject: seller,
        line_id: None,
        vehicle_form_id: None,
        price_id: None,
        weight_id: None,
        period_id: None,
        previous_id: None,
        fn_code: "↓.GG",
        scene_code: SCENE,
        factor_code: FACTOR,
        user_id: 1,
    }
}

/// 漂移自检：参考测试库可能缺诉求叶表（ns 库均有）——缺表时以**自解释消息失败**（不静默跳过），
/// 明确指向环境漂移而非代码缺陷。证据：openspec/changes/archive/2026-09-11-fix-wz-writer-single-source-and-spec-drift/tasks.md §6.4/6.5。
async fn assert_request_leaf(pool: &PgPool) {
    let has_request_leaf: bool =
        sqlx::query_scalar(r#"SELECT to_regclass('"isahl"."zc_id_cont-request"') IS NOT NULL"#)
            .fetch_one(pool)
            .await
            .expect("regclass probe");
    assert!(
        has_request_leaf,
        "isahl.zc_id_cont-request 缺失 = 参考测试库 schema 漂移（ns 库 alioth/wz/avic_caacsec/cosmic_tools 均具备该叶表）\
         ——本用例须在已同步库运行；解除条件见 change 归档 tasks §6.4/6.5，非代码缺陷"
    );
}

/// NGAC 基线幂等自愈（对齐 contract 服务 `test_common::ensure_ngac_baseline` 范式）：
/// `ngac_object_attribute.fk_policy_class` 非空 + 写件取 `LIMIT 1`、`created_by_id` FK 到
/// `auth_users`——共享测试库被并行会话重置后两者 0 行 ⇒ 行级注册整段失败（warn 不阻断）、
/// NGAC 断言红。缺行即补 `default` 类 + 测试账号 1，与 approval/contract 测试 harness 同款自愈。
async fn ensure_ngac_policy_class(pool: &PgPool) {
    let has: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM isahl_auth.ngac_policy_class)")
            .fetch_one(pool)
            .await
            .expect("policy class probe");
    if !has {
        sqlx::query(
            r#"INSERT INTO isahl_auth.ngac_policy_class (o_name, description)
               VALUES ('default', 'contract-writer 测试默认策略类')"#,
        )
        .execute(pool)
        .await
        .expect("policy class seed");
    }
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES (1, 'test-user-1', 'test-user-1', 'test-user-1@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .execute(pool)
    .await
    .expect("auth user seed");
}

/// 测试残留清理（按编号前缀）。
async fn purge(pool: &PgPool, prefix: &str) {
    let like = format!("{prefix}%");
    for sql in [
        r#"DELETE FROM isahl."zc_id_contract_rr_symmetry"
           WHERE code LIKE $1
              OR ref_left IN (SELECT id FROM isahl."zc_id_contract" WHERE code LIKE $1)
              OR ref_right IN (SELECT id FROM isahl."zc_id_contract" WHERE code LIKE $1)"#,
        r#"DELETE FROM isahl."zc_id_contract_rr_party"
           WHERE ref_left IN (SELECT id FROM isahl."zc_id_contract" WHERE code LIKE $1)"#,
        r#"DELETE FROM isahl."zc_id_contract_rr_agreement"
           WHERE ref_left IN (SELECT id FROM isahl."zc_id_contract" WHERE code LIKE $1)"#,
        r#"DELETE FROM isahl."zc_id_contract_rr_deal"
           WHERE ref_left IN (SELECT id FROM isahl."zc_id_contract" WHERE code LIKE $1)"#,
        r#"DELETE FROM isahl."zc_id_lifecycle_r_primary-status"
           WHERE ref_left IN (SELECT id FROM isahl."zc_id_contract" WHERE code LIKE $1)"#,
        r#"DELETE FROM isahl."zc_id_prod-transport_rr_stop"
           WHERE ref_left IN (
             SELECT id FROM isahl."zc_id_prod-freight_road-sales" WHERE code LIKE $1
             UNION ALL
             SELECT id FROM isahl."zc_id_prod-freight_road-purchase" WHERE code LIKE $1)"#,
        r#"DELETE FROM isahl."zc_id_prod-freight_road-sales" WHERE code LIKE $1"#,
        r#"DELETE FROM isahl."zc_id_prod-freight_road-purchase" WHERE code LIKE $1"#,
        r#"DELETE FROM isahl."zc_id_cont-sales" WHERE code LIKE $1"#,
        r#"DELETE FROM isahl."zc_id_cont-purchase" WHERE code LIKE $1"#,
    ] {
        sqlx::query(sql)
            .bind(&like)
            .execute(pool)
            .await
            .expect("purge");
    }
    // 诉求叶表在参考测试库漂移时缺失（ns 库均有）——清理按存在性守卫，避免无关用例因清理语句
    // 解析失败而整体变红；断言侧保持严格（缺表时业务用例仍显式失败）。
    let has_request_leaf: bool =
        sqlx::query_scalar(r#"SELECT to_regclass('"isahl"."zc_id_cont-request"') IS NOT NULL"#)
            .fetch_one(pool)
            .await
            .expect("regclass probe");
    if has_request_leaf {
        sqlx::query(r#"DELETE FROM isahl."zc_id_cont-request" WHERE code LIKE $1"#)
            .bind(&like)
            .execute(pool)
            .await
            .expect("purge request leaf");
    }
}

#[tokio::test]
async fn contract_pair_lands_opposite_leaves_with_same_parties() {
    let pool = connect_test_db().await;
    ensure_ngac_policy_class(&pool).await;
    let code = unique_code("PAIR");
    let a = ensure_subject(&pool, "T-CW-SUBJ-A").await;
    let b = ensure_subject(&pool, "T-CW-SUBJ-B").await;
    let draft = draft_status_id(&pool).await;

    let parties = vec![
        ContractParty {
            subject_id: Some(a),
            name: "甲方".into(),
            period_id: None,
        },
        ContractParty {
            subject_id: Some(b),
            name: "乙方".into(),
            period_id: None,
        },
    ];
    let input = row_input(&code, ContractLeaf::Sales, parties, draft);
    let mut conn = pool.acquire().await.expect("acquire");

    let (main_id, mirror_id) = insert_contract_pair_tx(&mut conn, &input)
        .await
        .expect("insert pair");

    // 叶表配对（手建我方销售 → 客户诉求行）+ 编号 +R
    let main_leaf: (String, Option<String>, Option<String>) =
        sqlx::query_as(r#"SELECT code, "_f_", "_t_" FROM isahl."zc_id_cont-sales" WHERE id = $1"#)
            .bind(main_id)
            .fetch_one(&pool)
            .await
            .expect("main row");
    assert_eq!(main_leaf.0, code);
    assert_eq!(
        (main_leaf.1.as_deref(), main_leaf.2.as_deref()),
        (Some("实现"), Some("范例"))
    );

    let mirror_leaf: (String,) =
        sqlx::query_as(r#"SELECT code FROM isahl."zc_id_cont-request" WHERE id = $1"#)
            .bind(mirror_id)
            .fetch_one(&pool)
            .await
            .expect("mirror row");
    assert_eq!(mirror_leaf.0, format!("{code}-R"));

    // 合同方同序（同交易双账本）：主 P1=A / P2=B；镜像同序
    let main_parties: Vec<(String, Option<i64>)> = sqlx::query_as(
        r#"SELECT code, ref_right FROM isahl."zc_id_contract_rr_party"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY code"#,
    )
    .bind(main_id)
    .fetch_all(&pool)
    .await
    .expect("main parties");
    assert_eq!(
        main_parties,
        vec![("P1".into(), Some(a)), ("P2".into(), Some(b))]
    );

    let mirror_parties: Vec<Option<i64>> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl."zc_id_contract_rr_party"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY code"#,
    )
    .bind(mirror_id)
    .fetch_all(&pool)
    .await
    .expect("mirror parties");
    assert_eq!(
        mirror_parties,
        vec![Some(a), Some(b)],
        "同交易双账本：镜像合同方不互换"
    );

    // MIR 桥（方向 = 叶子语义：销售正本属供给性 → ref_right；诉求镜像属需求性 → ref_left）
    let bridge: Vec<(i64, i64, String)> = sqlx::query_as(
        r#"SELECT ref_left, ref_right, code FROM isahl."zc_id_contract_rr_symmetry"
           WHERE (ref_left = $1 OR ref_right = $1) AND deleted_at IS NULL"#,
    )
    .bind(main_id)
    .fetch_all(&pool)
    .await
    .expect("bridge");
    assert_eq!(
        bridge,
        vec![(mirror_id, main_id, format!("MIR-{main_id}-{mirror_id}"))]
    );

    // 行级 NGAC：主 + 镜像合同行各 1 条对象属性（写件内单源注册，2026-09-14 裁决）
    let ngac_rows: Vec<i64> = sqlx::query_scalar(
        r#"SELECT fk_resource FROM isahl_auth.ngac_object_attribute
           WHERE resource_type = 'contracts' AND fk_resource = ANY($1) AND deleted_at IS NULL
           ORDER BY fk_resource"#,
    )
    .bind(vec![main_id, mirror_id])
    .fetch_all(&pool)
    .await
    .expect("contract row ngac");
    let mut expect_ngac = vec![main_id, mirror_id];
    expect_ngac.sort_unstable();
    assert_eq!(ngac_rows, expect_ngac, "主/镜像合同行均须有行级 NGAC");

    // 镜像草稿状态桥（字典在场时）
    if draft.is_some() {
        let status_rows: i64 = sqlx::query_scalar(
            r#"SELECT count(*) FROM isahl."zc_id_lifecycle_r_primary-status"
               WHERE ref_left = $1 AND deleted_at IS NULL"#,
        )
        .bind(mirror_id)
        .fetch_one(&pool)
        .await
        .expect("mirror status");
        assert_eq!(status_rows, 1);
    }

    drop(conn);
    purge(&pool, "T-CW-PAIR").await;
}

#[tokio::test]
async fn product_rows_land_both_families() {
    let pool = connect_test_db().await;
    let code = unique_code("PRD");
    let a = ensure_subject(&pool, "T-CW-SUBJ-A").await;
    let b = ensure_subject(&pool, "T-CW-SUBJ-B").await;
    let mut conn = pool.acquire().await.expect("acquire");

    let sales_id = insert_product_row_tx(&mut conn, &product_input(&code, true, a, b))
        .await
        .expect("sales product");
    let purchase_id =
        insert_product_row_tx(&mut conn, &product_input(&format!("{code}-R"), false, a, b))
            .await
            .expect("purchase product");

    let sales_leaf: (Option<String>, i64, i64) = sqlx::query_as(
        r#"SELECT "_f_", "fk_subj-demand", "fk_subj-provider"
           FROM isahl."zc_id_prod-freight_road-sales" WHERE id = $1"#,
    )
    .bind(sales_id)
    .fetch_one(&pool)
    .await
    .expect("sales leaf");
    assert_eq!(sales_leaf, (Some("实现".into()), a, b));

    let purchase_leaf: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_prod-freight_road-purchase" WHERE id = $1"#,
    )
    .bind(purchase_id)
    .fetch_one(&pool)
    .await
    .expect("purchase leaf");
    assert_eq!(purchase_leaf, purchase_id);

    drop(conn);
    purge(&pool, "T-CW-PRD").await;
}

#[tokio::test]
async fn mirror_of_existing_contract_and_cascade_delete() {
    let pool = connect_test_db().await;
    let code = unique_code("MIR");
    let a = ensure_subject(&pool, "T-CW-SUBJ-A").await;
    let b = ensure_subject(&pool, "T-CW-SUBJ-B").await;
    let mut conn = pool.acquire().await.expect("acquire");

    // 主合约（采购向）
    let parties = vec![
        ContractParty {
            subject_id: Some(a),
            name: "甲方".into(),
            period_id: None,
        },
        ContractParty {
            subject_id: Some(b),
            name: "乙方".into(),
            period_id: None,
        },
    ];
    let main = row_input(&code, ContractLeaf::Purchase, parties, None);
    let main_id = contract_writer::insert_contract_row_tx(&mut conn, &main)
        .await
        .expect("main row");

    // 镜像（销售向，甲乙互换，编号 -R）
    let mirror_code = format!("{code}-R");
    let mirror = row_input(
        &mirror_code,
        ContractLeaf::Sales,
        vec![
            ContractParty {
                subject_id: Some(b),
                name: "甲方".into(),
                period_id: None,
            },
            ContractParty {
                subject_id: Some(a),
                name: "乙方".into(),
                period_id: None,
            },
        ],
        None,
    );
    let mirror_id = insert_mirror_of_contract_tx(&mut conn, main_id, &mirror)
        .await
        .expect("mirror row");

    let resolved = resolve_mirror_ids_tx(&mut conn, main_id)
        .await
        .expect("resolve");
    assert_eq!(resolved, vec![mirror_id]);

    // 镜像产品（走 MIR 桥识别 → 挂 matter 桥；此处直接经 matter 桥登记以覆盖级联路径）
    let product_id = insert_product_row_tx(
        &mut conn,
        &product_input(&format!("PRD-{mirror_code}"), true, a, b),
    )
    .await
    .expect("mirror product");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contract_rr_deal" (id, ref_left, ref_right, notice, created_by_id)
           VALUES (isahl.gen_next_uid(670), $1, $2, '级联测试', 1)"#,
    )
    .bind(mirror_id)
    .bind(product_id)
    .execute(&mut *conn)
    .await
    .expect("matter bridge");

    soft_delete_mirror_tx(&mut conn, mirror_id, 1)
        .await
        .expect("cascade delete");

    let live: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM isahl."zc_id_contract" WHERE id = $1 AND deleted_at IS NULL),
             (SELECT count(*) FROM isahl."zc_id_prod-freight_road-sales" WHERE id = $2 AND deleted_at IS NULL),
             (SELECT count(*) FROM isahl."zc_id_contract_rr_symmetry" WHERE ref_right = $1 AND deleted_at IS NULL),
             (SELECT count(*) FROM isahl."zc_id_contract_rr_party" WHERE ref_left = $1 AND deleted_at IS NULL)"#,
    )
    .bind(mirror_id)
    .bind(product_id)
    .fetch_one(&pool)
    .await
    .expect("cascade probe");
    assert_eq!(live, (0, 0, 0, 0), "镜像合约/产品/桥/合同方须全部软删");
    // 主合约不受影响
    let main_live: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM isahl."zc_id_contract" WHERE id = $1 AND deleted_at IS NULL"#,
    )
    .bind(main_id)
    .fetch_one(&pool)
    .await
    .expect("main alive");
    assert_eq!(main_live, 1);

    drop(conn);
    purge(&pool, "T-CW-MIR").await;
}

#[tokio::test]
async fn duplicate_code_conflicts_and_invalid_fn_code_rejected() {
    let pool = connect_test_db().await;
    let code = unique_code("DUP");
    let a = ensure_subject(&pool, "T-CW-SUBJ-A").await;
    let b = ensure_subject(&pool, "T-CW-SUBJ-B").await;
    let mut conn = pool.acquire().await.expect("acquire");

    let parties = || {
        vec![
            ContractParty {
                subject_id: Some(a),
                name: "甲方".into(),
                period_id: None,
            },
            ContractParty {
                subject_id: Some(b),
                name: "乙方".into(),
                period_id: None,
            },
        ]
    };
    insert_contract_pair_tx(
        &mut conn,
        &row_input(&code, ContractLeaf::Sales, parties(), None),
    )
    .await
    .expect("first pair");

    // 同编号重放 → 冲突（且不产生镜像）
    let err = insert_contract_pair_tx(
        &mut conn,
        &row_input(&code, ContractLeaf::Sales, parties(), None),
    )
    .await
    .expect_err("duplicate must conflict");
    assert!(
        matches!(err, common::AliothError::Conflict(_)),
        "冲突错误类型"
    );

    // 非法职能码 → 校验错误
    let bad_code = format!("{code}-BAD");
    let mut bad = row_input(&bad_code, ContractLeaf::Sales, parties(), None);
    bad.fn_code = "XX.XX";
    let err = insert_contract_row_tx(&mut conn, &bad)
        .await
        .expect_err("invalid fn_code");
    assert!(
        matches!(err, common::AliothError::Validation { .. }),
        "校验错误类型"
    );

    drop(conn);
    purge(&pool, "T-CW-DUP").await;
}

/// 产品对：主族 + 相反族 + `{code}-R`，**镜像不互换买卖主体对**（同交易双账本，2026-09-17），各自 `fk_previous` 挂各自单据。
#[tokio::test]
async fn product_pair_lands_opposite_families() {
    let pool = connect_test_db().await;
    let code = unique_code("PAIRP");
    let a = ensure_subject(&pool, "T-CW-SUBJ-A").await;
    let b = ensure_subject(&pool, "T-CW-SUBJ-B").await;
    let mut conn = pool.acquire().await.expect("acquire");

    let (main_id, mirror_id) = insert_product_pair_tx(
        &mut conn,
        &ProductPairInput {
            is_sales: true,
            code: &code,
            notice: "主产品",
            comments: "主注释",
            mirror_notice: None,
            mirror_comments: None,
            demand_subject: a,
            provider_subject: b,
            line_id: None,
            vehicle_form_id: None,
            price_id: None,
            weight_id: None,
            period_id: None,
            previous_main: Some(7),
            previous_mirror: Some(8),
            fn_code: "↓.GG",
            scene_code: SCENE,
            factor_code: FACTOR,
            user_id: 1,
        },
    )
    .await
    .expect("product pair");

    let main_row: (String, Option<i64>) = sqlx::query_as(
        r#"SELECT code, fk_previous FROM isahl."zc_id_prod-freight_road-sales" WHERE id = $1"#,
    )
    .bind(main_id)
    .fetch_one(&pool)
    .await
    .expect("main product");
    assert_eq!(main_row, (code.clone(), Some(7)));

    let mirror_row: (String, i64, Option<i64>) = sqlx::query_as(
        r#"SELECT code, "fk_subj-demand", fk_previous
           FROM isahl."zc_id_prod-freight_road-purchase" WHERE id = $1"#,
    )
    .bind(mirror_id)
    .fetch_one(&pool)
    .await
    .expect("mirror product");
    assert_eq!(
        mirror_row,
        (format!("{code}-R"), a, Some(8)),
        "同交易双账本：镜像产品 demand 与正本一致"
    );

    drop(conn);
    purge(&pool, "T-CW-PAIRP").await;
}
/// 对称桥方向 = 叶子语义固化（用户裁决 2026-09-17「创建时逻辑恒为 需求侧→供给侧」
/// 「先有语义判定再有位置判定」）：销售先建对 → `ref_left` = 采购叶镜像（需求性）、
/// `ref_right` = 销售叶正本（供给性）；采购先建对 → 原序即语义序；
/// code 恒 `MIR-{正本}-{镜像}`；对端解析双向（任一端取另一端）。
#[tokio::test]
async fn symmetry_bridge_direction_is_semantic() {
    let pool = connect_test_db().await;
    let a = ensure_subject(&pool, "T-CW-SUBJ-A").await;
    let b = ensure_subject(&pool, "T-CW-SUBJ-B").await;
    let mut conn = pool.acquire().await.expect("acquire");
    let parties = vec![
        ContractParty {
            subject_id: Some(a),
            name: "甲方".into(),
            period_id: None,
        },
        ContractParty {
            subject_id: Some(b),
            name: "乙方".into(),
            period_id: None,
        },
    ];

    // ① 销售先建：正本 = 销售叶（供给性）→ 桥位翻转，code 保持正本序
    let code_s = unique_code("SYMS");
    let (main_s, mirror_p) = insert_contract_pair_tx(
        &mut conn,
        &row_input(&code_s, ContractLeaf::Sales, parties.clone(), None),
    )
    .await
    .expect("pair sales-first");
    let (l, r, mir_code): (i64, i64, String) = sqlx::query_as(
        r#"SELECT ref_left, ref_right, code FROM isahl."zc_id_contract_rr_symmetry"
           WHERE (ref_left = $1 OR ref_right = $1) AND code LIKE 'MIR-%' AND deleted_at IS NULL"#,
    )
    .bind(main_s)
    .fetch_one(&pool)
    .await
    .expect("mir row sales-first");
    assert_eq!(
        (l, r),
        (mirror_p, main_s),
        "销售先建 → ref_left=采购镜像（需求性）/ ref_right=销售正本（供给性）"
    );
    assert_eq!(
        mir_code,
        format!("MIR-{main_s}-{mirror_p}"),
        "code 恒正本序"
    );

    // ② 采购先建：正本 = 采购叶（需求性）→ 桥位即语义序
    let code_p = unique_code("SYMP");
    let (main_p, mirror_s) = insert_contract_pair_tx(
        &mut conn,
        &row_input(&code_p, ContractLeaf::Purchase, parties, None),
    )
    .await
    .expect("pair purchase-first");
    let (l2, r2): (i64, i64) = sqlx::query_as(
        r#"SELECT ref_left, ref_right FROM isahl."zc_id_contract_rr_symmetry"
           WHERE (ref_left = $1 OR ref_right = $1) AND code LIKE 'MIR-%' AND deleted_at IS NULL"#,
    )
    .bind(main_p)
    .fetch_one(&pool)
    .await
    .expect("mir row purchase-first");
    assert_eq!(
        (l2, r2),
        (main_p, mirror_s),
        "采购先建 → ref_left=采购正本（需求性）/ ref_right=销售镜像（供给性）"
    );

    // ③ 对端解析双向：从镜像侧反查正本
    let rev = resolve_mirror_ids_tx(&mut conn, mirror_p)
        .await
        .expect("reverse resolve");
    assert_eq!(rev, vec![main_s], "镜像侧反查 MUST 命中正本");
}

/// 合同驱动产品组装：产品行 + 起讫桥 ×2 + 合同桥（销售 master → rr_goods）。
#[tokio::test]
async fn contract_product_assembly_writes_stops_and_bridge() {
    let pool = connect_test_db().await;
    let code = unique_code("ASM");
    let a = ensure_subject(&pool, "T-CW-SUBJ-A").await;
    let b = ensure_subject(&pool, "T-CW-SUBJ-B").await;
    let mut conn = pool.acquire().await.expect("acquire");

    let parties = vec![
        ContractParty {
            subject_id: Some(a),
            name: "甲方".into(),
            period_id: None,
        },
        ContractParty {
            subject_id: Some(b),
            name: "乙方".into(),
            period_id: None,
        },
    ];
    let (main_id, _mirror_id) = insert_contract_pair_tx(
        &mut conn,
        &row_input(&code, ContractLeaf::Sales, parties, None),
    )
    .await
    .expect("contract pair");

    let product_id = create_contract_transport_product_tx(
        &mut conn,
        &ContractProductInput {
            contract_id: main_id,
            is_sales: true,
            is_order: false,
            contract_code: &code,
            notice: "合同运输服务产品",
            comments: "自动化测试",
            demand_subject: a,
            provider_subject: b,
            line_id: 1,
            vehicle_form_id: Some(1),
            origin_place_id: 1,
            dest_place_id: 2,
            price_id: None,
            weight_id: None,
            period_id: None,
            valid_segm_id: None,
            fn_code: "↓.GG",
            scene_code: "GC",
            factor_code: FACTOR,
            user_id: 1,
        },
    )
    .await
    .expect("contract product");

    let product_code: String = sqlx::query_scalar(
        r#"SELECT code FROM isahl."zc_id_prod-freight_road-sales" WHERE id = $1"#,
    )
    .bind(product_id)
    .fetch_one(&pool)
    .await
    .expect("product code");
    assert_eq!(product_code, format!("PRD-{code}"));

    let stops: Vec<String> = sqlx::query_scalar(
        r#"SELECT code FROM isahl."zc_id_prod-transport_rr_stop"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY code"#,
    )
    .bind(product_id)
    .fetch_all(&pool)
    .await
    .expect("stops");
    assert_eq!(
        stops,
        vec![format!("STOP-PRD-{code}-A"), format!("STOP-PRD-{code}-D"),]
    );

    let bridge: (i64, i64) = sqlx::query_as(
        r#"SELECT ref_left, ref_right FROM isahl."zc_id_contract_rr_goods"
           WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(main_id)
    .fetch_one(&pool)
    .await
    .expect("bridge");
    assert_eq!(bridge, (main_id, product_id));

    drop(conn);
    purge(&pool, "T-CW-ASM").await;
}

/// 主合约级联软删：产品（含起讫桥）+ 明细/合同方/状态桥 + 合约行；镜像不受影响。
#[tokio::test]
async fn soft_delete_contract_cascades_main_side() {
    let pool = connect_test_db().await;
    let code = unique_code("CASC");
    let a = ensure_subject(&pool, "T-CW-SUBJ-A").await;
    let b = ensure_subject(&pool, "T-CW-SUBJ-B").await;
    let draft = draft_status_id(&pool).await;
    let mut conn = pool.acquire().await.expect("acquire");

    let parties = vec![
        ContractParty {
            subject_id: Some(a),
            name: "甲方".into(),
            period_id: None,
        },
        ContractParty {
            subject_id: Some(b),
            name: "乙方".into(),
            period_id: None,
        },
    ];
    let (main_id, mirror_id) = insert_contract_pair_tx(
        &mut conn,
        &row_input(&code, ContractLeaf::Sales, parties, draft),
    )
    .await
    .expect("contract pair");

    let product_id = insert_product_row_tx(
        &mut conn,
        &product_input(&format!("PRD-{code}"), true, a, b),
    )
    .await
    .expect("product");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_contract_rr_deal" (id, ref_left, ref_right, notice, created_by_id)
           VALUES (isahl.gen_next_uid(670), $1, $2, '级联测试', 1)"#,
    )
    .bind(main_id)
    .bind(product_id)
    .execute(&mut *conn)
    .await
    .expect("matter");

    soft_delete_contract_tx(&mut conn, main_id, 1)
        .await
        .expect("cascade main");

    let live: (i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM isahl."zc_id_contract" WHERE id = $1 AND deleted_at IS NULL),
             (SELECT count(*) FROM isahl."zc_id_prod-freight_road-sales" WHERE id = $2 AND deleted_at IS NULL),
             (SELECT count(*) FROM isahl."zc_id_contract_rr_deal" WHERE ref_left = $1 AND deleted_at IS NULL),
             (SELECT count(*) FROM isahl."zc_id_contract_rr_party" WHERE ref_left = $1 AND deleted_at IS NULL)"#,
    )
    .bind(main_id)
    .bind(product_id)
    .fetch_one(&pool)
    .await
    .expect("cascade probe");
    assert_eq!(live, (0, 0, 0, 0), "主合约及其产物须全部软删");

    // 镜像不受影响（其桥仍在）
    let mirror_live: (i64, i64) = sqlx::query_as(
        r#"SELECT
             (SELECT count(*) FROM isahl."zc_id_contract" WHERE id = $1 AND deleted_at IS NULL),
             (SELECT count(*) FROM isahl."zc_id_contract_rr_symmetry"
              WHERE (ref_left = $1 OR ref_right = $1) AND deleted_at IS NULL)"#,
    )
    .bind(mirror_id)
    .fetch_one(&pool)
    .await
    .expect("mirror probe");
    assert_eq!(mirror_live, (1, 1), "镜像合约与其桥不受主侧级联影响");

    drop(conn);
    purge(&pool, "T-CW-CASC").await;
}

/// 客户诉求 ↔ 我方销售（用户裁决 2026-09-17「除了采购，客户诉求→我方销售」）：
/// 诉求叶（需求性，`ref_left`）成对的镜像落**销售叶**（供给性，`ref_right`）——
/// 两叶「我」同在 P2 → 合同方**不互换**（两份都是 甲=客户 / 乙=我）；编号 `-R`；MIR 桥一行。
#[tokio::test]
async fn request_leaf_pairs_with_sales() {
    let pool = connect_test_db().await;
    assert_request_leaf(&pool).await;
    let code = unique_code("REQ");
    let a = ensure_subject(&pool, "T-CW-SUBJ-A").await;
    let b = ensure_subject(&pool, "T-CW-SUBJ-B").await;
    let mut conn = pool.acquire().await.expect("acquire");

    let parties = vec![
        ContractParty {
            subject_id: Some(a),
            name: "甲方".into(),
            period_id: None,
        },
        ContractParty {
            subject_id: Some(b),
            name: "乙方".into(),
            period_id: None,
        },
    ];
    let (main_id, mirror_id) = insert_contract_pair_tx(
        &mut conn,
        &row_input(&code, ContractLeaf::Request, parties, None),
    )
    .await
    .expect("request pair");

    // 主落诉求叶（需求侧记录）、镜像落销售叶（供给侧记录，我方销售）
    assert_eq!(ContractLeaf::Request.table(), "zc_id_cont-request");
    let main_row: (i64, String) =
        sqlx::query_as(r#"SELECT id, code FROM isahl."zc_id_cont-request" WHERE id = $1"#)
            .bind(main_id)
            .fetch_one(&pool)
            .await
            .expect("request leaf main row");
    assert_eq!(main_row, (main_id, code.clone()));
    let mirror_row: (i64, String) =
        sqlx::query_as(r#"SELECT id, code FROM isahl."zc_id_cont-sales" WHERE id = $1"#)
            .bind(mirror_id)
            .fetch_one(&pool)
            .await
            .expect("sales leaf mirror row");
    assert_eq!(
        mirror_row,
        (mirror_id, format!("{code}-R")),
        "诉求的镜像 MUST 落销售叶（我方销售），编号 = 主编号 -R"
    );

    // 合同方不互换：主 P1=A / P2=B；镜像同序（诉求↔销售「我」同在 P2）
    let main_parties: Vec<Option<i64>> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl."zc_id_contract_rr_party"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY code"#,
    )
    .bind(main_id)
    .fetch_all(&pool)
    .await
    .expect("main parties");
    assert_eq!(main_parties, vec![Some(a), Some(b)]);

    let mirror_parties: Vec<Option<i64>> = sqlx::query_scalar(
        r#"SELECT ref_right FROM isahl."zc_id_contract_rr_party"
           WHERE ref_left = $1 AND deleted_at IS NULL ORDER BY code"#,
    )
    .bind(mirror_id)
    .fetch_all(&pool)
    .await
    .expect("mirror parties");
    assert_eq!(
        mirror_parties,
        vec![Some(a), Some(b)],
        "诉求↔销售同槽配对：合同方 MUST 不互换（两份都是 甲=客户 / 乙=我）"
    );

    // MIR 桥一行：诉求（需求性）→ ref_left、销售（供给性）→ ref_right；code 正本序
    let bridge: Vec<(i64, i64, String)> = sqlx::query_as(
        r#"SELECT ref_left, ref_right, code FROM isahl."zc_id_contract_rr_symmetry"
           WHERE (ref_left = $1 OR ref_right = $1) AND deleted_at IS NULL"#,
    )
    .bind(main_id)
    .fetch_all(&pool)
    .await
    .expect("mirror bridge");
    assert_eq!(
        bridge,
        vec![(main_id, mirror_id, format!("MIR-{main_id}-{mirror_id}"))]
    );

    drop(conn);
    purge(&pool, "T-CW-REQ").await;
}
