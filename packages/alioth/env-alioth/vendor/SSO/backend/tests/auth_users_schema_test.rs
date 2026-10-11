//! `auth-users-schema` 能力机械契约（openspec/changes/migrate-auth-users-enum-and-ldap-jsonb）
//!
//! 本用例是该能力**唯一**的机械守卫：把「列面 / 枚举类型与域 / 枚举拒绝非法值 / ldap jsonb 形态」
//! 四条不变量钉在测试库上。任何人把列改回 text、加回 name/email 冗余列、或把 ldap 换回两列，
//! 都会在此失败——这些正是本变更的消费者可见语义（登录门禁判据、管理端 LDAP 筛选、邮箱唯一存储）。
//!
//! 结构收敛入口与服务端同源：`common::setup_schema` → 运行时 ensure（单一实现）。

mod common;

use gateway_sso::ngac::ensure::ensure_ngac_extension_tables;
use sqlx::PgPool;

const LEGACY_COLUMNS: [&str; 4] = ["name", "email", "is_ldap_user", "ldap_dn"];
const STATUS_DOMAIN: [&str; 6] = [
    "active",
    "pending",
    "pending_approval",
    "identity_submitted",
    "disabled",
    "rejected",
];
const TYPE_DOMAIN: [&str; 4] = ["standard", "external", "service", "system"];
const PROBE_PREFIX: &str = "au_schema_probe_";

async fn connect() -> PgPool {
    let pool = ::common::testing::connect_test_db().await;
    common::setup_schema(&pool).await.expect("setup_schema");
    // ensure 幂等性：二次调用 MUST 不报错（服务每次启动都会重放）
    ensure_ngac_extension_tables(&pool).await;
    pool
}

async fn cleanup(pool: &PgPool) {
    let _ = sqlx::query("DELETE FROM isahl_auth.auth_users WHERE username LIKE $1")
        .bind(format!("{PROBE_PREFIX}%"))
        .execute(pool)
        .await;
}

#[tokio::test]
async fn auth_users_column_shape_has_no_legacy_columns() {
    let pool = connect().await;

    let names: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns \
          WHERE table_schema = 'isahl_auth' AND table_name = 'auth_users'",
    )
    .fetch_all(&pool)
    .await
    .expect("read columns");

    for legacy in LEGACY_COLUMNS {
        assert!(
            !names.iter().any(|n| n == legacy),
            "`{legacy}` MUST NOT 存在于 isahl_auth.auth_users（唯一存储/唯一载体见 auth-users-schema）"
        );
    }
    assert!(names.iter().any(|n| n == "ldap"), "`ldap` 列 MUST 存在");
    assert!(
        names.iter().any(|n| n == "username"),
        "`username` 唯一身份基点 MUST 存在"
    );

    let ldap_type: String = sqlx::query_scalar(
        "SELECT data_type FROM information_schema.columns \
          WHERE table_schema = 'isahl_auth' AND table_name = 'auth_users' AND column_name = 'ldap'",
    )
    .fetch_one(&pool)
    .await
    .expect("read ldap type");
    assert_eq!(ldap_type, "jsonb", "`ldap` MUST 为 jsonb");
}

#[tokio::test]
async fn auth_users_status_and_user_type_are_enums_with_exact_domains() {
    let pool = connect().await;

    for (column, type_name, default_leading) in [
        ("status", "isahl_auth.auth_user_status", "'active'"),
        ("user_type", "isahl_auth.auth_user_type", "'standard'"),
    ] {
        let is_enum: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT atttypid = '{type_name}'::regtype FROM pg_attribute \
              WHERE attrelid = 'isahl_auth.auth_users'::regclass AND attname = '{column}' AND NOT attisdropped"
        )))
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("read {column} type: {e}"));
        assert!(
            is_enum,
            "`{column}` MUST 为原生枚举 {type_name}（MUST NOT 退化为 text）"
        );

        let default_expr: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT pg_get_expr(d.adbin, d.adrelid) FROM pg_attrdef d \
               JOIN pg_attribute a ON a.attrelid = d.adrelid AND a.attnum = d.adnum \
              WHERE d.adrelid = 'isahl_auth.auth_users'::regclass AND a.attname = '{column}'"
        )))
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|e| panic!("read {column} default: {e}"));
        assert!(
            default_expr.starts_with(default_leading),
            "`{column}` 默认值 MUST 以 {default_leading} 起（实得 {default_expr}）"
        );
    }

    for (type_name, expected) in [
        ("isahl_auth.auth_user_status", STATUS_DOMAIN.to_vec()),
        ("isahl_auth.auth_user_type", TYPE_DOMAIN.to_vec()),
    ] {
        let labels: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT enumlabel FROM pg_enum WHERE enumtypid = '{type_name}'::regtype ORDER BY enumsortorder"
        )))
        .fetch_all(&pool)
        .await
        .unwrap_or_else(|e| panic!("read {type_name} labels: {e}"));
        assert_eq!(
            labels, expected,
            "{type_name} 值域 MUST 逐字匹配（新增值须先 ALTER TYPE ADD VALUE 再入代码）"
        );
    }
}

#[tokio::test]
async fn auth_users_rejects_out_of_domain_status() {
    let pool = connect().await;
    cleanup(&pool).await;

    let err =
        sqlx::query("INSERT INTO isahl_auth.auth_users (username, status) VALUES ($1, 'bogus')")
            .bind(format!("{PROBE_PREFIX}bad"))
            .execute(&pool)
            .await;
    assert!(
        err.is_err(),
        "枚举域外值 `status='bogus'` MUST 被数据库拒绝"
    );

    // 合法值 MUST 通过（对照组，排除「一律拒绝」的假阳性）
    sqlx::query("INSERT INTO isahl_auth.auth_users (username, display_name) VALUES ($1, 'probe')")
        .bind(format!("{PROBE_PREFIX}ok"))
        .execute(&pool)
        .await
        .expect("合法行 MUST 可插入（status 取默认 active）");

    cleanup(&pool).await;
}

#[tokio::test]
async fn auth_users_ldap_jsonb_roundtrip() {
    let pool = connect().await;
    cleanup(&pool).await;

    sqlx::query(
        "INSERT INTO isahl_auth.auth_users (username, ldap) VALUES ($1, jsonb_build_object('dn', $2))",
    )
    .bind(format!("{PROBE_PREFIX}ldap"))
    .bind("uid=probe,dc=corp,dc=x")
    .execute(&pool)
    .await
    .expect("LDAP 绑定行 MUST 可写入");

    let dn: String =
        sqlx::query_scalar("SELECT ldap->>'dn' FROM isahl_auth.auth_users WHERE username = $1")
            .bind(format!("{PROBE_PREFIX}ldap"))
            .fetch_one(&pool)
            .await
            .expect("ldap->>'dn' MUST 可读");
    assert_eq!(dn, "uid=probe,dc=corp,dc=x");

    let is_null: bool = sqlx::query_scalar(
        "SELECT ldap IS NULL FROM isahl_auth.auth_users WHERE username NOT LIKE $1 LIMIT 1",
    )
    .bind(format!("{PROBE_PREFIX}%"))
    .fetch_one(&pool)
    .await
    .expect("非 LDAP 行 MUST 存在（库内基线账号）");
    assert!(
        is_null,
        "非 LDAP 账号 `ldap` MUST 为 NULL（判据 ldap IS NOT NULL）"
    );

    cleanup(&pool).await;
}
