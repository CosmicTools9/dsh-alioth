//! dev 日志邮件服务（`SSO_EMAIL_MODE=log` 时使用）
//!
//! 不实际投递邮件，将邮件内容（含验证码）打印到日志——dev 环境无 SMTP 时
//! 仍可走通 send-code → verify-code → register 全链路（验证码从日志获取）。
//! 生产默认 smtp 模式，本服务不会被选中（config.rs fail-closed，未设置/
//! 非法值一律落回 smtp）。

use async_trait::async_trait;
use common::EmailService;

/// 日志邮件服务：邮件内容写入 `log::info`（验证码随 body 可见）
#[derive(Debug, Clone, Default)]
pub struct LogEmailService;

#[async_trait]
impl EmailService for LogEmailService {
    async fn send(&self, to: &str, subject: &str, body: &str) -> common::Result<()> {
        log::info!("[dev-email] to={} subject={} body={}", to, subject, body);
        Ok(())
    }

    async fn send_html(&self, to: &str, subject: &str, html: &str) -> common::Result<()> {
        log::info!(
            "[dev-email] to={} subject={} html_body_len={}（HTML 内容见 dev 日志）",
            to,
            subject,
            html.len()
        );
        Ok(())
    }
}
