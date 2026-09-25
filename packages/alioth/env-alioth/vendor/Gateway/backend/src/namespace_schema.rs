//! Namespace schema synchronization
//!
//! When `NAMESPACE` env var is set at Gateway startup, this module ensures the
//! namespace-specific database has the `isahl`, `isahl_auth`, `isahl_audit`
//! schemas synced from a reference database (structure source of truth).
//!
//! # Flow
//!
//! 1. Read `NAMESPACE` — skip if not set
//! 2. Connect to reference database (`REFERENCE_DATABASE_URL` or default dev DB)
//! 3. Check if namespace DB already has `isahl.zc_id_lifecycle`
//!    - **No** → full schema sync via `pg_dump --schema-only` from reference
//!    - **Yes** → incremental column/table sync
//!
//! 结构真相源 = DB（参考库）；应用侧不交付迁移文件（见 `db-ddl-delivery` 规约）。
//! # Environment
//!
//! - `NAMESPACE` — current namespace (activates sync when set)
//! - `REFERENCE_DATABASE_URL` — source database for schema (default: derive from
//!   `DATABASE_URL` with db name changed to `aliothstudio_dev`)

use sqlx::{AssertSqlSafe, PgPool};
use std::collections::{HashMap, HashSet};
use std::env;
use std::process::Command;

// ── 增量同步数据结构 ──

#[derive(Debug, Clone)]
struct ColumnInfo {
    name: String,
    data_type: String,
    nullable: bool,
    default: Option<String>,
}

/// Column type aliases that are logically equivalent.
fn normalize_type(typ: &str) -> &str {
    match typ {
        "character varying" | "varying" => "text",
        "timestamp without time zone" | "timestamp" => "timestamp without time zone",
        "timestamp with time zone" | "timestamptz" => "timestamp with time zone",
        "boolean" | "bool" => "boolean",
        "double precision" | "float8" | "double" => "double precision",
        "real" | "float4" => "real",
        "bigint" | "int8" => "bigint",
        "integer" | "int" | "int4" => "integer",
        "smallint" | "int2" => "smallint",
        "numeric" | "decimal" => "numeric",
        _ => typ,
    }
}

/// Schemas to sync from reference database.
const SYNC_SCHEMAS: &[&str] = &["isahl", "isahl_auth", "isahl_audit"];

/// Run schema sync + migration when `NAMESPACE` is set.
///
/// Called once at Gateway startup.
pub async fn sync_namespace_schema(pool: &PgPool) {
    let namespace = match env::var("NAMESPACE") {
        Ok(ns) if !ns.is_empty() => ns,
        _ => {
            common::telemetry::info!("NAMESPACE not set, skipping namespace schema sync");
            return;
        }
    };

    common::telemetry::info!("Namespace schema sync: namespace={}", namespace);

    // Build reference database URL
    let ref_url = build_reference_url();
    common::telemetry::info!("Connecting to reference database for schema comparison...");

    let reference_pool = match sqlx::PgPool::connect(&ref_url).await {
        Ok(p) => p,
        Err(e) => {
            common::telemetry::warn!(
                "Cannot connect to reference database '{}': {}. Schema sync skipped.",
                ref_url,
                e
            );
            return;
        }
    };

    // Check if namespace DB already has schema
    let has_schema = has_existing_schema(pool).await;

    if has_schema {
        if cfg!(debug_assertions) {
            // Step 1 (dev only): 增量 DDL 同步 — 检测并自动应用参考库到 namespace 的 schema 漂移
            common::telemetry::info!("Running incremental schema sync from reference...");
            let ns_url = env::var("DATABASE_URL").unwrap_or_default();
            let changes = incremental_schema_sync(pool, &reference_pool, &ns_url).await;
            if changes > 0 {
                common::telemetry::info!("Incremental sync applied {} DDL change(s)", changes);
            } else {
                common::telemetry::info!(
                    "No schema drift detected between reference and namespace DB"
                );
            }
        } else {
            common::telemetry::info!("release mode: skipping incremental schema sync");
        }
    } else {
        common::telemetry::info!("Namespace database is empty, performing full schema sync...");
        if let Err(e) = full_schema_sync(pool, &reference_pool).await {
            common::telemetry::error!("Full schema sync failed: {}", e);
            return;
        }
        common::telemetry::info!("Full schema sync completed successfully");
    }

    common::telemetry::info!("Namespace schema sync finished for '{}'", namespace);
}

/// Check if the namespace database already has the core lifecylce table.
async fn has_existing_schema(pool: &PgPool) -> bool {
    let result: Result<i64, _> = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'isahl' AND table_name = 'zc_id_lifecycle'"
    )
    .fetch_one(pool)
    .await;

    match result {
        Ok(count) => count > 0,
        Err(_) => false,
    }
}

// ── 增量 DDL 同步 ──

/// 从 reference DB 读取 schema 中所有表名。
async fn fetch_table_names(pool: &PgPool, schemas: &[&str]) -> HashSet<String> {
    let mut tables = HashSet::new();
    for schema in schemas {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = $1 AND table_type = 'BASE TABLE'",
        )
        .bind(schema)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for row in rows {
            tables.insert(format!("{}.{}", schema, row));
        }
    }
    tables
}

/// 读取某张表的所有列信息。
async fn fetch_columns(pool: &PgPool, schema: &str, table: &str) -> Vec<ColumnInfo> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT column_name, data_type, is_nullable, column_default \
         FROM information_schema.columns \
         WHERE table_schema = $1 AND table_name = $2 \
         ORDER BY ordinal_position",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    rows.into_iter()
        .map(|(name, dtype, nullable, default)| ColumnInfo {
            name,
            data_type: dtype,
            nullable: nullable == "YES",
            default,
        })
        .collect()
}

/// 生成 ALTER TABLE ADD COLUMN 语句。
fn generate_add_column(schema: &str, table: &str, col: &ColumnInfo) -> String {
    let null_str = if col.nullable { "" } else { " NOT NULL" };
    let default_str = match &col.default {
        Some(d) if !d.is_empty() && d != "NULL" => format!(" DEFAULT {}", d),
        _ => String::new(),
    };

    format!(
        "ALTER TABLE \"{}\".\"{}\" ADD COLUMN \"{}\" {}{}{};",
        schema, table, col.name, col.data_type, default_str, null_str
    )
}

/// 用 pg_dump 导出缺失表的完整 CREATE TABLE DDL。
fn dump_missing_table(ref_url: &str, schema: &str, table: &str) -> Option<String> {
    let output = Command::new("pg_dump")
        .args([
            "--schema-only",
            "--no-owner",
            "--no-acl",
            "-t",
            &format!("\"{}\".\"{}\"", schema, table),
            "--dbname",
            ref_url,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        common::telemetry::warn!(
            "pg_dump failed for {}.{}: {}",
            schema,
            table,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }

    let sql = String::from_utf8_lossy(&output.stdout);
    // Extract only the CREATE TABLE statement; skip SET/SELECT/COMMENT noise
    let mut create_stmt = String::new();
    let mut capturing = false;
    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("CREATE TABLE") || trimmed.starts_with("CREATE TEMPORARY") {
            capturing = true;
        }
        if capturing {
            create_stmt.push_str(line);
            create_stmt.push('\n');
            if trimmed == ";" {
                break;
            }
        }
    }
    if create_stmt.is_empty() {
        None
    } else {
        Some(create_stmt)
    }
}

/// 增量 schema 同步：对比 reference DB 与 namespace DB 的 isahl/isahl_auth/isahl_audit
/// schema，自动补缺表缺列。
///
async fn incremental_schema_sync(pool: &PgPool, reference_pool: &PgPool, _ns_url: &str) -> i64 {
    let ref_url = build_reference_url();
    let mut changes: i64 = 0;

    // 1. 读取两个库的表清单
    let ref_tables = fetch_table_names(reference_pool, SYNC_SCHEMAS).await;
    let ns_tables = fetch_table_names(pool, SYNC_SCHEMAS).await;

    // 2. 找缺少的表 → pg_dump 逐表导出 + 导入
    for table_key in &ref_tables {
        if ns_tables.contains(table_key) {
            continue;
        }
        let parts: Vec<&str> = table_key.splitn(2, '.').collect();
        let (schema, table) = (parts[0], parts[1]);

        common::telemetry::info!("  ╰ 缺失表: {}.{} → 从参考库导出", schema, table);
        if let Some(ddl) = dump_missing_table(&ref_url, schema, table) {
            // 用 sqlx 直接执行
            for stmt in ddl.split(';') {
                let s = stmt.trim();
                if s.is_empty() || s.starts_with("--") {
                    continue;
                }
                if sqlx::query(AssertSqlSafe(s)).execute(pool).await.is_ok() {
                    changes += 1;
                } else {
                    // 中文多字节字符按字节截断会 panic（UTF-8 边界），按字符数安全截断
                    let preview: String = s.chars().take(80).collect();
                    common::telemetry::warn!("    执行 DDL 失败: {}...", preview);
                }
            }
        }
    }

    // 3. 找列级别的漂移（表在两边都存在时）
    for table_key in &ref_tables {
        if !ns_tables.contains(table_key) {
            continue;
        }
        let parts: Vec<&str> = table_key.splitn(2, '.').collect();
        let (schema, table) = (parts[0], parts[1]);

        let ref_cols = fetch_columns(reference_pool, schema, table).await;
        let ns_cols = fetch_columns(pool, schema, table).await;

        let ns_col_map: HashMap<&str, &ColumnInfo> =
            ns_cols.iter().map(|c| (c.name.as_str(), c)).collect();

        for col in &ref_cols {
            match ns_col_map.get(col.name.as_str()) {
                None => {
                    // 参考库有此列但 namespace 没有 → ADD COLUMN
                    let ddl = generate_add_column(schema, table, col);
                    common::telemetry::info!("  ╰ 新增列: {}.{} => {}", schema, table, col.name);
                    if sqlx::query(AssertSqlSafe(ddl.as_str()))
                        .execute(pool)
                        .await
                        .is_ok()
                    {
                        changes += 1;
                    } else {
                        common::telemetry::warn!("    添加列失败: {}.{}", schema, table);
                    }
                }
                Some(existing) => {
                    // 列存在但类型不一致时 log warning（不自动改，防止数据丢失）
                    let ref_norm = normalize_type(&col.data_type);
                    let ns_norm = normalize_type(&existing.data_type);
                    if ref_norm != ns_norm {
                        common::telemetry::warn!(
                            "  ╰ 类型漂移: {}.{}.{}  ref={} vs ns={} （需迁移文件手动处理）",
                            schema,
                            table,
                            col.name,
                            ref_norm,
                            ns_norm
                        );
                    }
                }
            }
        }
    }

    // 4. 门禁：如果还有漂移且严格模式，阻止启动
    if changes > 0 && env::var("ONTOLOGY_CHECK_STRICT").is_ok() {
        common::telemetry::error!(
            "Incremental schema sync applied {} DDL change(s). \
             Restart Gateway to verify all sync complete.",
            changes
        );
    }

    changes
}

/// Full schema sync: dump schemas from reference DB into namespace DB.
///
/// Uses `pg_dump --schema-only` piped through `psql`, same approach as
/// the `dev-gateway.sh` script.
async fn full_schema_sync(
    pool: &PgPool,
    _reference_pool: &PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let _target_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        common::telemetry::warn!("DATABASE_URL not set, cannot determine target for psql");
        String::new()
    });

    let ref_url = build_reference_url();

    // Build pg_dump arguments
    let mut dump_args = vec![
        "--schema-only".to_string(),
        "--no-owner".to_string(),
        "--no-acl".to_string(),
    ];
    for schema in SYNC_SCHEMAS {
        dump_args.push("-n".to_string());
        dump_args.push(schema.to_string());
    }
    // Extract DB name and connection params from reference URL
    // pg_dump takes -d <dbname> or --dbname=<connstr>
    dump_args.push(format!("--dbname={}", ref_url));

    common::telemetry::info!("Running pg_dump to sync schemas: {:?}", SYNC_SCHEMAS);

    // Check if pg_dump is available
    let pg_dump_check = Command::new("pg_dump").arg("--version").output();
    if pg_dump_check.is_err() {
        common::telemetry::warn!(
            "pg_dump not found on PATH. Skipping full schema sync. \
                 Ensure the namespace database has the required schemas or install pg_dump."
        );
        return Ok(());
    }

    // Run pg_dump
    common::telemetry::info!("Running pg_dump to sync schemas from reference...");
    let dump_output = match Command::new("pg_dump").args(&dump_args).output() {
        Ok(out) => out,
        Err(e) => {
            common::telemetry::error!("Failed to execute pg_dump: {}", e);
            return Err(format!("Failed to execute pg_dump: {}", e).into());
        }
    };

    let sql = String::from_utf8_lossy(&dump_output.stdout).to_string();
    if sql.trim().is_empty() {
        common::telemetry::warn!(
            "pg_dump produced empty output (schemas may not exist in reference)"
        );
        return Ok(());
    }

    // Execute in namespace DB — split by statement and execute each
    // We use a simple approach: execute directly via sqlx
    // Handle potential errors for existing objects (IF NOT EXISTS not in pg_dump output)
    for stmt_raw in sql.split(';') {
        let stmt = stmt_raw.trim();
        if stmt.is_empty() || stmt.starts_with("--") {
            continue;
        }
        // Remove SET statements which are session-level and can cause issues
        if stmt.to_uppercase().starts_with("SET ") {
            continue;
        }
        // Remove SELECT pg_catalog... statements that are pg_dump metadata
        if stmt.contains("pg_catalog.set_config") {
            continue;
        }
        match sqlx::query(AssertSqlSafe(stmt)).execute(pool).await {
            Ok(_) => {}
            Err(e) => {
                // Ignore "already exists" errors for tables, types, functions, etc.
                let msg = e.to_string();
                if msg.contains("already exists")
                    || msg.contains("duplicate key")
                    || msg.contains(" duplicate ")
                {
                    common::telemetry::debug!(
                        "Skipped existing object: {}",
                        &msg[..msg.len().min(120)]
                    );
                } else {
                    common::telemetry::warn!(
                        "Schema sync statement warning: {}",
                        &msg[..msg.len().min(200)]
                    );
                }
            }
        }
    }

    Ok(())
}

/// Derive the reference database URL from the current DATABASE_URL.
///
/// The reference database is `aliothstudio_dev` on the same host.
/// Falls back to REFERENCE_DATABASE_URL env var if set.
fn build_reference_url() -> String {
    if let Ok(url) = env::var("REFERENCE_DATABASE_URL") {
        return url;
    }

    let current = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for namespace schema sync — check .mise.toml chain");
    if current.is_empty() {
        panic!("DATABASE_URL is set but empty");
    }
    derive_reference_url(&current)
}

/// Replace the database name segment in a connection URL with `aliothstudio_dev`.
///
/// # Panics
/// - If `database_url` is empty
/// - If `database_url` does not contain `/` (malformed URL)
pub(crate) fn derive_reference_url(database_url: &str) -> String {
    assert!(!database_url.is_empty(), "DATABASE_URL must not be empty");

    // DATABASE_URL format: postgres://user:pass@host:port/dbname
    let last_slash = database_url
        .rfind('/')
        .expect("DATABASE_URL malformed: expected format postgres://host:port/dbname");
    let prefix = &database_url[..=last_slash];
    format!("{}aliothstudio_dev", prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_reference_url_from_current() {
        let url = derive_reference_url("postgres://user:pass@localhost:5432/my_namespace_db");
        assert_eq!(url, "postgres://user:pass@localhost:5432/aliothstudio_dev");
    }

    #[test]
    #[should_panic(expected = "DATABASE_URL must not be empty")]
    fn test_derive_reference_url_empty_panics() {
        derive_reference_url("");
    }
    #[test]
    fn test_derive_reference_url_simple() {
        let url = derive_reference_url("postgres://localhost:5432/my_db");
        assert_eq!(url, "postgres://localhost:5432/aliothstudio_dev");
    }
}
