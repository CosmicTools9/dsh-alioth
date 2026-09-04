//! 进程内启动种子自愈：模型级 + namespace 级（add-gateway-startup-seed-autoload）
//!
//! 定位：把种子自动载入下沉到 Gateway 进程本身——无论经 wrapper 脚本
//! （dev-gateway.sh / Deploy start.sh）还是直接启动（mise run dev / cargo run /
//! 二进制），启动序列都会幂等重放两级种子：
//!
//! 1. 模型级：`Framework/seed/`（dev）或 `Deploy/{ns}/seed/model-seed` 软链
//!    （release，DEPLOY_PATH）。按 `seed-dimensions.meta.json` 键先建
//!    `uq_seed_id_*` 唯一索引（对齐 Deploy start.sh 4b），再字典序重放
//!    `seed-*.sql`（剥除 pg_dump 的 `\restrict`/`\unrestrict`）。
//! 2. namespace 级：`Pre-Proc/{ns}/seed/`（dev）或 `Deploy/{ns}/seed/`
//!    （release）。以 `seed-manifest.json` 为唯一契约：仅重放 `in_suite=true`
//!    文件、按 `order` 升序、逐文件做目标表存在性门禁（缺失 WARN 跳过）。
//!
//! 边界（refine-seed-execution-boundary 豁免域 + SECURITY_SPEC §6）：
//! - 只重放种子体系目录内幂等 DML 文件（INSERT ... ON CONFLICT / WHERE NOT EXISTS）。
//! - 凭据类（seed-isahl-user.sh）、e2e/测试数据（in_suite=false）、.sh 形态
//!   步骤不在此处执行——保持脚本链与 remove-tms-seed-fixtures 语义。
//! - 不新增 DDL（uq 索引为 start.sh 4b 既有形态的进程内对齐，应用运行时行为）。
//!
//! 语义：幂等，单文件失败 WARN 不阻断启动（与 Deploy start.sh 4b/4c 对齐）。

use std::collections::HashMap;
use std::path::PathBuf;

use sqlx::{AssertSqlSafe, PgPool};

/// 启动种子自检统计
#[derive(Debug, Default, Clone, Copy)]
pub struct StartupSeedStats {
    pub model_loaded: usize,
    pub model_skipped: usize,
    pub model_failed: usize,
    pub ns_loaded: usize,
    pub ns_skipped: usize,
    pub ns_failed: usize,
}

impl StartupSeedStats {
    fn log(self) {
        common::telemetry::info!(
            "启动种子自愈完成：模型级 载入 {} 跳过 {} 失败 {}；namespace 级 载入 {} 跳过 {} 失败 {}",
            self.model_loaded,
            self.model_skipped,
            self.model_failed,
            self.ns_loaded,
            self.ns_skipped,
            self.ns_failed,
        );
    }
}

/// seed-manifest.json 契约（与 scripts/ts/build-seed-manifest.ts 产出字段 1:1）
#[derive(serde::Deserialize)]
struct SeedManifest {
    #[allow(dead_code)]
    namespace: String,
    #[allow(dead_code)]
    db: String,
    #[allow(dead_code)]
    has_suite: bool,
    #[allow(dead_code)]
    suite_script: Option<String>,
    files: Vec<SeedManifestFile>,
}

#[derive(serde::Deserialize)]
struct SeedManifestFile {
    file: String,
    order: u64,
    #[allow(dead_code)]
    category: String,
    in_suite: bool,
    #[allow(dead_code)]
    parse_ok: bool,
    /// 目标表（`schema.table`）→ 期望行数
    tables: HashMap<String, u64>,
}

/// 启动种子自愈统一入口：模型级 → namespace 级（业务种子硬编码依赖维度坐标）。
///
/// 任何启动方式都会经过 main.rs 调用本函数；目录缺失/契约缺失 WARN 跳过，
/// 单文件失败不阻断启动（幂等，可下次启动重试）。
pub async fn ensure_startup_seed_self_check(pool: &PgPool) {
    let mut stats = StartupSeedStats::default();
    let (model_dir, ns_dir) = resolve_seed_dirs();

    if let Some(dir) = model_dir {
        replay_model_seeds(pool, &dir, &mut stats).await;
    }
    if let Some(dir) = ns_dir {
        replay_ns_seeds(pool, &dir, &mut stats).await;
    }

    stats.log();
}

/// 解析模型级与 namespace 级种子目录（dev/release 双态）。
///
/// - `DEPLOY_PATH` 已设置且存在 → release：`{DEPLOY_PATH}/seed/model-seed` +
///   `{DEPLOY_PATH}/seed`（release-to-namespace.sh 生成软链与种子落包）。
/// - 否则 → dev：项目根 `Framework/seed` + `Pre-Proc/{NAMESPACE}/seed`
///   （CARGO_MANIFEST_DIR 为 `Gateway/backend`，上溯两级到仓库根）。
///
/// 返回的路径不做存在性校验——由调用方 WARN 跳过（零兼容性降级：禁止
/// glob/目录扫描回退，ENVIRONMENT_SPEC 基线断裂 fail-loud 原则的启动侧
/// 语义 = 缺失即跳过并告警，与 load_extensions 一致）。
fn resolve_seed_dirs() -> (Option<PathBuf>, Option<PathBuf>) {
    let deploy_path = std::env::var("DEPLOY_PATH")
        .ok()
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.exists());

    if let Some(dp) = deploy_path {
        let model = dp.join("seed").join("model-seed");
        let ns = dp.join("seed");
        return (Some(model), Some(ns));
    }

    // dev：仓库根探测（编译期 CARGO_MANIFEST_DIR = Gateway/backend，上溯两级）
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let model = root.join("Framework").join("seed");
    let ns = root.join("Pre-Proc").join(ns_env()).join("seed");
    (Some(model), Some(ns))
}

fn ns_env() -> String {
    std::env::var("NAMESPACE").unwrap_or_default()
}

/// 模型级种子重放：uq_seed_id_* 唯一索引 + seed-*.sql 字典序幂等重放。
async fn replay_model_seeds(pool: &PgPool, dir: &PathBuf, stats: &mut StartupSeedStats) {
    if !dir.is_dir() {
        common::telemetry::warn!(
            "模型级种子目录缺失，跳过自动载入（{}；部署包未携带 Framework/seed 软链时属预期）",
            dir.display()
        );
        stats.model_skipped += 1;
        return;
    }

    // 1. 按 meta.json 键建 uq_seed_id_* 唯一索引（对齐 start.sh 4b；
    //    存量重复行库上建索引必然失败——忽略，不阻断启动）
    let meta_path = dir.join("seed-dimensions.meta.json");
    if meta_path.is_file() {
        match std::fs::read_to_string(&meta_path).ok().and_then(|s| {
            serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&s).ok()
        }) {
            Some(keys) => {
                let mut created = 0usize;
                for table in keys.keys() {
                    let sql = format!(
                        "CREATE UNIQUE INDEX IF NOT EXISTS \"uq_seed_id_{}\" ON ONLY isahl.\"{}\" (id)",
                        quote_ident(table),
                        quote_ident(table)
                    );
                    if sqlx::query(AssertSqlSafe(sql.as_str()))
                        .execute(pool)
                        .await
                        .is_ok()
                    {
                        created += 1;
                    }
                }
                common::telemetry::info!(
                    "模型级种子唯一索引就绪：{} 张表（失败已忽略，存量重复行库属预期）",
                    created
                );
            }
            None => {
                common::telemetry::warn!(
                    "seed-dimensions.meta.json 不可读/解析失败，跳过唯一索引建设（重放仍继续）"
                );
            }
        }
    }

    // 2. seed-*.sql 字典序重放（幂等 ON CONFLICT，失败 WARN 不阻断）
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("seed-") && n.ends_with(".sql"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();

    for path in &files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        match replay_sql_file(pool, path).await {
            Ok(()) => {
                common::telemetry::info!("模型级种子载入：{}", name);
                stats.model_loaded += 1;
            }
            Err(e) => {
                common::telemetry::warn!(
                    "模型级种子载入失败（幂等，下次启动重试）：{} — {}",
                    name,
                    e
                );
                stats.model_failed += 1;
            }
        }
    }
}

/// namespace 级种子重放：seed-manifest.json 驱动（order 升序 / in_suite=true /
/// 目标表存在性门禁）。
async fn replay_ns_seeds(pool: &PgPool, dir: &PathBuf, stats: &mut StartupSeedStats) {
    if !dir.is_dir() {
        common::telemetry::warn!(
            "namespace 级种子目录缺失，跳过自动载入（{}）",
            dir.display()
        );
        stats.ns_skipped += 1;
        return;
    }

    let manifest_path = dir.join("seed-manifest.json");
    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(_) => {
            common::telemetry::warn!(
                "seed-manifest.json 缺失（{}），跳过 namespace 级种子自动载入（种子契约漂移，请重跑 build-seed-manifest.ts）",
                manifest_path.display()
            );
            stats.ns_skipped += 1;
            return;
        }
    };
    let manifest: SeedManifest = match serde_json::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            common::telemetry::warn!(
                "seed-manifest.json 解析失败（{}）：{}，跳过 namespace 级种子自动载入",
                manifest_path.display(),
                e
            );
            stats.ns_skipped += 1;
            return;
        }
    };

    let mut suite: Vec<SeedManifestFile> = manifest
        .files
        .into_iter()
        .filter(|f| f.in_suite && f.parse_ok)
        .collect();
    suite.sort_by_key(|f| f.order);

    for file in suite {
        let path = dir.join(&file.file);
        // psql 变量注入检测（:'name' 语法）：依赖 psql -v 注入（如 AVIC
        // seed-avic-owner.sql 的 :'isahl_id'），进程内 raw_sql 无法提供变量，
        // 归脚本链（seed-{ns}-all.sh / start.sh）执行。
        if let Ok(content) = std::fs::read_to_string(&path) {
            if content.contains(":'") {
                common::telemetry::warn!(
                    "namespace 种子跳过：{}（含 psql 变量注入 :'var'，归脚本链执行）",
                    file.file
                );
                stats.ns_skipped += 1;
                continue;
            }
        }
        // 目标表存在性门禁（对齐 seed-{ns}-all.sh 的 table_exists skip 语义）
        if let Some(missing) = missing_table(pool, &file.tables).await {
            common::telemetry::warn!(
                "namespace 种子跳过：{}（目标表缺失：{}）",
                file.file,
                missing
            );
            stats.ns_skipped += 1;
            continue;
        }
        match replay_sql_file(pool, &path).await {
            Ok(()) => {
                common::telemetry::info!("namespace 种子载入：{}", file.file);
                stats.ns_loaded += 1;
            }
            Err(e) => {
                common::telemetry::warn!(
                    "namespace 种子载入失败（幂等，下次启动重试）：{} — {}",
                    file.file,
                    e
                );
                stats.ns_failed += 1;
            }
        }
    }
}

/// 整文件幂等重放：剥除 psql meta-command 行后以 raw_sql 执行
/// （PostgreSQL 原生解析注释/分号/引号，禁止 split(';')——
/// namespace_schema.rs 迁移执行同款设施）。
async fn replay_sql_file(pool: &PgPool, path: &PathBuf) -> Result<(), String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {}", e))?;
    let stripped = strip_psql_meta_commands(&content);
    if stripped.trim().is_empty() {
        return Ok(());
    }
    // 固定连接执行：失败后需在同一连接上清理事务状态。
    // 池 execute 走随机连接，若文件含显式 BEGIN 且失败，aborted 事务会随脏连接回池，
    // 后续文件报 25P02「当前事务被终止」（seed-wz-unit-geo 实测）——必须同连接 ROLLBACK。
    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| format!("获取连接失败: {}", e))?;
    if let Err(e) = sqlx::raw_sql(AssertSqlSafe(stripped.as_str()))
        .execute(&mut *conn)
        .await
    {
        // 失败后补发 ROLLBACK 清理（无事务时仅为 NOTICE，无害）。
        let _ = sqlx::raw_sql("ROLLBACK").execute(&mut *conn).await;
        return Err(format!("执行失败: {}", e));
    }
    Ok(())
}

/// 剥除 pg_dump 产物中的 `\restrict` / `\unrestrict` 行（psql meta-command，
/// sqlx 不识别；对齐 start.sh 4b 的 grep -v 语义）。
fn strip_psql_meta_commands(sql: &str) -> String {
    sql.lines()
        .filter(|line| {
            let t = line.trim_start();
            !(t.starts_with("\\restrict") || t.starts_with("\\unrestrict"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 逐表检查 information_schema 存在性；返回第一个缺失的 `schema.table`。
async fn missing_table(pool: &PgPool, tables: &HashMap<String, u64>) -> Option<String> {
    for table in tables.keys() {
        let (schema, name) = match table.split_once('.') {
            Some((s, n)) => (s, n),
            None => ("isahl", table.as_str()),
        };
        let exists: Result<i32, _> = sqlx::query_scalar(
            "SELECT 1 FROM information_schema.tables \
             WHERE table_schema = $1 AND table_name = $2 LIMIT 1",
        )
        .bind(schema)
        .bind(name)
        .fetch_one(pool)
        .await;
        match exists {
            Ok(1) => continue,
            _ => return Some(table.clone()),
        }
    }
    None
}

/// 标识符加双引号（表名含连字符；内嵌引号翻倍转义）。
fn quote_ident(s: &str) -> String {
    s.replace('"', "\"\"")
}
