//! identity-org 组织管理集成测试（consolidate-org-management-identity-org 5.1）
//!
//! 覆盖：position 双桥全量替换 / parent_id 环检测 / 组织树挂接与环拒绝 /
//! 任职主体路由（empl-natural / empl-agent）/ group 成员桥。
//! 直接执行与 org_tree.rs handler 相同的 SQL 语义（handler DTO 字段私有，
//! 外部 crate 不可构造；SQL 即 handler 行为，测试防回归漂移）。
//!
//! 依赖：test 库存在 isahl.zc_id_subj-position / zc_id_subj-org_rr_position /
//! zc_id_subj-post_rr_subordinate / zc_id_subj-org_rr_subordinate /
//! zc_id_subj-post_rr_employee / zc_id_subj-org_rr_employee /
//! zc_id_subj-group_rr_member / zc_id_subj-group / zc_id_orga-department /
//! zc_id_orga-non-banking-legal / zc_id_empl-natural / zc_id_empl-agent。

use sqlx::PgPool;

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://isahl@localhost:5432/aliothstudio_test".to_string());
    let pool = PgPool::connect(&url).await.expect("connect test db");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .expect("current_database");
    assert!(db.contains("_test"), "REFUSED: non-test db {db}");
    pool
}

/// 动态测试 id 段（进程+纳秒派生，跨运行不冲突；测试不清理数据）
fn tid(base: i64) -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    ((nanos % 1_000_000) as i64) * 100 + base
}

async fn ensure_org(pool: &PgPool, id: i64, notice: &str) {
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_orga-department" (id, notice, code)
           VALUES ($1, $2, $3) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(notice)
    .bind(format!("T-ORG-{id}"))
    .execute(pool)
    .await
    .expect("ensure org");
}

async fn ensure_position(pool: &PgPool, id: i64, notice: &str, parent_id: Option<i64>) {
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, fk_parent)
           VALUES ($1, $2, $3, $4) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(notice)
    .bind(format!("T-POS-{id}"))
    .bind(parent_id)
    .execute(pool)
    .await
    .expect("ensure position");
}

/// 桥行计数（未删除）
async fn bridge_count(pool: &PgPool, table: &str, left: i64, right: i64) -> i64 {
    let sql = format!(
        r#"SELECT COUNT(*) FROM isahl."{}" WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
        table
    );
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(left)
        .bind(right)
        .fetch_one(pool)
        .await
        .expect("bridge count")
}

/// 双桥全量替换：软删旧关联 + 插新关联（write_position_bridges 同语义）
#[tokio::test]
async fn position_bridges_replace_soft_deleted_old() {
    let pool = test_pool().await;
    let pos = tid(1);
    let org_a = tid(11);
    let org_b = tid(12);
    let sub_a = tid(21);
    let sub_b = tid(22);
    ensure_position(&pool, pos, "双桥测试岗位", None).await;
    ensure_org(&pool, org_a, "双桥组织A").await;
    ensure_org(&pool, org_b, "双桥组织B").await;
    ensure_org(&pool, sub_a, "双桥下辖A").await;
    ensure_org(&pool, sub_b, "双桥下辖B").await;

    // 事务内：软删旧桥 + 插新桥（org_rr_position ×2、post_rr_subordinate ×2）
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-org_rr_position" (ref_left, ref_right)
           VALUES ($1, $2), ($3, $2)"#,
    )
    .bind(org_a)
    .bind(pos)
    .bind(org_b)
    .execute(&mut *tx)
    .await
    .expect("insert old org bridge");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-post_rr_subordinate" (ref_left, ref_right)
           VALUES ($1, $2), ($1, $3)"#,
    )
    .bind(pos)
    .bind(sub_a)
    .bind(sub_b)
    .execute(&mut *tx)
    .await
    .expect("insert old sub bridge");
    tx.commit().await.expect("commit");

    // 全量替换：只保留 org_a + sub_a
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_position"
           SET deleted_at = now() WHERE ref_right = $1 AND deleted_at IS NULL"#,
    )
    .bind(pos)
    .execute(&mut *tx)
    .await
    .expect("soft delete org bridge");
    // 复活同键软删行（唯一约束含 qk_period 表达式无法 ON CONFLICT 推断）+ 幂等插入
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_position" SET deleted_at = NULL, deleted_by_id = NULL
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
    )
    .bind(org_a)
    .bind(pos)
    .execute(&mut *tx)
    .await
    .expect("revive org bridge");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-org_rr_position" (ref_left, ref_right) VALUES ($1, $2)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(org_a)
    .bind(pos)
    .execute(&mut *tx)
    .await
    .expect("insert new org bridge");
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-post_rr_subordinate"
           SET deleted_at = now() WHERE ref_left = $1 AND deleted_at IS NULL"#,
    )
    .bind(pos)
    .execute(&mut *tx)
    .await
    .expect("soft delete sub bridge");
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-post_rr_subordinate" SET deleted_at = NULL, deleted_by_id = NULL
           WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
    )
    .bind(pos)
    .bind(sub_a)
    .execute(&mut *tx)
    .await
    .expect("revive sub bridge");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-post_rr_subordinate" (ref_left, ref_right) VALUES ($1, $2)
           ON CONFLICT DO NOTHING"#,
    )
    .bind(pos)
    .bind(sub_a)
    .execute(&mut *tx)
    .await
    .expect("insert new sub bridge");
    tx.commit().await.expect("commit");

    assert_eq!(
        bridge_count(&pool, "zc_id_subj-org_rr_position", org_a, pos).await,
        1
    );
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-org_rr_position", org_b, pos).await,
        0,
        "旧 org 桥应软删"
    );
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-post_rr_subordinate", pos, sub_a).await,
        1
    );
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-post_rr_subordinate", pos, sub_b).await,
        0,
        "旧 sub 桥应软删"
    );
}

/// fk_parent 环检测（check_parent_cycle 同 CTE）：A→B、B→A 时以 B 为新上级查 A 命中
#[tokio::test]
async fn position_parent_cycle_rejected() {
    let pool = test_pool().await;
    let a = tid(31);
    let b = tid(32);
    ensure_position(&pool, a, "环岗位A", Some(b)).await;
    ensure_position(&pool, b, "环岗位B", Some(a)).await;

    // CTE：anc 从新上级 B 上溯（B→A→B 去重），查是否含当前岗位 A
    let cycle: Option<i32> = sqlx::query_scalar(
        r#"WITH RECURSIVE anc AS (
            SELECT id, fk_parent FROM isahl."zc_id_subj-position" WHERE id = $1
            UNION
            SELECT p.id, p.fk_parent FROM isahl."zc_id_subj-position" p JOIN anc a ON a.fk_parent = p.id
        ) SELECT 1 FROM anc WHERE id = $2"#,
    )
    .bind(b)
    .bind(a)
    .fetch_optional(&pool)
    .await
    .expect("cycle check");
    assert!(cycle.is_some(), "A 的祖先链（经 B）应含 A 自身 → 成环");

    // 非环对照：C 无父，B 以 C 为新上级不命中
    let c = tid(33);
    ensure_position(&pool, c, "环岗位C", None).await;
    let cycle2: Option<i32> = sqlx::query_scalar(
        r#"WITH RECURSIVE anc AS (
            SELECT id, fk_parent FROM isahl."zc_id_subj-position" WHERE id = $1
            UNION
            SELECT p.id, p.fk_parent FROM isahl."zc_id_subj-position" p JOIN anc a ON a.fk_parent = p.id
        ) SELECT 1 FROM anc WHERE id = $2"#,
    )
    .bind(c)
    .bind(b)
    .fetch_optional(&pool)
    .await
    .expect("cycle check 2");
    assert!(cycle2.is_none(), "B 的祖先链（经 C）不应含 B");
}

/// 组织树：挂接 + 环拒绝（check_org_tree_cycle 同 around CTE）+ 子树下钻
#[tokio::test]
async fn org_tree_attach_detach_and_cycle() {
    let pool = test_pool().await;
    let root = tid(41);
    let child = tid(42);
    let grand = tid(43);
    ensure_org(&pool, root, "树根").await;
    ensure_org(&pool, child, "树子").await;
    ensure_org(&pool, grand, "树孙").await;

    // 挂接 root→child、child→grand
    for (l, r) in [(root, child), (child, grand)] {
        sqlx::query(
            r#"UPDATE isahl."zc_id_subj-org_rr_subordinate" SET deleted_at = NULL, deleted_by_id = NULL
               WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NOT NULL"#,
        )
        .bind(l)
        .bind(r)
        .execute(&pool)
        .await
        .expect("revive attach");
        sqlx::query(
            r#"INSERT INTO isahl."zc_id_subj-org_rr_subordinate" (ref_left, ref_right)
               VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
        )
        .bind(l)
        .bind(r)
        .execute(&pool)
        .await
        .expect("attach");
    }
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-org_rr_subordinate", root, child).await,
        1
    );

    // 环检测：root 挂到 grand 下（root 已是 grand 的祖先）→ 单向上溯 anc(grand) 含 root
    let cycle: Option<i32> = sqlx::query_scalar(
        r#"WITH RECURSIVE anc AS (
            SELECT ref_left AS node FROM isahl."zc_id_subj-org_rr_subordinate" WHERE ref_right = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT r.ref_left FROM isahl."zc_id_subj-org_rr_subordinate" r
            JOIN anc a ON a.node = r.ref_right WHERE r.deleted_at IS NULL
        )
        SELECT 1 FROM anc WHERE node = $2 LIMIT 1"#,
    )
    .bind(grand)
    .bind(root)
    .fetch_optional(&pool)
    .await
    .expect("tree cycle check");
    assert!(cycle.is_some(), "anc(grand) 应含 root → 成环");
    // 对照：root 挂 child 不环（child 非 root 祖先）
    let ok: Option<i32> = sqlx::query_scalar(
        r#"WITH RECURSIVE anc AS (
            SELECT ref_left AS node FROM isahl."zc_id_subj-org_rr_subordinate" WHERE ref_right = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT r.ref_left FROM isahl."zc_id_subj-org_rr_subordinate" r
            JOIN anc a ON a.node = r.ref_right WHERE r.deleted_at IS NULL
        )
        SELECT 1 FROM anc WHERE node = $2 LIMIT 1"#,
    )
    .bind(root)
    .bind(child)
    .fetch_optional(&pool)
    .await
    .expect("tree cycle check ok");
    assert!(ok.is_none(), "anc(root) 不应含 child");

    // 子树下钻（get_org_subtree 同 CTE）：root 下应含 child 与 grand
    let rows: Vec<(i64, i32)> = sqlx::query_as(
        r#"WITH RECURSIVE subtree AS (
            SELECT o.id, 0 AS level
            FROM (
                SELECT id FROM isahl."zc_id_orga-department" WHERE deleted_at IS NULL
                UNION ALL
                SELECT id FROM isahl."zc_id_orga-non-banking-legal" WHERE deleted_at IS NULL
            ) o WHERE o.id = $1
            UNION ALL
            SELECT n.id, s.level + 1
            FROM subtree s
            JOIN isahl."zc_id_subj-org_rr_subordinate" r ON r.ref_left = s.id AND r.deleted_at IS NULL
            JOIN (
                SELECT id FROM isahl."zc_id_orga-department" WHERE deleted_at IS NULL
                UNION ALL
                SELECT id FROM isahl."zc_id_orga-non-banking-legal" WHERE deleted_at IS NULL
            ) n ON n.id = r.ref_right
            WHERE s.level < 64
        )
        SELECT id, level FROM subtree"#,
    )
    .bind(root)
    .fetch_all(&pool)
    .await
    .expect("subtree");
    let ids: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&child), "子树应含 child");
    assert!(ids.contains(&grand), "子树应含 grand");

    // 解除挂接（软删）后子树不含 child
    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-org_rr_subordinate"
           SET deleted_at = now() WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(root)
    .bind(child)
    .execute(&pool)
    .await
    .expect("detach");
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-org_rr_subordinate", root, child).await,
        0
    );
}

/// 任职主体路由（route_employee_subject 同语义）：empl-natural / empl-agent 识别 + 桥挂接
#[tokio::test]
async fn employment_subject_routing_and_bridge() {
    let pool = test_pool().await;
    let natural = tid(51);
    let agent = tid(52);
    let unknown = tid(53);
    let pos = tid(54);
    let org = tid(55);

    sqlx::query(
        r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code) VALUES ($1, $2, $3)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(natural)
    .bind("自然人")
    .bind(format!("T-NAT-{natural}"))
    .execute(&pool)
    .await
    .expect("ensure natural");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_empl-agent" (id, notice, code) VALUES ($1, $2, $3)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(agent)
    .bind("智能体")
    .bind(format!("T-AGT-{agent}"))
    .execute(&pool)
    .await
    .expect("ensure agent");
    ensure_position(&pool, pos, "任职岗位", None).await;
    ensure_org(&pool, org, "任职组织").await;

    // 路由判断（与 route_employee_subject 相同两查询）
    let in_natural: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_empl-natural\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(natural)
    .fetch_one(&pool)
    .await
    .expect("natural check");
    assert!(in_natural, "自然人在 empl-natural 命中");
    let in_natural_agent: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_empl-natural\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(agent)
    .fetch_one(&pool)
    .await
    .expect("agent natural check");
    assert!(!in_natural_agent, "智能体不在 empl-natural");
    let in_agent: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_empl-agent\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(agent)
    .fetch_one(&pool)
    .await
    .expect("agent check");
    assert!(in_agent, "智能体在 empl-agent 命中");

    // 桥挂接：岗位任职 + 组织雇员
    for (table, left) in [
        ("zc_id_subj-post_rr_employee", pos),
        ("zc_id_subj-org_rr_employee", org),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(
            format!(
                r#"INSERT INTO isahl."{}" (ref_left, ref_right) VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
                table
            )
            .as_str(),
        ))
        .bind(left)
        .bind(natural)
        .execute(&pool)
        .await
        .expect("insert employment bridge");
    }
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-post_rr_employee", pos, natural).await,
        1
    );
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-org_rr_employee", org, natural).await,
        1
    );

    // 未知主体：两叶表皆无 → 路由拒绝（模拟 handler 的 400 分支）
    let in_nat_unknown: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_empl-natural\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(unknown)
    .fetch_one(&pool)
    .await
    .expect("unknown natural");
    let in_agt_unknown: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.\"zc_id_empl-agent\" WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(unknown)
    .fetch_one(&pool)
    .await
    .expect("unknown agent");
    assert!(
        !in_nat_unknown && !in_agt_unknown,
        "未知主体两叶表皆不命中 → handler 400 分支触发"
    );
}

/// group 成员桥：挂接（幂等）与软删解除
#[tokio::test]
async fn group_member_bridge_attach_and_detach() {
    let pool = test_pool().await;
    let group = tid(61);
    let member = tid(62);

    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-group" (id, notice, code) VALUES ($1, $2, $3)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(group)
    .bind("测试群组")
    .bind(format!("T-GRP-{group}"))
    .execute(&pool)
    .await
    .expect("ensure group");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code) VALUES ($1, $2, $3)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(member)
    .bind("群成员")
    .bind(format!("T-MEM-{member}"))
    .execute(&pool)
    .await
    .expect("ensure member");

    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-group_rr_member" (ref_left, ref_right)
           VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
    )
    .bind(group)
    .bind(member)
    .execute(&pool)
    .await
    .expect("attach member");
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-group_rr_member", group, member).await,
        1
    );

    sqlx::query(
        r#"UPDATE isahl."zc_id_subj-group_rr_member"
           SET deleted_at = now() WHERE ref_left = $1 AND ref_right = $2 AND deleted_at IS NULL"#,
    )
    .bind(group)
    .bind(member)
    .execute(&pool)
    .await
    .expect("detach member");
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-group_rr_member", group, member).await,
        0
    );
}
