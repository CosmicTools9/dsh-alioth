//! 主体主状态语义码（模型字典 `isahl.zc_id_stus-org` → 继承 `zc_id_stus-subject`）
//!
//! 黑名单语义是**跨端点唯一判据**：主体列表/选择器的「不可选」过滤（`exclude_disabled`）、
//! 相对方签约拒绝（contract `ensure_subject_active`）、列表 `blacklisted` 投影三者
//! MUST 用同一码集——此前各自硬编码（identity-org 只认 `disabled`，contract 认两码），
//! 导致吊销（`LP-REVOKED`）主体仍出现在「隐藏停用主体」的选择器里。
//!
//! 前端 `BlacklistStatusCodes`（`comprehensive-wz/types/subject.ts`）为同码集的 TS 侧镜像。

/// 黑名单语义码：`LP-REVOKED`（模型级「吊销」）+ `disabled`（存量兼容码，旧 fixture 码集）
pub const BLACKLIST_STATUS_CODES: [&str; 2] = ["LP-REVOKED", "disabled"];

/// 主状态码是否黑名单语义（读侧判定唯一实现）
pub fn is_blacklisted(code: Option<&str>) -> bool {
    matches!(code, Some(c) if BLACKLIST_STATUS_CODES.contains(&c))
}
