//! 线路局部更新：静态 SQL 化（`check-dynamic-table-name` 列位债务清偿）回归测试。
//!
//! 背景：`TrafficLineRepository::update` 原以 `format!` 拼 `SET {…}`（谓词式局部更新），
//! 属门禁口径的「列位可枚举未静态化」债务。清偿后为**读—改—写 + 全静态 SQL**（同 crate
//! `seal.rs` 同形）。本测试锁其对外契约（DB 行为，非实现形态）：
//!   ① 缺省字段沿用既有值（`None` = 不改动该列）；
//!   ② 五个绑定槽位互不串位（文本列与两个标量引用列用可辨识取值交叉验证）；
//!   ③ 无字段改动 ⇒ **不写库**（`updated_at`/`updated_by_id` 不推进）；
//!   ④ 目标行不存在 ⇒ `Ok(None)`。
//!
//! 依赖：test 库存在 `isahl.zc_id_stor-traffic_line` 与 `("TX","FJA","↑_GG")` 坐标声明。
//! （该表**无外键约束**，故两个标量引用列可用任意可辨识 i64 取值。）

use identity_org::models::UpdateTrafficLineRequest;
use identity_org::repository::traffic_line::TrafficLineRepository;
use sqlx::PgPool;

use crud::repository::AliothRepository;

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

/// 动态测试 id 段（纳秒派生；不与他测/他轮冲突）
fn base() -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as i64;
    8_950_000_000_000 + (nanos % 1_000_000) * 1000
}

/// 已存在行读数：`(code, notice, comments, fk_trustee, qk_path, updated_by_id, updated_at_epoch)`
type Row = (String, String, String, i64, Option<i64>, i64, f64);

async fn read_row(pool: &PgPool, id: i64) -> Row {
    sqlx::query_as(
        r#"SELECT code, notice, comments, fk_trustee, qk_path, updated_by_id,
                  COALESCE(extract(epoch from updated_at), 0)::float8
           FROM "isahl"."zc_id_stor-traffic_line" WHERE id = $1"#,
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("read row")
}

/// 建一行：可写字段有可辨识取值（两标量引用列取互不相同且非空的值，用于捕获绑序缺陷）；
/// `qk_path` 可置空——`get` 的坐标节点回读（`enrich_qk_path_ak_nodes`）对 `point` 列类型的
/// 既有依赖未决（见账本 Open），凡不测该槽的用例传 `None` 以免耦合该路径。
async fn setup(pool: &PgPool, qk_path: Option<i64>) -> (i64, Row) {
    let b = base();
    let (dk_scene, dk_factor, dk_function) = ontology_binding::resolve(pool, ("TX", "FJA", "↑_GG"))
        .await
        .expect("resolve traffic-line coords");
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO "isahl"."zc_id_stor-traffic_line"
           (code, notice, comments, fk_trustee, qk_path, dk_scene, dk_factor, dk_function,
            created_by_id, updated_by_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, 1) RETURNING id"#,
    )
    .bind(format!("TL-U{b}"))
    .bind("原线路名")
    .bind("原注释")
    .bind(b + 100_000_000)
    .bind(qk_path)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(pool)
    .await
    .expect("insert traffic line");
    (id, read_row(pool, id).await)
}

fn req(
    code: Option<&str>,
    notice: Option<&str>,
    comments: Option<&str>,
    fk_trustee: Option<i64>,
    qk_path: Option<i64>,
) -> UpdateTrafficLineRequest {
    UpdateTrafficLineRequest {
        code: code.map(str::to_string),
        notice: notice.map(str::to_string),
        comments: comments.map(str::to_string),
        fk_trustee,
        qk_path,
    }
}

#[tokio::test]
async fn partial_update_preserves_absent_fields() {
    let pool = test_pool().await;
    let (id, before) = setup(&pool, Some(base() + 200_000_000)).await;
    let repo = TrafficLineRepository::new(pool.clone());

    // 只改 notice：其余四列（含两个标量引用列）MUST 原样保留
    let ret = repo
        .update(id, req(None, Some("改后线路名"), None, None, None), 42)
        .await
        .expect("update notice");
    assert_eq!(
        ret.expect("row exists").notice.as_deref(),
        Some("改后线路名"),
        "返回值须为更新后的行"
    );
    let after = read_row(&pool, id).await;
    assert_eq!(after.0, before.0, "code 不得被改（缺省字段沿用既有值）");
    assert_eq!(after.2, before.2, "comments 不得被改");
    assert_eq!(after.3, before.3, "fk_trustee 不得被改");
    assert_eq!(after.4, before.4, "qk_path 不得被改");
    assert_eq!(after.1, "改后线路名", "notice 须已更新");
    assert_eq!(after.5, 42, "updated_by_id 须记录操作者");
    assert!(after.6 > before.6, "真改动须推进 updated_at");

    // 只改 qk_path：验证两个标量引用绑定槽不串位（同类型，串位不会被类型系统发现）
    let new_path = before.4.expect("fixture 设置 qk_path") + 1;
    let ret = repo
        .update(id, req(None, None, None, None, Some(new_path)), 42)
        .await
        .expect("update qk_path");
    assert!(ret.is_some(), "行存在 ⇒ Some");
    let after2 = read_row(&pool, id).await;
    assert_eq!(after2.4, Some(new_path), "qk_path 须已更新");
    assert_eq!(after2.3, before.3, "fk_trustee 不得随 qk_path 写入而变动");
    assert_eq!(after2.0, before.0, "code 不得变动");
    assert_eq!(after2.1, after.1, "notice 不得变动");
    assert_eq!(after2.2, before.2, "comments 不得变动");
}

#[tokio::test]
async fn empty_update_does_not_write() {
    let pool = test_pool().await;
    // `qk_path` 置空：本用例只测「空更新不写库」，不依赖坐标节点回读路径
    let (id, before) = setup(&pool, None).await;
    let repo = TrafficLineRepository::new(pool.clone());

    // 先做一次真改动（把 updated_by_id 推进到 42），再发空更新：MUST 不写库
    repo.update(id, req(None, Some("先改一次"), None, None, None), 42)
        .await
        .expect("first update")
        .expect("row exists");
    let mid = read_row(&pool, id).await;

    let ret = repo
        .update(id, req(None, None, None, None, None), 999)
        .await
        .expect("empty update");
    assert!(ret.is_some(), "空更新仍须返回该行");
    let after = read_row(&pool, id).await;
    assert_eq!(
        after, mid,
        "空更新 MUST 零写库（updated_at/updated_by_id 均不推进）"
    );
    assert_eq!(after.5, 42, "updated_by_id 不得被空更新改写为 999");
    // 第一次真改动确实生效（防「空更新不写」被误判为「全不写」）
    assert_eq!(after.1, "先改一次");
    assert!(after.6 > before.6);
}

#[tokio::test]
async fn geometry_read_path_respects_form_contract() {
    let pool = test_pool().await;
    let (id, _before) = setup(&pool, Some(base() + 200_000_000)).await;
    let repo = TrafficLineRepository::new(pool.clone());

    let form: String = sqlx::query_scalar(
        r#"SELECT udt_schema || '.' || udt_name
           FROM information_schema.columns
           WHERE table_schema = 'isahl' AND table_name = 'zc_id_geom-coordinate' AND column_name = 'point'"#,
    )
    .fetch_one(&pool)
    .await
    .expect("读取几何列形态");

    // ADR D-028 §7（用户裁决 2026-09-21）：运行时仅支持 PostGIS 形态；发布产物的原生形态
    // 仅用于开源模型分发，读径 MUST 点名报错而非透出底层方言错误。断言按实际形态分支，
    // 故本用例在「运行时形态库」与「发布形态库」两种环境都可运行。
    let res = repo.get(id).await;
    if form.starts_with("postgis.") {
        assert!(
            res.is_ok(),
            "PostGIS 形态（运行时形态）下几何读径 MUST 可用，实际：{:?}",
            res.err()
        );
    } else {
        let err = res.expect_err("非支持形态下几何读径 MUST 报错");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("几何列形态不受支持") && msg.contains(&form),
            "错误 MUST 点名实际形态（{form}）与整改指引，实际：{msg}"
        );
        assert!(
            !msg.contains("decode("),
            "MUST NOT 透出底层方言错误（`函数 decode(point, unknown) 不存在`）：{msg}"
        );
    }
}

#[tokio::test]
async fn update_missing_row_returns_none() {
    let pool = test_pool().await;
    let repo = TrafficLineRepository::new(pool.clone());
    // 极小 id 段：几乎不可能与真实行冲突（真实 id 量级 8.9e13+）
    let ghost = 1_000_001;
    let ret = repo
        .update(ghost, req(None, Some("不存在"), None, None, None), 42)
        .await
        .expect("update must not error");
    assert!(ret.is_none(), "目标行不存在 ⇒ Ok(None)");
}
