//! 图像理解工具（inspect_image 模型链路的文件入口）
//!
//! 用法：
//! ```rust,no_run
//! # async fn demo() -> Result<(), llm::LlmError> {
//! let svc = llm::LlmService::new(llm::LlmServiceConfig::from_env())?;
//! let text = llm::image::inspect_image_file(&svc, std::path::Path::new("receipt.png"), "识别票据金额").await?;
//! # Ok(())
//! # }
//! ```
//!
//! vision 模型链路：`LLM_VISION_PROVIDER` / `LLM_VISION_API_KEY` /
//! `LLM_VISION_BASE_URL` / `LLM_MODEL_VISION` 配置独立视觉后端；
//! 未配置时回退主 provider（DeepSeek 官方 V4 无图像输入，会由上游拒绝并返回可读错误）。

use std::path::Path;

use crate::backends::ImageContent;

/// 按扩展名映射 MIME（不嗅探内容）；未知类型返回 None
pub fn mime_for_path(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png".into()),
        "jpg" | "jpeg" => Some("image/jpeg".into()),
        "gif" => Some("image/gif".into()),
        "webp" => Some("image/webp".into()),
        _ => None,
    }
}

/// 读取图像文件并调用 vision 链路，返回视觉模型文本分析
pub async fn inspect_image_file(
    service: &crate::LlmService,
    path: &Path,
    question: &str,
) -> Result<String, crate::LlmError> {
    let mime = mime_for_path(path)
        .ok_or_else(|| crate::LlmError::Config(format!("不支持的文件类型: {}", path.display())))?;
    let data = std::fs::read(path)
        .map_err(|e| crate::LlmError::Config(format!("读取图像失败 {}: {}", path.display(), e)))?;
    service
        .inspect_image(&ImageContent { mime, data }, question)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_mapping_known_and_unknown() {
        assert_eq!(
            mime_for_path(Path::new("a.png")).as_deref(),
            Some("image/png")
        );
        assert_eq!(
            mime_for_path(Path::new("a.JPG")).as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(
            mime_for_path(Path::new("a.jpeg")).as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(
            mime_for_path(Path::new("a.webp")).as_deref(),
            Some("image/webp")
        );
        assert_eq!(
            mime_for_path(Path::new("a.gif")).as_deref(),
            Some("image/gif")
        );
        assert_eq!(mime_for_path(Path::new("a.pdf")), None);
        assert_eq!(mime_for_path(Path::new("noext")), None);
    }
}
