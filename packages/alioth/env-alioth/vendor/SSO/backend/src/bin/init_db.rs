use sqlx::{AssertSqlSafe, PgPool};
use std::env;
use std::fs;
use std::path::Path;

#[tokio::main]
async fn main() {
    let database_url = env::var("DATABASE_URL").expect("需要设置 DATABASE_URL 环境变量");

    let migrations_dir = env::args()
        .nth(1)
        .or_else(|| env::var("MIGRATIONS_DIR").ok())
        .unwrap_or_else(|| "migrations".to_string());

    println!("Connecting to database...");
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to create pool");

    let migrations_path = Path::new(&migrations_dir);

    println!(
        "Looking for migrations in: {:?}",
        migrations_path
            .canonicalize()
            .unwrap_or_else(|_| migrations_path.to_path_buf())
    );
    println!(
        "Note: SSO 迁移(004-007)为增量，基础 schema(isahl_auth/isahl_audit)由共享库提供；\
         独立部署时 DATABASE_URL 应指向与 Gateway 共享的库。"
    );

    let mut entries: Vec<_> = fs::read_dir(migrations_path)
        .expect("Failed to read migrations directory")
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            path.extension().is_some_and(|ext| ext == "sql")
        })
        .collect();

    // 按文件名排序
    entries.sort_by_key(|a| a.file_name());

    // 迁移 tracking 表（对齐 Gateway namespace_schema.rs 的 MIGRATIONS_TABLE 模式）：
    // 记录已成功应用的迁移，避免每次全量重跑（幂等重跑靠容错，但无记录）。
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS isahl_auth.sso_migrations (\
             filename TEXT PRIMARY KEY, \
             applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW())",
    )
    .execute(&pool)
    .await
    .expect("create sso_migrations tracking table");

    for entry in entries {
        let path = entry.path();
        let filename = path.file_name().unwrap().to_str().unwrap();

        // 已记录 → 跳过
        let already_applied: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM isahl_auth.sso_migrations WHERE filename = $1)",
        )
        .bind(filename)
        .fetch_one(&pool)
        .await
        .expect("query sso_migrations");
        if already_applied {
            println!("Skipping: {} (already applied)", filename);
            continue;
        }

        println!("Applying: {}", filename);

        let sql =
            fs::read_to_string(&path).unwrap_or_else(|_| panic!("Failed to read {}", filename));

        // 用 raw_sql 整文件执行：PostgreSQL 原生解析注释/分号/引号/DO 块，
        // 避免 split(';') 把 DO \$\$ ... \$\$ 内部的分号误判为语句边界，
        // 导致 seed 迁移（005/008/015 含 DO 块）被切碎后整体失败而静默丢失。
        // 对齐 Gateway namespace_schema.rs 的迁移执行模式。
        let result = sqlx::raw_sql(AssertSqlSafe(sql.as_str()))
            .execute(&pool)
            .await;
        match result {
            Ok(_) => {
                record_migration(&pool, filename).await;
                println!("  ✓ Success");
            }
            Err(e) => {
                // 幂等重跑语义：对象已存在/唯一约束冲突时跳过。
                // 用 SQLSTATE 判定（23505 unique_violation / 42P07 duplicate_table /
                // 42710 duplicate_object / 42701 duplicate_column / 42P04 duplicate_database），
                // 不依赖错误文案（本地化 PostgreSQL 报中文「重复键」，英文 contains 匹配失效）。
                let idempotent = e
                    .as_database_error()
                    .and_then(|d| d.code().map(|c| c.to_string()))
                    .is_some_and(|code| {
                        matches!(
                            code.as_str(),
                            "23505" | "42P07" | "42710" | "42701" | "42P04"
                        )
                    });
                if idempotent {
                    // 视为已应用（其 seed 部分在首次执行时已完成，此处仅重跑冲突）。
                    record_migration(&pool, filename).await;
                    println!("  ✓ Success (idempotent skip)");
                } else {
                    // 不再静默忽略 does not exist 等错误——那会掩盖缺失依赖/迁移顺序问题。
                    println!("  ✗ Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    println!("\n✅ Database initialized!");
}

/// 记录迁移已应用（幂等）。
async fn record_migration(pool: &PgPool, filename: &str) {
    if let Err(e) = sqlx::query(
        "INSERT INTO isahl_auth.sso_migrations (filename) VALUES ($1) ON CONFLICT (filename) DO NOTHING",
    )
    .bind(filename)
    .execute(pool)
    .await
    {
        eprintln!("⚠️  Failed to record migration {}: {}", filename, e);
    }
}
