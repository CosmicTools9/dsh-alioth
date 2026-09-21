//! age — Apache AGE 三态准入（AGE 读路径守护面）
//!
//! 设计正本：`openspec/specs/age-graph-projection/spec.md`（explicit-degradation /
//! shared-cypher-client）、`openspec/specs/knowledge-graph-cypher/spec.md`。
//!
//! ## AGE 语义实测基线（2026-09-20，PG 18.6 + AGE 1.8.0，全库核对）
//!
//! - **search_path 强依赖**：AGE 生成的执行计划用未限定名解析运算符
//!   （`graphid = graphid`、`agtype @> agtype` 等）——MUST 在含 `ag_catalog` 的
//!   search_path 下执行（曾误诊为「C 层损坏」，zh 错误消息参数序颠倒加剧误判）。
//! - **查询串必须是 dollar-quoted 字面量**（单引号字面量报「a dollar-quoted
//!   string constant is expected」）——客户端以 `$$ ... $$` 包裹。
//! - **第三参必须是裸 `Param` 节点，且其声明类型恰为 `ag_catalog.agtype`**
//!   （AGE `src/backend/parser/cypher_analyze.c` 仅做 `IsA(arg3, Param)`，任何 cast
//!   包装都被拒）；sqlx 只能把绑定参数声明为 text ⇒ 参数 MUST 经 `isahl_graph`
//!   包装器转交（外层 text 入参 → 内层 `$1` 声明类型恰为 agtype、无 Coercion 节点）。
//! - **参数以 `$key` 直接引用**（第三参 map 的键即参数名；`$map.key` 形式静默得 null）。
//! - `pg_dump --exclude-schema=ag_catalog`：restore 后 `ag_graph` 注册表丢失——
//!   图引导/重建由 change `add-age-graph-projection` A-1b 迁移负责。
//! - 包装器由本模块幂等 ensure（工程 schema `isahl_graph`，不在 `schema:isahl` 冻结面）：
//!   消费方含 Gateway / SSO / ns Service / 测试，无共享迁移运行器，故归属客户端单一实现。
//!
//! ## 契约
//!
//! - [`age_status`] 三态探测是全部 AGE 读路径的**唯一准入**：扩展缺失 / 扩展版本 < `1.8.0` /
//!   图未注册 / 包装器不可用 / canary 往返失败 → 调用方 MUST 走 SQL 等价降级实现
//!   （[`try_cypher_json`] 已封装，降级 = `Ok(None)` + `warn` 日志）。canary MUST 经包装器
//!   往返 ⇒ `Usable` 等价于**真实读路径可执行**。**禁止「扩展存在即视为可用」**——幽灵模块
//!   `query_router` 的空图静默空结果缺陷即源于此（change proposal §Why #3）。
//! - 运行时值（id 等）MUST 以绑定参数传递（经包装器转入 AGE），MUST NOT 拼接进
//!   Cypher 文本；图名 MUST 为简单标识符（本模块 fail-fast 校验）。
//! - 查询单列契约：Cypher 查询 MUST RETURN 单个 agtype 值/行（map/list 均可），
//!   服务端 `to_jsonb(r)` 解码为 JSON。
//! - 执行时本客户端自动开事务并 `SET LOCAL search_path = ag_catalog, pg_temp`
//!   （运算符解析前置条件），事务结束自动还原。
//! - **VLE 无 `|` 边类型联合**（`-[:A|B*1..2]->` 报 `syntax error 在 "|" 或附近`）
//!   → 多类型路径 MUST 按边类型分支 UNION（故混合类型链仅 SQL 等价实现可达）。

use log::warn;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// AGE 可用性三态（探测顺序：扩展 → 图注册 → canary 往返）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgeStatus {
    /// 全链路可用（canary cypher 往返成功）——唯一允许执行 Cypher 的状态。
    Usable,
    /// 扩展未安装（部署镜像缺 age）。
    ExtensionMissing,
    /// 扩展在、图未注册或注册悬挂（`pg_dump --exclude-schema=ag_catalog` 致 restore 后注册丢失；
    /// 或 schema 重建后旧注册行存活而 label 关系已消失）。
    GraphMissing,
    /// canary 执行失败（C 层损坏 / 权限 / 超时）——详情进日志。
    NotUsable(String),
}

/// canary 与常规查询的 tokio 级超时（服务端 statement_timeout 由调用方会话策略兜底）。
const CYPHER_TIMEOUT: Duration = Duration::from_secs(10);

/// NGAC 策略图投影的图名（`isahl_auth` 既有占位图，change design D1/D2）。
pub const NGAC_GRAPH: &str = "isahl_auth";

/// 知识图谱投影图名（change extend-knowledge-graph-cypher-backend B-0）。
pub const KNOWLEDGE_GRAPH: &str = "isahl_knowledge";

/// 全部 Cypher 读路径的唯一 SQL 形态：图名 / 查询文本 / 参数 map 三个 text 绑定。
///
/// 包装器（工程 schema `isahl_graph`，非 `isahl`/`isahl_meta` 冻结面，见 change design D2）
/// 负责把参数转成 AGE 要求的 agtype 第三参（AGE 解析期只接受裸 agtype `Param`），并把每行
/// 包成 `{"v": row}` 信封（规避 agtype 标量直转 jsonb 失败）。schema/函数名与本常量的一致性
/// 由单测锚定（`cypher_sql_is_single_wrapper_shape_with_three_binds`）。
const CYPHER_SQL: &str = "SELECT j FROM isahl_graph.cypher_json($1, $2, $3) AS j";

/// 包装器 ensure DDL：幂等（已存在即跳过），AGE 缺失环境下也无副作用。
///
/// 内层语句经 `EXECUTE … USING (params)::ag_catalog.agtype` 构造 ⇒ 内层 `$1` 的声明
/// 类型恰为 agtype（无 Coercion 节点，AGE 的 `IsA(arg3, Param)` 通过），外层入参保持
/// text（sqlx 可绑定）。图名经 `%L` 内联（AGE 要求 `Const`），查询文本经 dollar 标签
/// `$age_cypher$` 内联（标签冲突显式拒绝）。
const CYPHER_WRAPPER_ENSURE_SQL: &str = r#"
DO $ensure$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_proc p
          JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname = 'isahl_graph' AND p.proname = 'cypher_json'
    ) THEN
        CREATE SCHEMA IF NOT EXISTS isahl_graph;
        CREATE OR REPLACE FUNCTION isahl_graph.cypher_json(
            graph_name name, query_string text, params text
        ) RETURNS SETOF jsonb
        LANGUAGE plpgsql STABLE
        AS $fn$
        BEGIN
            IF position('$age_cypher$' IN query_string) > 0 THEN
                RAISE EXCEPTION 'Cypher 文本包含包装器 dollar 标签';
            END IF;
            RETURN QUERY EXECUTE format(
                'SELECT to_jsonb(ag_catalog.agtype_build_map(''v'', r)) AS j FROM ag_catalog.cypher(%L, $age_cypher$%s$age_cypher$, $1) AS (r ag_catalog.agtype)',
                graph_name, query_string)
            USING (params)::ag_catalog.agtype;
        END
        $fn$;
    END IF;
END
$ensure$
"#;

/// AGE 最低版本下限（`agtype_to_jsonb` 是投影对账函数 `age_*_projection_diff` 的依赖；
/// 1.7.x 上该函数因 `check_function_bodies` 失败而**静默缺失**）。
const AGE_MIN_VERSION: &str = "1.8.0";

/// 版本下限判定：按**数值段**比较前两段（`x.y`），忽略预发布后缀（`1.8.0-beta` 视为 1.8）。
fn age_version_supported(version: &str) -> bool {
    fn pair(v: &str) -> (u32, u32) {
        let mut parts = v.split('.').map(|s| {
            s.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .unwrap_or(0)
        });
        (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
    }
    pair(version) >= pair(AGE_MIN_VERSION)
}

/// 图名合法性：简单标识符（包装器内 `%L` 前的 fail-fast 注入面校验）。
fn valid_graph_name(graph: &str) -> bool {
    !graph.is_empty()
        && graph.len() <= 63
        && graph
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && graph.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 进程内 ensure 缓存（键 = 目标库名；同进程可能连多库：测试与多 ns 运行期共用）。
static WRAPPER_ENSURED: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// 幂等 ensure 包装器（每进程每库一次，稳态零 DDL）。
///
/// 失败向上抛错，由 [`age_status`] 收敛为 `NotUsable`：包装器不可用时 AGE 读路径 MUST
/// 显式降级，MUST NOT 以「扩展在册」冒充可用。
async fn ensure_cypher_wrapper(pool: &PgPool) -> Result<(), sqlx::Error> {
    let db = pool
        .connect_options()
        .get_database()
        .unwrap_or_default()
        .to_string();
    if WRAPPER_ENSURED
        .lock()
        .map(|cache| cache.contains(&db))
        .unwrap_or(false)
    {
        return Ok(());
    }
    sqlx::raw_sql(CYPHER_WRAPPER_ENSURE_SQL)
        .execute(pool)
        .await?;
    if let Ok(mut cache) = WRAPPER_ENSURED.lock() {
        cache.insert(db);
    }
    Ok(())
}

/// 三态可用性探测。
///
/// 每个探测步骤的 SQL 错误（权限/超时）收敛为 [`AgeStatus::NotUsable`]，不向调用方
/// 抛错——探测本身属于降级判定的守卫面，不是业务路径。
pub async fn age_status(pool: &PgPool, graph: &str) -> AgeStatus {
    if !valid_graph_name(graph) {
        return AgeStatus::NotUsable(format!("非法图名：{graph:?}"));
    }
    let ext: Result<Option<String>, sqlx::Error> =
        sqlx::query_scalar("SELECT extversion FROM pg_extension WHERE extname = 'age'")
            .fetch_optional(pool)
            .await;
    let ext_version = match ext {
        Ok(Some(v)) => v,
        Ok(None) => return AgeStatus::ExtensionMissing,
        Err(e) => return AgeStatus::NotUsable(format!("扩展探测失败：{e}")),
    };
    if !age_version_supported(&ext_version) {
        return AgeStatus::NotUsable(format!(
            "AGE 版本过低：{ext_version} < {AGE_MIN_VERSION}（缺 agtype_to_jsonb ⇒ 投影对账函数不可建）"
        ));
    }
    // 注册探测 MUST 同时要求该图的已注册 label 关系真实存在：schema 重建（DROP SCHEMA）不触碰
    // ag_catalog，旧注册行会存活而 label 关系消失；此时若判「已注册」，随后任意触及该 label 的
    // Cypher 会落进 AGE C 层的段错误路径——**服务端 crash 无法被本层的错误处理捕获**
    // （2026-09-20 实测：整个 PG 实例 crash + recovery）。关系存在性用 OID 比对。
    let registered: Result<bool, sqlx::Error> = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM ag_catalog.ag_graph WHERE name = $1) \
         AND NOT EXISTS ( \
             SELECT 1 FROM ag_catalog.ag_label l \
              JOIN ag_catalog.ag_graph g ON g.graphid = l.graph \
              WHERE g.name = $1 \
                AND NOT EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = l.relation))",
    )
    .bind(graph)
    .fetch_one(pool)
    .await;
    match registered {
        Ok(true) => {}
        Ok(false) => return AgeStatus::GraphMissing,
        Err(e) => return AgeStatus::NotUsable(format!("图注册探测失败：{e}")),
    }
    // 包装器：参数通道的唯一落地形态，缺失即幂等 ensure；失败不得判可用。
    if let Err(e) = ensure_cypher_wrapper(pool).await {
        return AgeStatus::NotUsable(format!("包装器 ensure 失败：{e}"));
    }
    // canary 经**真实读路径**（包装器）往返 ⇒ Usable 等价于「真实查询可执行」，
    // 而非「扩展在册」（本模块模块头为何禁止后者）。图名/查询经绑定参数传入。
    let canary = tokio::time::timeout(
        CYPHER_TIMEOUT,
        sqlx::query_scalar::<_, Value>(CYPHER_SQL)
            .bind(graph)
            .bind("RETURN {ok: 1}")
            .bind("{}")
            .fetch_one(pool),
    )
    .await;
    match canary {
        Ok(Ok(_)) => AgeStatus::Usable,
        Ok(Err(e)) => AgeStatus::NotUsable(format!("canary 执行失败：{e}")),
        Err(_) => AgeStatus::NotUsable("canary 超时".into()),
    }
}

/// 显式降级的 Cypher 读：AGE 不可用 / 执行异常 → `Ok(None)`（+ warn 日志），
/// 调用方走 SQL 等价实现；可用且成功 → `Ok(Some(rows))`。
///
/// `params` 为 Cypher `$key` 参数的 JSON map（MUST NOT 拼接进查询文本）；`None` 与
/// `{}` 等价。查询文本经包装器内联进 AGE 调用，守卫（dollar 标签冲突）在包装器侧。
pub async fn try_cypher_json(
    pool: &PgPool,
    graph: &str,
    query: &str,
    params: Option<&Value>,
) -> Result<Option<Vec<Value>>, sqlx::Error> {
    match age_status(pool, graph).await {
        AgeStatus::Usable => {}
        status => {
            warn!("[age] 降级 SQL 路径：graph={graph} 状态={status:?}");
            return Ok(None);
        }
    }
    let params_json = params.map(Value::to_string).unwrap_or_else(|| "{}".into());
    // 事务内 SET LOCAL search_path：AGE 执行计划的运算符解析前置条件（模块头基线）。
    // SQL 为编译期常量（[`CYPHER_SQL`]），三个运行时值全走绑定参数。
    let fut = async move {
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(e) => return Err(e),
        };
        sqlx::query("SET LOCAL search_path TO ag_catalog, pg_temp")
            .execute(&mut *tx)
            .await?;
        let rows = sqlx::query_scalar::<_, Value>(CYPHER_SQL)
            .bind(graph)
            .bind(query)
            .bind(params_json)
            .fetch_all(&mut *tx)
            .await;
        // 提交/回滚仅收尾事务（只读语义）；失败不掩盖查询结果
        let _ = tx.commit().await;
        rows
    };
    match tokio::time::timeout(CYPHER_TIMEOUT, fut).await {
        // 解包 agtype_build_map('v', r) 信封（规避 agtype 标量直转 jsonb 失败）
        Ok(Ok(rows)) => Ok(Some(
            rows.into_iter()
                .map(|j| j.get("v").cloned().unwrap_or(j))
                .collect(),
        )),
        Ok(Err(e)) => {
            warn!("[age] 降级 SQL 路径：graph={graph} Cypher 执行失败：{e}");
            Ok(None)
        }
        Err(_) => {
            warn!("[age] 降级 SQL 路径：graph={graph} Cypher 超时（{CYPHER_TIMEOUT:?}）");
            Ok(None)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_name_validation() {
        assert!(valid_graph_name("isahl_auth"));
        assert!(valid_graph_name("g"));
        assert!(!valid_graph_name("")); // 空
        assert!(!valid_graph_name("1abc")); // 数字开头
        assert!(!valid_graph_name("a-b")); // 连字符
        assert!(!valid_graph_name("x'; DROP")); // 注入面
        assert!(!valid_graph_name(&"x".repeat(64))); // 超 63
    }

    #[test]
    fn age_version_floor_is_one_eight() {
        assert!(age_version_supported("1.8.0"));
        assert!(age_version_supported("1.10")); // 数值段比较，非字典序
        assert!(age_version_supported("2.0.0"));
        assert!(age_version_supported("1.8.0-beta")); // 预发布后缀忽略
        assert!(!age_version_supported("1.7.0"));
        assert!(!age_version_supported("0.9"));
        assert!(!age_version_supported(""));
    }

    #[test]
    fn cypher_sql_is_single_wrapper_shape_with_three_binds() {
        // 读路径唯一形态：AGE 调用只出现在包装器内部，三个运行时值全为绑定参数。
        assert!(CYPHER_SQL.contains("isahl_graph.cypher_json($1, $2, $3)"));
        assert!(!CYPHER_SQL.contains("ag_catalog.cypher"));
        // 包装器 schema/函数名与 ensure DDL 一致（单一来源在 SQL 文本，无冗余常量）
        assert!(CYPHER_WRAPPER_ENSURE_SQL.contains("'isahl_graph'"));
        assert!(CYPHER_WRAPPER_ENSURE_SQL.contains("'cypher_json'"));
    }

    #[test]
    fn wrapper_ddl_keeps_agtype_cast_and_tag_guard() {
        // 通道成立的充分条件：内层 `$1` 声明类型恰为 agtype（USING 表达式类型）；
        // 标签守卫与查询文本内联（AGE 要求 arg2 为 dollar-quoted 常量）。
        assert!(CYPHER_WRAPPER_ENSURE_SQL.contains("CREATE SCHEMA IF NOT EXISTS isahl_graph"));
        assert!(CYPHER_WRAPPER_ENSURE_SQL.contains("USING (params)::ag_catalog.agtype"));
        assert!(CYPHER_WRAPPER_ENSURE_SQL.contains("position('$age_cypher$' IN query_string)"));
        assert!(CYPHER_WRAPPER_ENSURE_SQL.contains("$age_cypher$%s$age_cypher$"));
        assert!(CYPHER_WRAPPER_ENSURE_SQL.contains("RETURNS SETOF jsonb"));
    }

    /// 集成：AGE 1.8.0 + 图引导落地后的现况锚定（change A-1b）。
    ///
    /// test 库经 037 迁移注册 `isahl_auth` 图 + label 全集 → canary 往返成功 = Usable。
    /// 图引导被移除/未跑该迁移的环境会回退 GraphMissing——本断言同时守住
    /// 「扩展在即视为可用」的误判面（canary 不通过就不判 Usable）。
    #[tokio::test]
    async fn age_status_tri_state_contract() {
        let pool = crate::testing::connect_test_db().await;
        let status = age_status(&pool, NGAC_GRAPH).await;
        assert_eq!(status, AgeStatus::Usable);
    }

    /// 集成：真实 Cypher 往返（参数化 + 事务内 search_path）与降级契约双向锚定。
    #[tokio::test]
    async fn try_cypher_json_roundtrip_and_degradation() {
        let pool = crate::testing::connect_test_db().await;
        // 往返：NGAC 投影图上的顶点计数（A-1b 重建后的实况）
        let out = try_cypher_json(&pool, NGAC_GRAPH, "MATCH (n) RETURN count(n)", None)
            .await
            .expect("可用路径不应向调用方抛错");
        assert!(out.is_some(), "Usable 状态下 MUST 走 AGE 路径返回 Some");

        // 参数通道：绑定参数经包装器转入 agtype 第三参（修复前该路径恒降级）
        let out = try_cypher_json(
            &pool,
            NGAC_GRAPH,
            "RETURN {given: $probe}",
            Some(&serde_json::json!({"probe": "ok"})),
        )
        .await
        .expect("可用路径不应向调用方抛错")
        .expect("参数通道可用时 MUST 返回 Some");
        assert_eq!(out[0]["given"], serde_json::json!("ok"));

        // 包装器自动就位：首次调用后即可见（调用方无需 DB 侧前置动作）
        let wrapper: bool = sqlx::query_scalar(
            "SELECT to_regprocedure('isahl_graph.cypher_json(name,text,text)') IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .expect("包装器存在性查询失败");
        assert!(wrapper, "首次调用后包装器 MUST 已 ensure");

        // 守卫：查询文本含包装器 dollar 标签 → 包装器拒绝 ⇒ 显式降级
        let out = try_cypher_json(&pool, NGAC_GRAPH, "RETURN {x: 1} // $age_cypher$", None)
            .await
            .expect("降级路径不应向调用方抛错");
        assert_eq!(out, None);

        // 降级：非法图名（注入面校验拒绝）→ Ok(None)
        let out = try_cypher_json(&pool, "x'; DROP", "MATCH (n) RETURN count(n)", None)
            .await
            .expect("降级路径不应向调用方抛错");
        assert_eq!(out, None);

        // 降级：未注册图 → Ok(None)（禁止静默空结果冒充业务答案）
        let out = try_cypher_json(&pool, "no_such_graph", "MATCH (n) RETURN count(n)", None)
            .await
            .expect("降级路径不应向调用方抛错");
        assert_eq!(out, None);
        assert_eq!(age_status(&pool, KNOWLEDGE_GRAPH).await, AgeStatus::Usable);
    }
}
