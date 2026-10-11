//! ngac-host — NGAC **宿主侧**求值 crate（openspec change `extract-ngac-decision-host`）。
//!
//! 与纯决策核心 `ngac-decision` 的分工：
//! - `ngac-decision`：纯语义（(UA,OA) 对求值、deny-overrides 合并、决策序、conditions、
//!   索引化求值原语）——MUST NOT 依赖 sqlx。
//! - `ngac-host`（本 crate）：**决策输入供给 + 编排 + 宿主接线**——资源作用域装配
//!   （密级上限 → bootstrap 判据 → 用户有效 UA 闭包[有效 ∪ 属权] → 目标 OA 闭包 →
//!   `ConditionContext` → admin 判据 → 合并 → creator 派生翻转 → `decide_ordered`）与
//!   **唯一策略图快照容器**。
//!
//! 分层：`ngac-decision` ← `ngac-host` ← `common` ← SSO / Gateway / Framework crates。
//! 决策输入 SQL 单一定义于本 crate；`common` 及其下游 MUST 引用 `ngac_host::…`，
//! MUST NOT 复制该 SQL（门禁 `scripts/check/check-ngac-decision-chain-single-source.ts`）。

pub mod evaluator;
pub mod input;

pub use evaluator::{DecisionInputs, Evaluator, Verdict};
pub use input::*;
