//! NGAC 决策核心（唯一实现，openspec change `extract-ngac-decision-core`）。
//!
//! 承载 NGAC 决策语义的全部纯函数件，SSO PDP 端点族（decide/explain/check/batch）
//! 与 Framework 内层门（`common::permissions::require_resource_access`）共同消费：
//!
//! - [`PolicyGraph`]：内存策略图（association / access_right / prohibition / 策略类名索引）；
//! - [`evaluate_pair_in`]：(UA, OA) 对级求值（prohibition 早停 → association 早停 → NotApplicable）；
//! - [`evaluate_merge_in`]：跨对 deny-overrides 全局合并 + admin 遍历后兜底；
//! - [`decide_ordered`]：规范决策序（密级上限 → bootstrap → 合并 → creator 派生翻转）；
//! - [`evaluate_conditions`] / [`ConditionContext`]：conditions v1/v2 求值（失败封闭）。
//!
//! 本 crate MUST 保持纯函数形态（禁止 sqlx/actix/tokio 依赖）：IO 与输入装载
//! （PIP SQL、版本探针、派生授权谓词）归宿主；仓内 MUST NOT 存在第二份
//! 合并循环、决策序编排或 conditions 求值器实现
//! （门禁 `scripts/check/check-ngac-decision-chain-single-source.ts`）。

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ── 决策与轨迹类型 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Permit,
    Deny,
    NotApplicable,
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Decision::Permit => write!(f, "Permit"),
            Decision::Deny => write!(f, "Deny"),
            Decision::NotApplicable => write!(f, "NotApplicable"),
        }
    }
}

/// 单条策略的求值轨迹（`evaluate_pair_in` 返回，供 explain 端点渲染"为什么"）。
#[derive(Debug, Clone, Serialize)]
pub struct MatchedRule {
    /// "association"（允许） | "prohibition"（禁止）
    pub rule_type: String,
    /// "allow" | "deny"
    pub kind: String,
    pub access_rights: Vec<String>,
    pub conditions: Option<serde_json::Value>,
    pub conditions_met: bool,
    /// 本次操作是否命中（命中即决定该对结果）
    pub matched: bool,
}

/// 逐对合并明细（供 explain 完备 steps 与影响预览 before/after 记录）。
#[derive(Debug, Clone)]
pub struct PairOutcome {
    pub ua_id: i64,
    pub oa_id: i64,
    pub decision: Decision,
    pub rules: Vec<MatchedRule>,
}

// ── 策略图数据类型（纯数据；DB 行装载归宿主，宿主 FromRow DTO 转入本类型） ──────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Association {
    pub id: i64, // id-json-ok: 决策核心叶 crate；common 依赖本 crate，引 common::serde_zuid 成环，zuid 约定宿主在 common
    pub fk_user_attribute: i64, // id-json-ok: 同上（叶 crate 环约束）
    pub fk_object_attribute: i64, // id-json-ok: 同上（叶 crate 环约束）
    pub ak_access_rights: Vec<i64>, // id-json-ok: 同上（叶 crate 环约束）
    #[serde(default)]
    pub conditions: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessRight {
    pub id: i64, // id-json-ok: 决策核心叶 crate；common 依赖本 crate，引 common::serde_zuid 成环，zuid 约定宿主在 common
    pub o_name: String,
    #[serde(default)]
    pub applicable_types: Vec<String>,
    #[serde(default)]
    pub is_system: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prohibition {
    pub id: i64, // id-json-ok: 决策核心叶 crate；common 依赖本 crate，引 common::serde_zuid 成环，zuid 约定宿主在 common
    #[serde(default)]
    pub o_name: String,
    pub fk_user_attribute: i64,     // id-json-ok: 同上（叶 crate 环约束）
    pub fk_object_attribute: i64,   // id-json-ok: 同上（叶 crate 环约束）
    pub ak_access_rights: Vec<i64>, // id-json-ok: 同上（叶 crate 环约束）
    pub is_active: bool,
    #[serde(default)]
    pub conditions: Option<Value>,
}

// ── 策略图 ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct PolicyGraph {
    /// 只读遍历面（矩阵/影响预览等投影消费方直接迭代；写入口仅 `add_*`/`remove_*`）。
    pub associations: DashMap<i64, Association>,
    pub access_rights: DashMap<i64, AccessRight>,
    pub prohibitions: DashMap<i64, Prohibition>,
    pub user_attr_index: DashMap<i64, Vec<i64>>,
    pub object_attr_index: DashMap<i64, Vec<i64>>,
    pub access_right_name_index: DashMap<String, i64>,
    pub policy_classes: DashMap<String, i64>,
    /// UA id → 策略类 id（NGAC 分区；求值对级短信用）。
    pub user_attr_policy_class: DashMap<i64, i64>,
    /// OA id → 策略类 id（同上）。
    pub object_attr_policy_class: DashMap<i64, i64>,
}

impl PolicyGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_associations(&self) -> bool {
        !self.associations.is_empty()
    }

    pub fn add_association(&self, assoc: Association) {
        let fk_ua = assoc.fk_user_attribute;
        let fk_oa = assoc.fk_object_attribute;

        self.associations.insert(assoc.id, assoc.clone());

        self.user_attr_index
            .entry(fk_ua)
            .or_default()
            .push(assoc.id);

        self.object_attr_index
            .entry(fk_oa)
            .or_default()
            .push(assoc.id);
    }

    pub fn add_access_right(&self, ar: AccessRight) {
        self.access_right_name_index
            .insert(ar.o_name.clone(), ar.id);
        self.access_rights.insert(ar.id, ar);
    }

    pub fn add_prohibition(&self, proh: Prohibition) {
        self.prohibitions.insert(proh.id, proh);
    }

    /// 登记策略类名（bootstrap 判据需要 default 类存在性；id 仅作登记）。
    pub fn add_policy_class(&self, o_name: &str, id: i64) {
        self.policy_classes.insert(o_name.to_string(), id);
    }

    /// 登记 UA 的策略类（NGAC 分区；缺登记 ⇒ 求值不按分区短路）。
    pub fn add_user_attribute_pc(&self, id: i64, policy_class: i64) {
        self.user_attr_policy_class.insert(id, policy_class);
    }

    /// 登记 OA 的策略类（同上）。
    pub fn add_object_attribute_pc(&self, id: i64, policy_class: i64) {
        self.object_attr_policy_class.insert(id, policy_class);
    }

    /// bootstrap 判据（与 Framework 内层门 Phase C 收窄同语义）：
    /// **从未初始化** = 无 default 策略类 **且** 零 association。只看
    /// `!has_associations()` 会把「策略类已种、association 种子部分失败」的
    /// 半配置窗口误判为未初始化而 fail-open——两个宿主 MUST 用同一判据。
    pub fn bootstrap_empty(&self) -> bool {
        !self.has_associations() && !self.policy_classes.contains_key("default")
    }

    /// 操作名 → access_right id（求值入口解析；未登记返回 `None` ⇒ NotApplicable）。
    pub fn access_right_id(&self, operation: &str) -> Option<i64> {
        self.access_right_name_index.get(operation).map(|v| *v)
    }

    /// 移除一条 association（含双侧索引）。仅供影响预览/dry-run 在**克隆图**上
    /// 使用——运行时图只经 reload 原子换图。
    pub fn remove_association(&self, id: i64) {
        if let Some((_, assoc)) = self.associations.remove(&id) {
            if let Some(mut ids) = self.user_attr_index.get_mut(&assoc.fk_user_attribute) {
                ids.retain(|x| *x != id);
            }
            if let Some(mut ids) = self.object_attr_index.get_mut(&assoc.fk_object_attribute) {
                ids.retain(|x| *x != id);
            }
        }
    }

    /// 移除一条 prohibition（同上，仅预览克隆图用）。
    pub fn remove_prohibition(&self, id: i64) {
        self.prohibitions.remove(&id);
    }
}

// ── conditions 求值（v1 时间窗 + v2 属性闭包；失败封闭） ───────────────────────

/// 条件授权上下文。决策时用于求值 association / prohibition 的 `conditions` JSONB。
///
/// **注入式 Rhai 求值器（`rhai_eval`）**：`conditions` 第三形态 `{"rhai": "<expr>"}` 的求值
/// 归**宿主层**（平台唯一引擎 `runtime-engine::RhaiExpressionEngine`）；决策核心只回调本字段
/// ——核心因此保持**无 IO、无引擎依赖**（注入的是纯谓词，等价于注入比较器），且逐对（per-pair）
/// 求值面（matrix/pip/review/list）与单资源决策面得到**同一语义**。
/// `None`（缺省/未注入）时含 `rhai` 键的条件恒**不满足**（fail-closed）。
pub type RhaiCondEval = std::sync::Arc<dyn Fn(&str, &ConditionContext) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct ConditionContext {
    /// 决策时刻（用于时间窗条件 `not_before` / `not_after`）。
    pub now: DateTime<Utc>,
    /// 决策用户的全部有效 UA 名（含认知派生/委托派生/属权派生），供 `user_attr_in` 求值。
    /// Default 为空集 → `user_attr_in` 永假（fail-closed）。
    pub user_ua_names: Vec<String>,
    /// 目标 OA 祖先闭包名（含自身），供 `object_attr_in` 求值。Default 为空集。
    pub oa_closure_names: Vec<String>,
    /// 宿主注入的 Rhai 条件求值器（`{"rhai": …}` 形态的唯一求值入口；`None` ⇒ fail-closed）。
    pub rhai_eval: Option<RhaiCondEval>,
}

impl std::fmt::Debug for ConditionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConditionContext")
            .field("now", &self.now)
            .field("user_ua_names", &self.user_ua_names)
            .field("oa_closure_names", &self.oa_closure_names)
            .field("rhai_eval", &self.rhai_eval.as_ref().map(|_| "<injected>"))
            .finish()
    }
}

impl Default for ConditionContext {
    fn default() -> Self {
        Self {
            now: Utc::now(),
            user_ua_names: Vec::new(),
            oa_closure_names: Vec::new(),
            rhai_eval: None,
        }
    }
}

/// `conditions` 的合法键集（v1 时间窗 + v2 属性闭包 + v3 `rhai`）。
///
/// **唯一真源**：写径白名单校验（`ngac_host::validate_condition_keys`）与核心求值
/// 同常量——新增条件形态 MUST 同时登记在此，否则写径拒绝、核心 fail-closed。
pub const CONDITION_KEYS: [&str; 5] = [
    "not_before",
    "not_after",
    "user_attr_in",
    "object_attr_in",
    "rhai",
];

/// 求值一条策略的 `conditions` JSONB。
///
/// 语义（与 `docs/specs/NGAC_SPEC.md §2.4` 一致）：
/// - `NULL` / 空对象 `{}` → 无条件生效，返回 `true`；
/// - 任一约束不满足 → 返回 `false`（该策略不参与本次决策）；
/// - 字段非法 / 解析失败 / **含未知键** → 返回 `false`（**失败封闭**，宁拒勿放）。
pub fn evaluate_conditions(conditions: &Option<Value>, ctx: &ConditionContext) -> bool {
    let conditions = match conditions {
        Some(v) if !v.is_null() => v,
        _ => return true,
    };
    let obj = match conditions.as_object() {
        Some(o) if !o.is_empty() => o,
        _ => return true,
    };

    // 未知键 ⇒ 失败封闭（change `wire-org-policy-declaration-fields` tasks 1.5）：
    // 白名单外的键既不被求值、也不能证明「无条件」⇒ 若静默忽略即「写错键名 = 条件被无声丢弃
    // ⇒ 该策略无条件生效」的静默放行面（实测：键名拼写错误此前会放行）。方向与 §2.4 失败封闭一致。
    if obj.keys().any(|k| !CONDITION_KEYS.contains(&k.as_str())) {
        return false;
    }

    // not_before：当前时间须 >= 该时间点
    if let Some(nb) = obj.get("not_before") {
        let nb = match parse_iso8601(nb) {
            Some(t) => t,
            None => return false,
        };
        if ctx.now < nb {
            return false;
        }
    }

    // not_after：当前时间须 <= 该时间点
    if let Some(na) = obj.get("not_after") {
        let na = match parse_iso8601(na) {
            Some(t) => t,
            None => return false,
        };
        if ctx.now > na {
            return false;
        }
    }

    // user_attr_in：用户有效 UA 集须含至少一个给定 UA 名（v2，add-ngac-condition-v2）。
    // 非法字段 / 空上下文 → fail-closed（永假）。
    if let Some(uai) = obj.get("user_attr_in") {
        let names: Vec<&str> = match uai.as_array() {
            Some(a) => a.iter().filter_map(|x| x.as_str()).collect(),
            None => return false,
        };
        if names.is_empty()
            || !names
                .iter()
                .any(|n| ctx.user_ua_names.iter().any(|u| u == n))
        {
            return false;
        }
    }

    // object_attr_in：目标 OA 祖先闭包须含至少一个给定 OA 名（v2）。
    if let Some(oai) = obj.get("object_attr_in") {
        let names: Vec<&str> = match oai.as_array() {
            Some(a) => a.iter().filter_map(|x| x.as_str()).collect(),
            None => return false,
        };
        if names.is_empty()
            || !names
                .iter()
                .any(|n| ctx.oa_closure_names.iter().any(|o| o == n))
        {
            return false;
        }
    }

    // rhai：规则边界条件 / 规范负向模板（`{"rhai": "<expr>"}` 第三形态，change
    // `wire-org-policy-declaration-fields`）。求值归**宿主注入的纯谓词**（`ctx.rhai_eval`）；
    // 未注入 / 谓词返回假 ⇒ 该条**不满足**（fail-closed——MUST NOT 因「未知键被忽略」静默放行）。
    // `rhai` 取 `null` / 空串 ⇒ 视为无条件（与 `{}` 同）。
    if let Some(r) = obj.get("rhai") {
        if let Some(src) = r.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            // 未注入求值器 / 求值为假 ⇒ 不满足（fail-closed）。
            let satisfied = matches!(ctx.rhai_eval.as_ref(), Some(eval) if eval(src, ctx));
            if !satisfied {
                return false;
            }
        }
    }

    true
}

fn parse_iso8601(v: &Value) -> Option<DateTime<Utc>> {
    match v {
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc)),
        _ => None,
    }
}

// ── 对级求值与全局合并 ───────────────────────────────────────────────────────

/// (UA, OA) 对级求值：prohibition 命中（含 conditions 满足）即 Deny（早停）；
/// 否则 association 命中即 Permit（早停）；否则 NotApplicable。
///
/// 对内语义（NGAC_SPEC §2.5）：prohibition 先于 association；返回全部参与求值的
/// 规则轨迹（explain 不早停的要求由 [`evaluate_merge_in`] 的全对遍历承载）。
pub fn evaluate_pair_in(
    pg: &PolicyGraph,
    fk_user_attribute: i64,
    fk_object_attribute: i64,
    operation: &str,
    ctx: &ConditionContext,
) -> (Decision, Vec<MatchedRule>) {
    let ar_id = match pg.access_right_id(operation) {
        Some(id) => id,
        None => return (Decision::NotApplicable, Vec::new()),
    };
    // 策略类分区（NGAC）：跨分区 (UA, OA) 对无策略 ⇒ NotApplicable。两侧皆登记且不等才短路；
    // 缺登记（测试夹具 / dry-run 克隆图补丁节点）⇒ 不短路，维持既有匹配语义。
    if let (Some(ua_pc), Some(oa_pc)) = (
        pg.user_attr_policy_class.get(&fk_user_attribute),
        pg.object_attr_policy_class.get(&fk_object_attribute),
    ) {
        if *ua_pc != *oa_pc {
            return (Decision::NotApplicable, Vec::new());
        }
    }
    let ar_names = |ids: &[i64]| -> Vec<String> {
        ids.iter()
            .filter_map(|id| pg.access_rights.get(id).map(|r| r.o_name.clone()))
            .collect()
    };

    let mut rules: Vec<MatchedRule> = Vec::new();

    for prohibition in pg.prohibitions.iter() {
        if prohibition.is_active
            && prohibition.fk_user_attribute == fk_user_attribute
            && prohibition.fk_object_attribute == fk_object_attribute
        {
            let conditions_met = evaluate_conditions(&prohibition.conditions, ctx);
            let matched = prohibition.ak_access_rights.contains(&ar_id) && conditions_met;
            rules.push(MatchedRule {
                rule_type: "prohibition".to_string(),
                kind: "deny".to_string(),
                access_rights: ar_names(&prohibition.ak_access_rights),
                conditions: prohibition.conditions.clone(),
                conditions_met,
                matched,
            });
            if matched {
                return (Decision::Deny, rules);
            }
        }
    }

    if let Some(assoc_ids) = pg.user_attr_index.get(&fk_user_attribute) {
        for assoc_id in assoc_ids.iter() {
            if let Some(assoc) = pg.associations.get(assoc_id) {
                if assoc.fk_object_attribute == fk_object_attribute {
                    let conditions_met = evaluate_conditions(&assoc.conditions, ctx);
                    let matched = assoc.ak_access_rights.contains(&ar_id) && conditions_met;
                    rules.push(MatchedRule {
                        rule_type: "association".to_string(),
                        kind: "allow".to_string(),
                        access_rights: ar_names(&assoc.ak_access_rights),
                        conditions: assoc.conditions.clone(),
                        conditions_met,
                        matched,
                    });
                    if matched {
                        return (Decision::Permit, rules);
                    }
                }
            }
        }
    }

    (Decision::NotApplicable, rules)
}

/// deny-overrides 全局合并 + admin 遍历后兜底（NGAC_SPEC §2.5）——唯一实现。
///
/// 在给定图快照上遍历 (UA 闭包 × OA 闭包) 全部对：任一对 Deny → 终态 Deny；否则
/// 任一 Permit → Permit；全不适用 → `is_admin` 时 Permit（admin 治理豁免 = 遍历后
/// 兜底——仅在无匹配时生效，显式 prohibition 对 admin 同样生效），否则 NotApplicable。
/// 遍历顺序不影响结果；明细记录全部参与求值的 (UA, OA) 对（不早停）。
pub fn evaluate_merge_in(
    pg: &PolicyGraph,
    ua_closure_ids: &[i64],
    oa_closure_ids: &[i64],
    operation: &str,
    ctx: &ConditionContext,
    is_admin: bool,
) -> (Decision, Vec<PairOutcome>) {
    let mut outcomes: Vec<PairOutcome> = Vec::new();
    let mut saw_permit = false;
    for &ua in ua_closure_ids {
        for &oa in oa_closure_ids {
            let (decision, rules) = evaluate_pair_in(pg, ua, oa, operation, ctx);
            if decision == Decision::Permit {
                saw_permit = true;
            }
            outcomes.push(PairOutcome {
                ua_id: ua,
                oa_id: oa,
                decision,
                rules,
            });
        }
    }
    let saw_deny = outcomes.iter().any(|o| o.decision == Decision::Deny);
    let decision = if saw_deny {
        Decision::Deny
    } else if saw_permit || is_admin {
        Decision::Permit
    } else {
        Decision::NotApplicable
    };
    (decision, outcomes)
}

// ── 索引化求值原语（indexed-evaluation-primitive）──────────────────────────────

/// 索引化策略求值原语：按 `(fk_user_attribute, fk_object_attribute)` 精确对预建索引，
/// 使批量求值（策略矩阵、RLS 列表）从「每对全扫关联」降为「按对命中」。
///
/// 语义 MUST 与 [`evaluate_pair_in`] / [`evaluate_merge_in`] 等价（deny-overrides +
/// admin 遍历后兜底，保留 Deny 短路）。索引在 [`Self::from`] 时自 `PolicyGraph` 构建，
/// 此后只读。消费方 MUST 复用本原语，MUST NOT 自建等价索引与合并循环
/// （门禁 `check-ngac-decision-chain-single-source.ts` P6）。
#[derive(Debug, Clone, Default)]
pub struct IndexedPolicyGraph {
    assoc_index: HashMap<(i64, i64), Vec<Association>>,
    prohib_index: HashMap<(i64, i64), Vec<Prohibition>>,
    access_right_name_index: HashMap<String, i64>,
    access_rights: HashMap<i64, AccessRight>,
    /// UA id → 策略类 id（分区短路，见 `evaluate_pair`）。
    user_attr_policy_class: HashMap<i64, i64>,
    /// OA id → 策略类 id（同上）。
    object_attr_policy_class: HashMap<i64, i64>,
}

impl IndexedPolicyGraph {
    /// 自策略图构建索引。同 (UA, OA) 的规则保序：association 按 `user_attr_index` 的
    /// 关联 id 序（= `evaluate_pair_in` 的遍历序），prohibition 按源图迭代序。
    pub fn from(pg: &PolicyGraph) -> Self {
        let mut assoc_index: HashMap<(i64, i64), Vec<Association>> = HashMap::new();
        for e in pg.user_attr_index.iter() {
            let ua = *e.key();
            for id in e.value().iter() {
                if let Some(a) = pg.associations.get(id) {
                    let a = a.value();
                    assoc_index
                        .entry((ua, a.fk_object_attribute))
                        .or_default()
                        .push(a.clone());
                }
            }
        }
        let mut prohib_index: HashMap<(i64, i64), Vec<Prohibition>> = HashMap::new();
        for e in pg.prohibitions.iter() {
            let p = e.value();
            prohib_index
                .entry((p.fk_user_attribute, p.fk_object_attribute))
                .or_default()
                .push(p.clone());
        }
        let access_right_name_index: HashMap<String, i64> = pg
            .access_right_name_index
            .iter()
            .map(|e| (e.key().clone(), *e.value()))
            .collect();
        let access_rights: HashMap<i64, AccessRight> = pg
            .access_rights
            .iter()
            .map(|e| (*e.key(), e.value().clone()))
            .collect();
        let user_attr_policy_class: HashMap<i64, i64> = pg
            .user_attr_policy_class
            .iter()
            .map(|e| (*e.key(), *e.value()))
            .collect();
        let object_attr_policy_class: HashMap<i64, i64> = pg
            .object_attr_policy_class
            .iter()
            .map(|e| (*e.key(), *e.value()))
            .collect();
        Self {
            assoc_index,
            prohib_index,
            access_right_name_index,
            access_rights,
            user_attr_policy_class,
            object_attr_policy_class,
        }
    }

    /// 操作名 → access_right id（与 [`PolicyGraph::access_right_id`] 同）。
    pub fn access_right_id(&self, operation: &str) -> Option<i64> {
        self.access_right_name_index.get(operation).copied()
    }

    fn ar_names(&self, ids: &[i64]) -> Vec<String> {
        ids.iter()
            .filter_map(|id| self.access_rights.get(id).map(|r| r.o_name.clone()))
            .collect()
    }

    /// (UA, OA) 对级求值——与 [`evaluate_pair_in`] 同序同判
    /// （prohibition 早停 → association 早停 → NotApplicable）。
    pub fn evaluate_pair(
        &self,
        fk_user_attribute: i64,
        fk_object_attribute: i64,
        operation: &str,
        ctx: &ConditionContext,
    ) -> (Decision, Vec<MatchedRule>) {
        let ar_id = match self.access_right_id(operation) {
            Some(id) => id,
            None => return (Decision::NotApplicable, Vec::new()),
        };
        // 策略类分区（与 `evaluate_pair_in` 同语义）：两侧皆登记且不等 ⇒ NotApplicable。
        if let (Some(ua_pc), Some(oa_pc)) = (
            self.user_attr_policy_class.get(&fk_user_attribute),
            self.object_attr_policy_class.get(&fk_object_attribute),
        ) {
            if ua_pc != oa_pc {
                return (Decision::NotApplicable, Vec::new());
            }
        }
        let mut rules: Vec<MatchedRule> = Vec::new();

        if let Some(plist) = self
            .prohib_index
            .get(&(fk_user_attribute, fk_object_attribute))
        {
            for prohibition in plist {
                if prohibition.is_active {
                    let conditions_met = evaluate_conditions(&prohibition.conditions, ctx);
                    let matched = prohibition.ak_access_rights.contains(&ar_id) && conditions_met;
                    rules.push(MatchedRule {
                        rule_type: "prohibition".to_string(),
                        kind: "deny".to_string(),
                        access_rights: self.ar_names(&prohibition.ak_access_rights),
                        conditions: prohibition.conditions.clone(),
                        conditions_met,
                        matched,
                    });
                    if matched {
                        return (Decision::Deny, rules);
                    }
                }
            }
        }

        if let Some(alist) = self
            .assoc_index
            .get(&(fk_user_attribute, fk_object_attribute))
        {
            for assoc in alist {
                let conditions_met = evaluate_conditions(&assoc.conditions, ctx);
                let matched = assoc.ak_access_rights.contains(&ar_id) && conditions_met;
                rules.push(MatchedRule {
                    rule_type: "association".to_string(),
                    kind: "allow".to_string(),
                    access_rights: self.ar_names(&assoc.ak_access_rights),
                    conditions: assoc.conditions.clone(),
                    conditions_met,
                    matched,
                });
                if matched {
                    return (Decision::Permit, rules);
                }
            }
        }

        (Decision::NotApplicable, rules)
    }

    /// deny-overrides 全局合并 + admin 遍历后兜底——语义等价 [`evaluate_merge_in`]。
    /// 遍历顺序不影响结果；明细记录全部参与求值的 (UA, OA) 对（不早停）。
    pub fn evaluate(
        &self,
        ua_closure_ids: &[i64],
        oa_closure_ids: &[i64],
        operation: &str,
        ctx: &ConditionContext,
        is_admin: bool,
    ) -> (Decision, Vec<PairOutcome>) {
        let mut outcomes: Vec<PairOutcome> = Vec::new();
        let mut saw_permit = false;
        for &ua in ua_closure_ids {
            for &oa in oa_closure_ids {
                let (decision, rules) = self.evaluate_pair(ua, oa, operation, ctx);
                if decision == Decision::Permit {
                    saw_permit = true;
                }
                outcomes.push(PairOutcome {
                    ua_id: ua,
                    oa_id: oa,
                    decision,
                    rules,
                });
            }
        }
        let saw_deny = outcomes.iter().any(|o| o.decision == Decision::Deny);
        let decision = if saw_deny {
            Decision::Deny
        } else if saw_permit || is_admin {
            Decision::Permit
        } else {
            Decision::NotApplicable
        };
        (decision, outcomes)
    }

    /// 轻量合并裁决（**不构建轨迹 + Deny 短路**）——终态语义等价 [`evaluate_merge_in`]。
    /// 供 RLS 逐行 / 高频批量路径使用（避免 [`Self::evaluate`] 的 `PairOutcome` 分配）。
    pub fn evaluate_decision(
        &self,
        ua_closure_ids: &[i64],
        oa_closure_ids: &[i64],
        operation: &str,
        ctx: &ConditionContext,
        is_admin: bool,
    ) -> Decision {
        let mut saw_permit = false;
        for &ua in ua_closure_ids {
            for &oa in oa_closure_ids {
                match self.evaluate_pair(ua, oa, operation, ctx).0 {
                    Decision::Deny => return Decision::Deny,
                    Decision::Permit => saw_permit = true,
                    Decision::NotApplicable => {}
                }
            }
        }
        if saw_permit || is_admin {
            Decision::Permit
        } else {
            Decision::NotApplicable
        }
    }

    /// 反向枚举：授予 `operation`、且目标 OA ∈ `oa_ids` 的全部 association 的 UA id
    /// （去重、升序）。供可达性分析（`/api/admin/ngac/analysis/reachability`）消费——
    /// 使宿主机不再直接迭代 `PolicyGraph::associations`（门禁 P6）。**不产 Permit/Deny**。
    pub fn uas_granting(&self, operation: &str, oa_ids: &[i64]) -> Vec<i64> {
        let ar_id = match self.access_right_id(operation) {
            Some(id) => id,
            None => return Vec::new(),
        };
        let mut out: Vec<i64> = Vec::new();
        for list in self.assoc_index.values() {
            for a in list {
                if a.ak_access_rights.contains(&ar_id) && oa_ids.contains(&a.fk_object_attribute) {
                    out.push(a.fk_user_attribute);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

// ── 规范决策序 ───────────────────────────────────────────────────────────────

/// 规范决策序（NGAC_SPEC §2.5/§2.7/§2.2.7）——唯一实现。
///
/// ① `ceiling_denied == Some(true)`（密级上限，判据布尔由宿主经
/// `ngac_org::label_clearance_denies` 单源 SQL 供给；`None`/`Some(false)` = 不介入）
/// ⇒ Deny——先于 bootstrap：白名单/标签已存在即平台非「未配置」态；
/// ② `bootstrap_empty`（从未初始化：无 default 策略类且零 association，
/// [`PolicyGraph::bootstrap_empty`]）⇒ Permit（首个管理员不被锁死）；
/// ③ `merged`（[`evaluate_merge_in`] 产出，含 admin 遍历后兜底）；
/// ④ creator 派生授权仅翻转 NotApplicable ⇒ Permit（**Deny 不可被派生覆盖**）。
///
/// 宿主可为性能提前返回，但终态 MUST 与本函数输出等价（等价性由测试守门）。
pub fn decide_ordered(
    ceiling_denied: Option<bool>,
    bootstrap_empty: bool,
    merged: Decision,
    derived_permits: bool,
) -> Decision {
    if ceiling_denied == Some(true) {
        return Decision::Deny;
    }
    if bootstrap_empty {
        return Decision::Permit;
    }
    match merged {
        Decision::Deny => Decision::Deny,
        Decision::Permit => Decision::Permit,
        Decision::NotApplicable => {
            if derived_permits {
                Decision::Permit
            } else {
                Decision::NotApplicable
            }
        }
    }
}

// ── dry-run 策略补丁（establish-age-graph-analysis-plane 任务 5.1）─────────────

/// 候选策略补丁（提案-确认管线的 dry-run 面：先在**克隆图**上求值受影响面，
/// 人工确认后才事务写关系表——写库后由版本 bump + 投影对账收敛）。
#[derive(Debug, Clone)]
pub enum PolicyPatch {
    AddAssociation(Association),
    RemoveAssociation(i64),
    AddProhibition(Prohibition),
    RemoveProhibition(i64),
}

/// 克隆图并按序重放补丁（源图零影响；同 id 重复 Add = 覆盖，Remove 不存在 = 无操作，
/// 与管理面 upsert/软删语义对齐）。dry-run 求值 = 在返回图上照常调
/// [`evaluate_merge_in`] / [`decide_ordered`]——**不经 AGE、不写任何存储**。
pub fn apply_patches(pg: &PolicyGraph, patches: &[PolicyPatch]) -> PolicyGraph {
    let next = pg.clone();
    for patch in patches {
        match patch {
            PolicyPatch::AddAssociation(a) => next.add_association(a.clone()),
            PolicyPatch::RemoveAssociation(id) => next.remove_association(*id),
            PolicyPatch::AddProhibition(p) => next.add_prohibition(p.clone()),
            PolicyPatch::RemoveProhibition(id) => next.remove_prohibition(*id),
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx_at(ts: &str) -> ConditionContext {
        ConditionContext {
            now: DateTime::parse_from_rfc3339(ts)
                .unwrap()
                .with_timezone(&Utc),
            user_ua_names: Vec::new(),
            oa_closure_names: Vec::new(),
            rhai_eval: None,
        }
    }

    // ── conditions（v1 时间窗 + v2 属性闭包，失败封闭） ──────────────────────

    #[test]
    fn conditions_null_is_unconditional() {
        assert!(evaluate_conditions(&None, &ctx_at("2026-06-01T00:00:00Z")));
    }

    #[test]
    fn conditions_empty_object_is_unconditional() {
        let c = Some(json!({}));
        assert!(evaluate_conditions(&c, &ctx_at("2026-06-01T00:00:00Z")));
    }

    #[test]
    fn time_window_active() {
        let c = Some(json!({
            "not_before": "2026-01-01T00:00:00Z",
            "not_after": "2026-12-31T23:59:59Z"
        }));
        assert!(evaluate_conditions(&c, &ctx_at("2026-06-01T00:00:00Z")));
    }

    #[test]
    fn time_window_before_not_before() {
        let c = Some(json!({"not_before": "2026-01-01T00:00:00Z"}));
        assert!(!evaluate_conditions(&c, &ctx_at("2025-12-31T23:59:59Z")));
    }

    #[test]
    fn time_window_after_not_after() {
        let c = Some(json!({"not_after": "2026-12-31T23:59:59Z"}));
        assert!(!evaluate_conditions(&c, &ctx_at("2027-01-01T00:00:00Z")));
    }

    #[test]
    fn malformed_timestamp_fails_closed() {
        let c = Some(json!({"not_before": "not-a-date"}));
        assert!(!evaluate_conditions(&c, &ctx_at("2026-06-01T00:00:00Z")));
    }

    #[test]
    fn non_string_timestamp_fails_closed() {
        let c = Some(json!({"not_before": 12345}));
        assert!(!evaluate_conditions(&c, &ctx_at("2026-06-01T00:00:00Z")));
    }

    #[test]
    fn user_attr_in_requires_membership() {
        let mut ctx = ctx_at("2026-06-01T00:00:00Z");
        ctx.user_ua_names = vec!["position:POS-1".to_string()];
        let hit = Some(json!({"user_attr_in": ["position:POS-1"]}));
        let miss = Some(json!({"user_attr_in": ["position:POS-2"]}));
        let empty = Some(json!({"user_attr_in": []}));
        assert!(evaluate_conditions(&hit, &ctx));
        assert!(!evaluate_conditions(&miss, &ctx));
        assert!(!evaluate_conditions(&empty, &ctx));
    }

    #[test]
    fn object_attr_in_requires_closure_membership() {
        let mut ctx = ctx_at("2026-06-01T00:00:00Z");
        ctx.oa_closure_names = vec!["engineers-collection".to_string()];
        let hit = Some(json!({"object_attr_in": ["engineers-collection"]}));
        let miss = Some(json!({"object_attr_in": ["nope"]}));
        assert!(evaluate_conditions(&hit, &ctx));
        assert!(!evaluate_conditions(&miss, &ctx));
    }

    // ── 合并（deny-overrides + admin 遍历后兜底，遍历序无关） ────────────────

    fn graph_fixture() -> PolicyGraph {
        let pg = PolicyGraph::new();
        pg.add_access_right(AccessRight {
            id: 1,
            o_name: "read".into(),
            applicable_types: vec![],
            is_system: true,
        });
        // UA 10 → OA 20 允许 read；UA 11 → OA 20 禁止 read
        pg.add_association(Association {
            id: 100,
            fk_user_attribute: 10,
            fk_object_attribute: 20,
            ak_access_rights: vec![1],
            conditions: None,
        });
        pg.add_prohibition(Prohibition {
            id: 200,
            o_name: String::new(),
            fk_user_attribute: 11,
            fk_object_attribute: 20,
            ak_access_rights: vec![1],
            is_active: true,
            conditions: None,
        });
        pg
    }

    #[test]
    fn merge_allow_and_prohibition_across_pairs_is_deny() {
        let pg = graph_fixture();
        let ctx = ConditionContext::default();
        // 两种遍历顺序（UA 升序 / 降序）终态一致
        let (d1, o1) = evaluate_merge_in(&pg, &[10, 11], &[20], "read", &ctx, false);
        let (d2, _) = evaluate_merge_in(&pg, &[11, 10], &[20], "read", &ctx, false);
        assert_eq!(d1, Decision::Deny);
        assert_eq!(d2, Decision::Deny);
        assert_eq!(o1.len(), 2);
    }

    #[test]
    fn merge_admin_fallback_only_when_not_applicable() {
        let pg = graph_fixture();
        let ctx = ConditionContext::default();
        // 无命中对 + admin ⇒ Permit；有 prohibition 命中 + admin ⇒ Deny
        let (d, _) = evaluate_merge_in(&pg, &[99], &[20], "read", &ctx, true);
        assert_eq!(d, Decision::Permit);
        let (d, _) = evaluate_merge_in(&pg, &[11], &[20], "read", &ctx, true);
        assert_eq!(d, Decision::Deny);
    }

    #[test]
    fn unknown_operation_is_not_applicable() {
        let pg = graph_fixture();
        let ctx = ConditionContext::default();
        let (d, _) = evaluate_merge_in(&pg, &[10], &[20], "nonexistent", &ctx, false);
        assert_eq!(d, Decision::NotApplicable);
    }

    // ── 决策序（ceiling → bootstrap → merged → derived） ─────────────────────

    #[test]
    fn ceiling_precedes_bootstrap_permit() {
        assert_eq!(
            decide_ordered(Some(true), true, Decision::NotApplicable, false),
            Decision::Deny
        );
    }

    #[test]
    fn bootstrap_permits_despite_no_policies() {
        assert_eq!(
            decide_ordered(None, true, Decision::NotApplicable, false),
            Decision::Permit
        );
    }

    #[test]
    fn derived_flip_never_overrides_deny() {
        assert_eq!(
            decide_ordered(None, false, Decision::Deny, true),
            Decision::Deny
        );
        assert_eq!(
            decide_ordered(None, false, Decision::NotApplicable, true),
            Decision::Permit
        );
        assert_eq!(
            decide_ordered(None, false, Decision::NotApplicable, false),
            Decision::NotApplicable
        );
    }

    // ── bootstrap 判据（无 default 类 且 零 association） ─────────────────────

    #[test]
    fn bootstrap_empty_requires_no_class_and_no_associations() {
        let pg = PolicyGraph::new();
        assert!(pg.bootstrap_empty());
        pg.add_policy_class("default", 1);
        assert!(
            !pg.bootstrap_empty(),
            "有 default 类即半配置态，不得 fail-open"
        );
        pg.add_association(Association {
            id: 1,
            fk_user_attribute: 1,
            fk_object_attribute: 1,
            ak_access_rights: vec![1],
            conditions: None,
        });
        assert!(!pg.bootstrap_empty());
    }

    // ── dry-run 补丁（任务 5.1）────────────────────────────────────────────

    #[test]
    fn dry_run_patch_keeps_source_graph_untouched() {
        let src = graph_fixture(); // UA10 → OA20 read 允许
        let patched = apply_patches(
            &src,
            &[
                PolicyPatch::AddProhibition(Prohibition {
                    id: 201,
                    o_name: String::new(),
                    fk_user_attribute: 10,
                    fk_object_attribute: 20,
                    ak_access_rights: vec![1],
                    is_active: true,
                    conditions: None,
                }),
                PolicyPatch::RemoveAssociation(100),
            ],
        );
        // 源图不受影响：原决策仍 Permit
        let ctx = ConditionContext::default();
        let (d, _) = evaluate_merge_in(&src, &[10], &[20], "read", &ctx, false);
        assert_eq!(d, Decision::Permit);
        // 克隆图：补丁后同请求翻转为 Deny（prohibition 命中 + allow 边已移除）
        let (d2, _) = evaluate_merge_in(&patched, &[10], &[20], "read", &ctx, false);
        assert_eq!(d2, Decision::Deny);
    }

    #[test]
    fn dry_run_remove_nonexistent_is_noop() {
        let src = graph_fixture();
        let patched = apply_patches(&src, &[PolicyPatch::RemoveProhibition(999)]);
        let ctx = ConditionContext::default();
        let (d, _) = evaluate_merge_in(&patched, &[10], &[20], "read", &ctx, false);
        assert_eq!(d, Decision::Permit, "移除不存在的边 = 无操作");
    }

    // ── 索引原语 ≡ 朴素合并（indexed-evaluation-primitive 等价性） ───────────

    /// 断言索引原语与朴素实现对同一 (UA 闭包 × OA 闭包 × op) 产出同一裁决与同一逐对裁决/轨迹。
    fn assert_indexed_eq_naive(
        pg: &PolicyGraph,
        ua: &[i64],
        oa: &[i64],
        op: &str,
        ctx: &ConditionContext,
        is_admin: bool,
    ) {
        let (naive, naive_outcomes) = evaluate_merge_in(pg, ua, oa, op, ctx, is_admin);
        let indexed_graph = IndexedPolicyGraph::from(pg);
        // 轻量路径（不构建轨迹 + Deny 短路）终态 MUST 与朴素合并一致
        assert_eq!(
            naive,
            indexed_graph.evaluate_decision(ua, oa, op, ctx, is_admin),
            "轻量裁决不一致 op={op} is_admin={is_admin}"
        );
        let (indexed, indexed_outcomes) = indexed_graph.evaluate(ua, oa, op, ctx, is_admin);
        assert_eq!(naive, indexed, "决策不一致 op={op} is_admin={is_admin}");
        assert_eq!(naive_outcomes.len(), indexed_outcomes.len());
        for (n, i) in naive_outcomes.iter().zip(indexed_outcomes.iter()) {
            assert_eq!(n.ua_id, i.ua_id);
            assert_eq!(n.oa_id, i.oa_id);
            assert_eq!(n.decision, i.decision, "对级裁决不一致");
            assert_eq!(n.rules.len(), i.rules.len(), "轨迹条数不一致");
            for (nr, ir) in n.rules.iter().zip(i.rules.iter()) {
                assert_eq!(nr.rule_type, ir.rule_type);
                assert_eq!(nr.matched, ir.matched);
                assert_eq!(nr.conditions_met, ir.conditions_met);
            }
        }
    }

    #[test]
    fn indexed_equivalent_to_naive_merge() {
        let pg = graph_fixture();
        let ctx = ConditionContext::default();
        // deny 冲突 / 仅 allow / 未知操作 / admin 兜底 / 无命中非 admin
        assert_indexed_eq_naive(&pg, &[10, 11], &[20], "read", &ctx, false);
        assert_indexed_eq_naive(&pg, &[11, 10], &[20], "read", &ctx, false); // 遍历序无关
        assert_indexed_eq_naive(&pg, &[10], &[20], "read", &ctx, false);
        assert_indexed_eq_naive(&pg, &[99], &[20], "read", &ctx, true);
        assert_indexed_eq_naive(&pg, &[99], &[20], "read", &ctx, false);
        assert_indexed_eq_naive(&pg, &[10], &[20], "nonexistent", &ctx, false);
    }

    #[test]
    fn cross_policy_class_pair_is_not_applicable() {
        let pg = graph_fixture();
        // 登记：UA 10 属 PC 1，OA 20 属 PC 2（跨分区）
        pg.add_user_attribute_pc(10, 1);
        pg.add_object_attribute_pc(20, 2);
        let ctx = ConditionContext::default();
        // 即便存在 (10 → 20) 的 association（read），跨分区对 MUST 判 NotApplicable
        let (d, rules) = evaluate_pair_in(&pg, 10, 20, "read", &ctx);
        assert_eq!(d, Decision::NotApplicable, "跨分区对无策略");
        assert!(rules.is_empty());
        // 索引原语同语义
        let (d2, _) = IndexedPolicyGraph::from(&pg).evaluate(&[10], &[20], "read", &ctx, false);
        assert_eq!(d2, Decision::NotApplicable);
    }

    #[test]
    fn same_policy_class_pair_unchanged() {
        let pg = graph_fixture();
        let ctx = ConditionContext::default();
        let before = evaluate_pair_in(&pg, 10, 20, "read", &ctx).0;
        // 同分区登记 ⇒ 行为不变
        pg.add_user_attribute_pc(10, 1);
        pg.add_object_attribute_pc(20, 1);
        assert_eq!(evaluate_pair_in(&pg, 10, 20, "read", &ctx).0, before);
        assert_eq!(before, Decision::Permit);
    }

    #[test]
    fn missing_pc_registration_does_not_short_circuit() {
        let pg = graph_fixture();
        let ctx = ConditionContext::default();
        // 未登记分区 ⇒ 不短路（维持既有匹配）
        assert_eq!(
            evaluate_pair_in(&pg, 10, 20, "read", &ctx).0,
            Decision::Permit
        );
    }

    #[test]
    fn indexed_equivalent_with_conditions_fail_closed() {
        // association 带过期条件（不满足）⇒ 不授予；索引与朴素一致（失败封闭）
        let pg = PolicyGraph::new();
        pg.add_access_right(AccessRight {
            id: 1,
            o_name: "read".into(),
            applicable_types: vec![],
            is_system: true,
        });
        pg.add_association(Association {
            id: 1,
            fk_user_attribute: 10,
            fk_object_attribute: 20,
            ak_access_rights: vec![1],
            conditions: Some(json!({"not_after": "2020-01-01T00:00:00Z"})),
        });
        let ctx = ctx_at("2026-06-01T00:00:00Z");
        assert_indexed_eq_naive(&pg, &[10], &[20], "read", &ctx, false);
        let (d, _) = IndexedPolicyGraph::from(&pg).evaluate(&[10], &[20], "read", &ctx, false);
        assert_eq!(
            d,
            Decision::NotApplicable,
            "条件不满足 ⇒ 不授予（失败封闭）"
        );
    }

    /// 实跑基准（`--ignored --nocapture`）：合成泛关联大图，对照索引原语与朴素合并。
    /// 语义等价 + 索引路径不劣于朴素（矩阵路径由索引原语承载）。
    #[test]
    #[ignore]
    fn bench_indexed_vs_naive_large_graph() {
        use std::time::Instant;
        const N: i64 = 6000;
        let pg = PolicyGraph::new();
        pg.add_access_right(AccessRight {
            id: 1,
            o_name: "read".into(),
            applicable_types: vec![],
            is_system: true,
        });
        // 泛关联：UA 10 关联全部 N 个 OA（矩阵场景的最坏形态）
        for oa in 0..N {
            pg.add_association(Association {
                id: oa + 1,
                fk_user_attribute: 10,
                fk_object_attribute: oa,
                ak_access_rights: vec![1],
                conditions: None,
            });
        }
        let ctx = ConditionContext::default();

        let t0 = Instant::now();
        let indexed = IndexedPolicyGraph::from(&pg);
        let t_build = t0.elapsed();

        let t1 = Instant::now();
        let mut permits = 0;
        for oa in 0..N {
            if indexed.evaluate_decision(&[10], &[oa], "read", &ctx, false) == Decision::Permit {
                permits += 1;
            }
        }
        let t_eval = t1.elapsed();

        let t2 = Instant::now();
        let mut naive_permits = 0;
        for oa in 0..N {
            if evaluate_merge_in(&pg, &[10], &[oa], "read", &ctx, false).0 == Decision::Permit {
                naive_permits += 1;
            }
        }
        let t_naive = t2.elapsed();

        println!(
            "[bench] N={N} index_build={t_build:?} indexed_eval={t_eval:?} naive_eval={t_naive:?} (permits {permits}/{naive_permits})"
        );
        assert_eq!(permits, naive_permits, "索引与朴素裁决一致");
    }

    // ── conditions 第三形态 `{"rhai": …}`（切片 B：注入式求值器） ──────────────

    fn ctx_with_eval(
        f: impl Fn(&str, &ConditionContext) -> bool + Send + Sync + 'static,
    ) -> ConditionContext {
        ConditionContext {
            now: Utc::now(),
            user_ua_names: Vec::new(),
            oa_closure_names: Vec::new(),
            rhai_eval: Some(std::sync::Arc::new(f)),
        }
    }

    #[test]
    fn rhai_condition_fail_closed_and_injected() {
        let conds = serde_json::json!({ "rhai": "action == \"read\"" });
        // 未注入求值器（`rhai_eval: None`）⇒ 不满足（fail-closed：MUST NOT 因「未知键被忽略」放行）
        assert!(!evaluate_conditions(
            &Some(conds.clone()),
            &ctx_at("2026-06-01T00:00:00Z")
        ));
        // 注入求值器：命中 ⇒ 满足；未命中 ⇒ 不满足
        assert!(evaluate_conditions(
            &Some(conds.clone()),
            &ctx_with_eval(|src, _| src.contains("read"))
        ));
        assert!(!evaluate_conditions(
            &Some(conds.clone()),
            &ctx_with_eval(|_, _| false)
        ));
        // `null` / 空串 ⇒ 视为无条件（不介入，与 `{}` 同）
        assert!(evaluate_conditions(
            &Some(serde_json::json!({ "rhai": null })),
            &ctx_at("2026-06-01T00:00:00Z")
        ));
        assert!(evaluate_conditions(
            &Some(serde_json::json!({ "rhai": "   " })),
            &ctx_at("2026-06-01T00:00:00Z")
        ));
        // 与 v2 共存：同一条件对象内为 AND 语义（v2 不满足 ⇒ 整体不满足）
        let mixed = serde_json::json!({ "user_attr_in": ["missing-ua"], "rhai": "true" });
        assert!(!evaluate_conditions(
            &Some(mixed),
            &ctx_with_eval(|_, _| true)
        ));
    }

    // ── 未知键 fail-closed（tasks 1.5：静默放行面收口） ─────────────────────────

    #[test]
    fn unknown_condition_key_is_fail_closed() {
        let ctx = ctx_at("2026-06-01T00:00:00Z");
        // 纯未知键：此前被静默忽略 ⇒ 视为无条件放行；现 MUST 不满足
        assert!(!evaluate_conditions(
            &Some(serde_json::json!({ "bogus": 1 })),
            &ctx
        ));
        // 键名拼写错误（`not_befor`）——最现实的误写形态，MUST 不满足（MUST NOT 退化为无条件）
        assert!(!evaluate_conditions(
            &Some(serde_json::json!({ "not_befor": "2020-01-01T00:00:00Z" })),
            &ctx
        ));
        // 未知键与合法键共存 ⇒ 整体不满足（未知键不可被「合法键已足够」掩盖）
        assert!(!evaluate_conditions(
            &Some(serde_json::json!({ "not_before": "2020-01-01T00:00:00Z", "bogus": true })),
            &ctx
        ));
        // 白名单五键各自单用：合法键 MUST NOT 被误判（时间窗满足 / v2 命中 / rhai 命中）
        assert!(evaluate_conditions(
            &Some(serde_json::json!({ "not_before": "2020-01-01T00:00:00Z" })),
            &ctx
        ));
        assert!(evaluate_conditions(
            &Some(serde_json::json!({ "not_after": "2999-01-01T00:00:00Z" })),
            &ctx
        ));
        let ctx_hit = ConditionContext {
            now: Utc::now(),
            user_ua_names: vec!["ua-1".to_string()],
            oa_closure_names: vec!["oa-1".to_string()],
            rhai_eval: Some(std::sync::Arc::new(|_, _| true)),
        };
        assert!(evaluate_conditions(
            &Some(serde_json::json!({ "user_attr_in": ["ua-1"] })),
            &ctx_hit
        ));
        assert!(evaluate_conditions(
            &Some(serde_json::json!({ "object_attr_in": ["oa-1"] })),
            &ctx_hit
        ));
        assert!(evaluate_conditions(
            &Some(serde_json::json!({ "rhai": "true" })),
            &ctx_hit
        ));
        // 空对象 / null ⇒ 无条件（既有语义不变）
        assert!(evaluate_conditions(&Some(serde_json::json!({})), &ctx));
        assert!(evaluate_conditions(&None, &ctx));
        // 常量与求值面同源（写径校验复用）
        assert_eq!(CONDITION_KEYS.len(), 5);
    }

    /// 常量-求值面同源（防 drift）：`evaluate_conditions` 内 `obj.get("<key>")` 的每个键 MUST 登记在
    /// [`CONDITION_KEYS`]，且登记项 MUST 在求值面出现。新增条件形态若只加求值分支而忘登记键 ⇒
    /// 写径放行该键、核心已能求值（无害）；反之只登记不求值 ⇒ 核心静默失败封闭而写径放行
    /// （正是 tasks 1.5 的静默面）。两者 MUT 同步。
    #[test]
    fn condition_keys_constant_matches_evaluator_branches() {
        let src = include_str!("lib.rs");
        let body = src
            .split("pub fn evaluate_conditions(")
            .nth(1)
            .expect("evaluate_conditions 源存在")
            .split("\nfn parse_iso8601(")
            .next()
            .expect("函数体边界存在");
        let mut referenced: Vec<&str> = Vec::new();
        let mut rest = body;
        while let Some(i) = rest.find("obj.get(\"") {
            let after = &rest[i + "obj.get(\"".len()..];
            let end = after.find('"').expect("键字面量闭合");
            referenced.push(&after[..end]);
            rest = &after[end..];
        }
        assert!(
            !referenced.is_empty(),
            "求值分支键提取为空（源码结构变化 ⇒ 同步更新本测试）"
        );
        for k in &referenced {
            assert!(
                CONDITION_KEYS.contains(k),
                "求值分支键 `{k}` 未登记在 CONDITION_KEYS（写径会放行该键而核心 fail-closed）"
            );
        }
        for k in CONDITION_KEYS {
            assert!(
                referenced.contains(&k),
                "CONDITION_KEYS 项 `{k}` 在求值面缺失（登记了但未实现）"
            );
        }
    }
}
