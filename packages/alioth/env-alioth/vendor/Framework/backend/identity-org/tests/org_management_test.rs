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
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve org coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_orga-department" (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(notice)
    .bind(format!("T-ORG-{id}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(pool)
    .await
    .expect("ensure org");
}

async fn ensure_position(pool: &PgPool, id: i64, notice: &str, parent_id: Option<i64>) {
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↓_GG"))
        .await
        .expect("resolve position coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, fk_parent, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(id)
    .bind(notice)
    .bind(format!("T-POS-{id}"))
    .bind(parent_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
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

    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .expect("resolve natural coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(natural)
    .bind("自然人")
    .bind(format!("T-NAT-{natural}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("ensure natural");
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("ZJ", "LNC", "↓_EH"))
            .await
            .expect("resolve agent coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_empl-agent" (id, notice, code, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(agent)
    .bind("智能体")
    .bind(format!("T-AGT-{agent}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
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

    // 通讯录联系人：employee-list「员工=通讯录」语义（fix-avic-employee-assignment-contacts），
    // route_employee_subject 第三分支命中 → 桥可挂接
    let contact = tid(56);
    // 类契约：坐标形态（trigger-registry entity.rs 通讯联系触发器同款）——
    // 按维度 notice 解析 ZUID 注入 dk_scene/dk_factor/dk_function，_f_/_t_ 由派生层处理
    let dk_scene: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl.zc_id_scene WHERE notice = '通讯联络' LIMIT 1")
            .fetch_one(&pool)
            .await
            .ok();
    let dk_factor: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl.zc_id_factor WHERE notice = '通讯主体' LIMIT 1")
            .fetch_one(&pool)
            .await
            .ok();
    let dk_function: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl.zc_id_function WHERE notice = '通讯联系' LIMIT 1")
            .fetch_one(&pool)
            .await
            .ok();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_contacts (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(contact)
    .bind("通讯录员工")
    .bind(format!("T-CT-{contact}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("ensure contact");
    let in_contact: bool = sqlx::query_scalar(
        "SELECT COUNT(*) > 0 FROM isahl.zc_id_contacts WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(contact)
    .fetch_one(&pool)
    .await
    .expect("contact check");
    assert!(in_contact, "联系人在 zc_id_contacts 命中");
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
        .bind(contact)
        .execute(&pool)
        .await
        .expect("insert contact employment bridge");
    }
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-post_rr_employee", pos, contact).await,
        1
    );
    assert_eq!(
        bridge_count(&pool, "zc_id_subj-org_rr_employee", org, contact).await,
        1
    );
}

/// group 成员桥：挂接（幂等）与软删解除
#[tokio::test]
async fn group_member_bridge_attach_and_detach() {
    let pool = test_pool().await;
    let group = tid(61);
    let member = tid(62);

    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("ZB", "LNC", "↓_DA"))
            .await
            .expect("resolve group coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-group" (id, notice, code, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(group)
    .bind("测试群组")
    .bind(format!("T-GRP-{group}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("ensure group");
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .expect("resolve member coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_empl-natural" (id, notice, code, dk_scene, dk_factor, dk_function) VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(member)
    .bind("群成员")
    .bind(format!("T-MEM-{member}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
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

/// 岗位类型显示回归（fix-avic-position-category-display）：
/// ck_category 解析 = 类目-岗位 code → zc_id_category（父表跨子族）notice → ''，
/// 绝不外泄 ck_category::text 数字 id（用户报告岗位列表显示 2251799813685249）。
/// 直接执行与 org_tree.rs POSITION_SELECT 相同的 COALESCE 语义（handler 字段私有，
/// SQL 即 handler 行为）。
#[tokio::test]
async fn position_category_resolves_visible_label_no_raw_id() {
    let pool = test_pool().await;

    // 治理类基表行（ngac.sql 同款：code='ccb_member'，notice='CCB 成员'）
    let cat_base = tid(31);
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_cate-position" (id, notice, code)
           VALUES ($1, 'CCB 成员', 'ccb_member') ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(cat_base)
    .execute(&pool)
    .await
    .expect("ensure base category");

    // 基表行类岗位（业务 seed 同款：ck_category 直绑基表行 id）
    let pos_gov = tid(32);
    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("TX", "FJA", "↓_GG"))
            .await
            .expect("resolve governance position coords");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, ck_category, dk_scene, dk_factor, dk_function)
           VALUES ($1, 'CCB 主席', $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(pos_gov)
    .bind(format!("T-GOV-{pos_gov}"))
    .bind(cat_base)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("ensure governance position");

    // 无类别岗位（ck_category NULL）
    let pos_none = tid(33);
    ensure_position(&pool, pos_none, "无类别岗位", None).await;

    // handler POSITION_SELECT 同款解析表达式
    let resolved: String = sqlx::query_scalar(
        r#"SELECT COALESCE(
                 (SELECT c.code FROM isahl."zc_id_cate-position" c
                  WHERE c.id = p.ck_category AND c.deleted_at IS NULL),
                 (SELECT c2.notice FROM isahl.zc_id_category c2
                  WHERE c2.id = p.ck_category AND c2.deleted_at IS NULL),
                 '')
           FROM isahl."zc_id_subj-position" p WHERE p.id = $1 AND p.deleted_at IS NULL"#,
    )
    .bind(pos_gov)
    .fetch_one(&pool)
    .await
    .expect("resolve governance category");
    assert_eq!(
        resolved, "CCB 成员",
        "基表行类别应解析为其字典 notice，而非 ck_category::text 数字 id"
    );

    let resolved_none: String = sqlx::query_scalar(
        r#"SELECT COALESCE(
                 (SELECT c.code FROM isahl."zc_id_cate-position" c
                  WHERE c.id = p.ck_category AND c.deleted_at IS NULL),
                 (SELECT c2.notice FROM isahl.zc_id_category c2
                  WHERE c2.id = p.ck_category AND c2.deleted_at IS NULL),
                 '')
           FROM isahl."zc_id_subj-position" p WHERE p.id = $1 AND p.deleted_at IS NULL"#,
    )
    .bind(pos_none)
    .fetch_one(&pool)
    .await
    .expect("resolve no category");
    assert_eq!(resolved_none, "", "无类别岗位显示为空串");
}

/// 岗位读投影的任职员工 MUST 来自任职桥（fix-position-employee-binding-display）。
///
/// 回归：读投影曾只取主表 `fk_user`（岗位表单从不写、恒 NULL），而「添加任职员工」
/// 写的是 `zc_id_subj-post_rr_employee` 桥 → 提示成功却恒不展示。
/// 本用例直接调用生产投影常量 `POSITION_SELECT`（与 handler 同一 SQL，杜绝漂移）。
#[tokio::test]
async fn position_read_projection_exposes_bridge_employees() {
    use identity_org::handlers::org_tree::{position_row_to_dto, PositionRow, POSITION_SELECT};

    let pool = test_pool().await;
    let pos = tid(60);
    let employee = tid(61);

    ensure_position(&pool, pos, "任职投影岗位", None).await;

    // 任职主体 = 通讯录联系人（岗位页 picker 数据源）；坐标形态落库
    // （trigger-registry 通讯联系触发器同款，与 employment_subject_routing_and_bridge 一致）
    let dk_scene: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl.zc_id_scene WHERE notice = '通讯联络' LIMIT 1")
            .fetch_one(&pool)
            .await
            .ok();
    let dk_factor: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl.zc_id_factor WHERE notice = '通讯主体' LIMIT 1")
            .fetch_one(&pool)
            .await
            .ok();
    let dk_function: Option<i64> =
        sqlx::query_scalar("SELECT id FROM isahl.zc_id_function WHERE notice = '通讯联系' LIMIT 1")
            .fetch_one(&pool)
            .await
            .ok();
    sqlx::query(
        r#"INSERT INTO isahl.zc_id_contacts (id, notice, code, dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(employee)
    .bind("任职投影员工")
    .bind(format!("T-CT-{employee}"))
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .expect("ensure contact employee");

    // 任职桥（POST /positions/{id}/employees 写端落点）
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_subj-post_rr_employee" (ref_left, ref_right)
           VALUES ($1, $2)"#,
    )
    .bind(pos)
    .bind(employee)
    .execute(&pool)
    .await
    .expect("attach employment bridge");

    let sql = format!(
        "{} WHERE p.id = $1 AND p.deleted_at IS NULL AND p._f_ IS NULL",
        POSITION_SELECT
    );
    let row: PositionRow = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(pos)
        .fetch_one(&pool)
        .await
        .expect("read position projection");
    let json = serde_json::to_value(position_row_to_dto(row)).expect("serialize position dto");

    assert_eq!(
        json["employees"][0]["id"],
        employee.to_string(),
        "桥行 MUST 出现在读投影 employees（列表/详情同一投影）"
    );
    assert_eq!(
        json["employees"][0]["name"], "任职投影员工",
        "名称经 zc_id_subjects 根 → zc_id_contacts 兜底解析"
    );
    assert_eq!(
        json["userId"],
        serde_json::Value::Null,
        "fk_user 未被岗位表单写入——任职展示不得依赖该标量列"
    );
}
