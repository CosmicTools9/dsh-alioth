//! 审批岗位（Approver）读径类谓词回归（fix-position-class-predicate）
//!
//! 审批岗位下拉的数据源 = `authority` 的 `zc_id_subj-position` 读径。本文件用
//! **生产仓储入口** `ApproverRepository::list_with_rls` 断言可观测结果：
//! - 类派生岗位行（`_f_='实现' / _t_='实例'`）与类列 NULL 的存量行 MUST 返回；
//! - 编制范例行（`_f_='设计' / _t_='范例'`）MUST NOT 返回。
//!
//! 修复前判据为 `_f_ IS NULL` ⇒ 类派生行全被排除（2026-09-23 事故：AVIC 审批节点
//! 「审批岗位」下拉恒空）。

use ::common::data::ListQuery;
use ::common::testing::connect_test_db;
use authority::repositories::ApproverRepository;
use sqlx::PgPool;

/// 动态测试 id 段（进程+纳秒派生，跨运行不冲突；测试不清理数据）
fn tid(base: i64) -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    ((nanos % 1_000_000) as i64) * 100 + base
}

async fn insert_position(pool: &PgPool, id: i64, notice: &str, class: Option<(&str, &str)>) -> i64 {
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve position coords");
    let (f_, t_) = match class {
        Some((f_, t_)) => (Some(f_), Some(t_)),
        None => (None, None),
    };
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position"
               (id, notice, code, _f_, _t_, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(notice)
    .bind(format!("T-APV-{id}"))
    .bind(f_)
    .bind(t_)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(pool)
    .await
    .expect("insert position");
    id
}

#[tokio::test]
async fn approver_list_returns_class_derived_positions_only() {
    let pool = connect_test_db().await;

    let base = tid(7200);
    let instance = insert_position(&pool, base, "T-审批岗位-类派生", Some(("实现", "实例"))).await;
    let template = insert_position(
        &pool,
        base + 1,
        "T-审批岗位-编制范例",
        Some(("设计", "范例")),
    )
    .await;
    let legacy = insert_position(&pool, base + 2, "T-审批岗位-存量", None).await;
    let ids: Vec<i64> = vec![instance, template, legacy];

    let repo = ApproverRepository::new(pool.clone());
    let page = repo
        .list_with_rls(&ListQuery::default(), Some(&ids))
        .await
        .expect("list approvers");

    let names: Vec<&str> = page.items.iter().map(|a| a.name.as_str()).collect();
    assert!(
        names.contains(&"T-审批岗位-类派生") && names.contains(&"T-审批岗位-存量"),
        "类派生岗位行与类列 NULL 的存量行 MUST 出现在审批岗位候选（修复前为空），实测: {names:?}"
    );
    assert!(
        !names.contains(&"T-审批岗位-编制范例"),
        "编制范例行 MUST NOT 出现在审批岗位候选，实测: {names:?}"
    );
}

/// 仅经任职桥挂接的活跃账号 MUST 出行（修复前只认标量 `fk_user`，组织管理挂人的岗位在审批候选里整条缺失）
#[tokio::test]
async fn position_options_expose_bridge_derived_incumbents() {
    let pool = connect_test_db().await;

    let base = tid(7300);
    let (account, person, pos_bridged, pos_bare) = (base, base + 1, base + 2, base + 3);
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .expect("resolve dk coords");

    // 活跃账号 + 任职自然人 + 岗位（标量空）+ 任职桥
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, $2, $2, $2 || '@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(account)
    .bind(format!("t-opt-{base}"))
    .execute(&pool)
    .await
    .expect("seed account");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pos_bridged)
    .bind(format!("T-OPT-BRIDGED-{base}"))
    .bind(format!("T-OPTC-BRIDGED-{base}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("seed position");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, fk_user, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(person)
    .bind(format!("T-OPT-EMP-{base}"))
    .bind(account)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("seed natural person");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-post_rr_employee" (ref_left, ref_right, notice)
           VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"#,
    )
    .bind(pos_bridged)
    .bind(person)
    .bind(format!("T-OPT-BRIDGE-{base}"))
    .execute(&pool)
    .await
    .expect("seed employment bridge");
    // 对照组：无任何任职账号
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pos_bare)
    .bind(format!("T-OPT-BARE-{base}"))
    .bind(format!("T-OPTC-BARE-{base}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("seed bare position");

    let opts = authority::repositories::list_position_options(&pool)
        .await
        .expect("list position options");

    let bridged: Vec<&authority::repositories::PositionOption> =
        opts.iter().filter(|o| o.id == pos_bridged).collect();
    let bridged = bridged
        .first()
        .expect("仅桥任职岗位 MUST 出现在 /positions 响应");
    assert_eq!(
        bridged.fk_user,
        Some(account),
        "行 fk_user MUST 为该岗位任职桥派生的账号（修复前该岗位整条缺失）"
    );

    assert!(
        !opts.iter().any(|o| o.id == pos_bare),
        "无活跃任职账号的岗位 MUST 不出现在 /positions 响应（无可解析审批人）"
    );
}

/// 多任职账号岗位 → 每账号一行（`ApproverSelPair` 按岗位名去重成「岗位 + 账号集合」）
#[tokio::test]
async fn position_options_emit_one_row_per_incumbent_account() {
    let pool = connect_test_db().await;

    let base = tid(7400);
    let (acc_a, acc_b, person_a, person_b, pos) = (base, base + 1, base + 2, base + 3, base + 4);
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .expect("resolve dk coords");

    for (id, tag) in [(acc_a, "a"), (acc_b, "b")] {
        sqlx::query(
            r#"INSERT INTO isahl_auth.auth_users
               (id, name, username, email, user_type, is_active, created_at, updated_at,
                failed_login_attempts, notification_preferences)
               VALUES ($1, $2, $2, $2 || '@test.local', 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(id)
        .bind(format!("t-multi-{base}-{tag}"))
        .execute(&pool)
        .await
        .expect("seed account");
    }
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pos)
    .bind(format!("T-MULTI-{base}"))
    .bind(format!("T-MULTIC-{base}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("seed position");
    for (person, account) in [(person_a, acc_a), (person_b, acc_b)] {
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, fk_user, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(person)
        .bind(format!("T-MULTI-EMP-{person}"))
        .bind(account)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .execute(&pool)
        .await
        .expect("seed natural person");
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_subj-post_rr_employee" (ref_left, ref_right, notice)
               VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"#,
        )
        .bind(pos)
        .bind(person)
        .bind(format!("T-MULTI-BRIDGE-{person}"))
        .execute(&pool)
        .await
        .expect("seed employment bridge");
    }

    let opts = authority::repositories::list_position_options(&pool)
        .await
        .expect("list position options");
    let rows: Vec<_> = opts.iter().filter(|o| o.id == pos).collect();
    let mut accounts: Vec<Option<i64>> = rows.iter().map(|r| r.fk_user).collect();
    accounts.sort();
    assert_eq!(
        accounts,
        vec![Some(acc_a), Some(acc_b)],
        "多任职账号岗位 MUST 每账号一行（前端按岗位名去重后成「岗位 + 账号集合」）"
    );
    assert!(
        rows.iter().all(|r| r.name == format!("T-MULTI-{base}")),
        "同一岗位各行 MUST 携带同一岗位名（前端按名去重的依据）"
    );
}
