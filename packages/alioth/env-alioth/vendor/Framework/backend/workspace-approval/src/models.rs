use serde::Serialize;

#[derive(Serialize)]
pub struct ApprovalActionResponse {
    pub success: bool,
    pub message: String,
}

impl ApprovalActionResponse {
    pub fn ok(msg: impl Into<String>) -> Self {
        Self {
            success: true,
            message: msg.into(),
        }
    }
    pub fn fail(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            message: msg.into(),
        }
    }
}

/// 审批操作执行者信息（传入 ApprovalService::execute）
#[derive(Debug, Clone)]
pub struct ApprovalActor {
    /// 操作人用户 ID
    pub user_id: i64,
    /// 审批意见（可选）
    pub opinion: Option<String>,
}

/// 意见通知常量——与 global_overview.rs CASE 口径一致
pub const APPROVAL_NOTICE_APPROVED: &str = "审批通过";
pub const APPROVAL_NOTICE_REJECTED: &str = "审批驳回";
