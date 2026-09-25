//! 跨 crate 复用的 SQL 片段（编译期拼接，零运行期分配）。

/// 叶表名表达式（SQL 片段）：`pg_class.relname`——与连接 `search_path` 无关的裸表名。
///
/// MUST NOT 用 `tableoid::regclass::text`：该文本随会话 `search_path` 渲染为
/// `zc_id_empl-natural` 或 `isahl."zc_id_empl-natural"`（连字符表名带引号），
/// 令分类判定 / 白名单匹配 / 叶表归属 / 维度派生在**同一进程的不同连接**上给出不同结果
/// （实测：同参请求交替返回全量 / 空集；分支码匹配时好时坏）。
/// 同款先例：`Framework/backend/approval/src/handlers/flow_lifecycle.rs`（branch 取 relname）。
///
/// **别名必填**：`pg_class` 自身也有 `tableoid` 列，子查询内不限定会解析到内层 `pg_class`，
/// 退化为「恒真/取任意行」——调用方 MUST 给被探测表显式别名（含 `INSERT INTO t AS e … RETURNING`
/// 形态），或按 `$col` 传入承载 oid 的子查询列。
///
/// 用法：
///
/// ```ignore
/// concat!("SELECT t.id, ", common::leaf_relname!(t), " AS leaf FROM isahl.tab t")
/// concat!("SELECT c.id, ", common::leaf_relname!(c, kind_oid), " AS leaf FROM (…) c")
/// ```
///
/// 调用点（本宏为唯一实现）：`identity-org` 主体列表/单条与证照 `kind`、`approval` 流程
/// `branch`/`context_leaf`/事件类型/范畴与模板叶、`measurement` 单位与换算率 `dimension`。
///
/// 判定口径见 `docs/specs/ALIOTH_ONTOLOGY_SPEC.md`（叶表归属与分类判定）；本宏只产出 SQL
/// 片段，`search_path` 语义由调用方的连接决定，故禁止在调用点另行拼接 `regclass`。
#[macro_export]
macro_rules! leaf_relname {
    ($alias:ident) => {
        concat!(
            "(SELECT relname FROM pg_class WHERE oid = ",
            stringify!($alias),
            ".tableoid)"
        )
    };
    ($alias:ident, $col:ident) => {
        concat!(
            "(SELECT relname FROM pg_class WHERE oid = ",
            stringify!($alias),
            ".",
            stringify!($col),
            ")"
        )
    };
}

/// 「**非**占位/系统主体」谓词（SQL 片段，编译期拼接，零运行期分配）。
///
/// 判据与 [`crate::actor_identity::normalize_business_subject`] **同源、单一实现**：
/// `code` ∈ {`SUBJ-ISAH-ADMIN`（种子回退的 ns 占位法人）, `POS-SYSTEM-ADMIN`（系统管理员岗位）,
/// `SUBJ-SYSTEM`（系统平台主体）} 或 `btrim(notice) = 'isahl 管理员'` 的主体即占位/系统主体。
///
/// 这类主体由种子/账号回退产生，MUST NOT 作业务主体（合同方、委托 B、产品属权、当事人「我」槽位
/// ——见 `WZ_EXTERNAL_CHAIN_SPEC` §7，2026-09-14 裁决），故 MUST NOT 出现在**业务主体候选**中。
/// 批注 51c2730d / aa12d9b9：合同乙方与委托「业务主体」下拉混入「isahl 管理员」「系统平台主体」
/// （二者因岗位视角链携带 `VIEW-BIZ`，任何按视角标签查主体的读径都会带出）。
///
/// 用 `COALESCE(code, '') <> ALL(ARRAY[…])` 而非 `NOT IN`：`code IS NULL` 的行在 `NOT IN` 下求值
/// `NULL` ⇒ 整行被静默丢弃（无 code 主体**不是**占位主体，应保留）；`COALESCE` 归一为 `''` 后
/// `<> ALL` 恒真 ⇒ 该行保留。
///
/// **勿写成 `IS DISTINCT FROM ALL(ARRAY[…])`——那不是合法 PostgreSQL 语法**（`IS DISTINCT FROM`
/// 只接受行构造器作右操作数）。曾按此写法拼接，编译期无感、运行期整个语句语法错误 ⇒
/// `GET /contracts/counterparties` 全量 500、合同甲乙方与委托「业务主体」候选全空（批注 72ceb947）。
///
/// 用法（`$alias` = 声明了 `code`/`notice` 列的主体表别名，MUST 为源码标识符）：
///
/// ```ignore
/// concat!("WHERE s.deleted_at IS NULL AND ", common::not_placeholder_subject!(s))
/// ```
#[macro_export]
macro_rules! not_placeholder_subject {
    ($alias:ident) => {
        concat!(
            "COALESCE(",
            stringify!($alias),
            ".code, '') <> ALL(ARRAY['SUBJ-ISAH-ADMIN','POS-SYSTEM-ADMIN','SUBJ-SYSTEM'])",
            " AND btrim(COALESCE(",
            stringify!($alias),
            ".notice, '')) <> 'isahl 管理员'"
        )
    };
}

/// 「**真实岗位行**」谓词（SQL 片段，编译期拼接，零运行期分配）。
///
/// 判据与 `ALIOTH_ONTOLOGY_SPEC.md §4.3.1` **同源、单一实现**：`zc_id_subj-position` 是
/// `zc_id_lifecycle` 子表，`_f_`（功能阶段）/`_t_`（抽象层级）由 `dk_function.code` 前两字符派生
/// （`trigger-registry::lifecycle::derive_form_type`）。`_t_='范例'` = 公有化属权的编制范例
/// （模板/模式行），其余（`_t_='实例'` 与类列 NULL 的存量行）= 可被业务引用的**具体岗位**。
///
/// MUST NOT 用 `_f_ IS NULL`（或 `_t_ IS NULL`）当「真实岗位」判据：岗位类列非空是模型要求，
/// **类列 NULL 是 §4.3.3 的违规形态 1**（「行存在但不可见」）而非真实岗位的标记；类列归一器
/// （各 ns 种子链 `seed-zz-class-columns-normalize.sql`）按 `dk_function` 前缀把岗位填成
/// `_f_='实现' / _t_='实例'` 后，NULL 判据会把**全部真实岗位静默排除**（2026-09-23 实证：
/// AVIC 审批节点「审批岗位」下拉恒空、组织管理岗位列表 3/13、WZ 1/22 —— 见
/// `openspec/changes/fix-position-class-predicate/`）。
///
/// 用 `IS DISTINCT FROM` 而非 `<>`：类列可空，`NULL <> '范例'` 求值 `NULL` ⇒ 整行被静默丢弃
/// （同 `not_placeholder_subject` 的 `COALESCE` 教训）。
///
/// 用法（`$alias` = 声明了 `_t_` 列的岗位表别名，MUST 为源码标识符）：
///
/// ```ignore
/// concat!("WHERE p.deleted_at IS NULL AND ", common::real_position_row!(p))
/// format!("{} WHERE deleted_at IS NULL AND {}", SELECT, common::real_position_row!())
/// ```
///
/// 调用点（本宏为唯一实现）：`authority` 审批岗位列表 / 审批人（Approver）仓储三径、
/// `identity-org` 岗位列表/计数/详情与岗位存在性/更新/软删守卫、主体视角挂接的岗位校验、
/// `common::ngac_org` 岗位类别 UA 派生与存量 UA 迁移。判据变更 MUST 先改规约
/// （`openspec/specs/identity-org-core` 的 `real-position-class-predicate`）。
#[macro_export]
macro_rules! real_position_row {
    ($alias:ident) => {
        concat!(stringify!($alias), "._t_ IS DISTINCT FROM '范例'")
    };
    () => {
        "_t_ IS DISTINCT FROM '范例'"
    };
}
