//! 本体坐标上下文（DkContext）
//!
//! 记录级本体三维坐标 `dk_scene / dk_factor / dk_function`，
//! 由前端在 CRUD 请求中通过 `X-Alioth-Coord` header 传递三个 ZUID，
//! 后端提取后供 `Repository.create()` 写入 INSERT 的 `dk_*` 列。
//!
//! 设计约束：
//! - 运行时只传递 ZUID（大整数），不传递 code 字符串
//! - 坐标值在预处理阶段（ontology-mapping）已完成 DB 校验
//! - 后端通过 ResourceRegistry 二次校验 ZUID 有效性

use crate::error::AliothError;

/// 本体三维坐标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DkContext {
    pub dk_scene: i64,
    pub dk_factor: i64,
    pub dk_function: i64,
}

impl DkContext {
    /// 解析原始 `X-Alioth-Coord` header 值。
    /// 格式: `scene=123,factor=456,function=789`
    pub fn parse_header(value: &str) -> Result<Self, AliothError> {
        let mut scene = None;
        let mut factor = None;
        let mut function = None;

        for part in value.split(',') {
            let kv: Vec<&str> = part.splitn(2, '=').collect();
            if kv.len() != 2 {
                continue;
            }
            let val: i64 = kv[1].trim().parse().map_err(|_| {
                AliothError::BadRequest(format!("Invalid X-Alioth-Coord value: {}", kv[1]))
            })?;
            match kv[0].trim() {
                "scene" => scene = Some(val),
                "factor" => factor = Some(val),
                "function" => function = Some(val),
                _ => {}
            }
        }

        let dk_scene =
            scene.ok_or_else(|| AliothError::BadRequest("X-Alioth-Coord missing scene".into()))?;
        let dk_factor = factor
            .ok_or_else(|| AliothError::BadRequest("X-Alioth-Coord missing factor".into()))?;
        let dk_function = function
            .ok_or_else(|| AliothError::BadRequest("X-Alioth-Coord missing function".into()))?;

        Ok(Self {
            dk_scene,
            dk_factor,
            dk_function,
        })
    }

    /// 从 `HttpRequest` 的 `X-Alioth-Coord` header 解析。
    pub fn from_request(req: &actix_web::HttpRequest) -> Result<Self, AliothError> {
        let header = req
            .headers()
            .get("X-Alioth-Coord")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AliothError::BadRequest("Missing X-Alioth-Coord header".into()))?;
        Self::parse_header(header)
    }

    /// 从实体声明的坐标构造上下文（`X-Alioth-Coord` header 缺失时的回退路径，
    /// 供通用 CRUD handler 在 `from_request` 失败后按实体声明回填，见
    /// `crud::handler::resolve_dk_ctx`）。
    ///
    /// 三个坐标均声明且为正 → `Some`；任一缺失或非正 → `None`
    /// （保持 `dk_*` 为 NULL，不 fail-closed，兼容无坐标声明的实体）。
    pub fn from_declared(
        scene: Option<i64>,
        factor: Option<i64>,
        function: Option<i64>,
    ) -> Option<Self> {
        let ctx = Self {
            dk_scene: scene?,
            dk_factor: factor?,
            dk_function: function?,
        };
        ctx.is_valid().then_some(ctx)
    }

    /// 验证三个 ZUID 均非零。
    pub fn is_valid(&self) -> bool {
        self.dk_scene > 0 && self.dk_factor > 0 && self.dk_function > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_header() {
        let ctx = DkContext::parse_header("scene=123,factor=456,function=789").unwrap();
        assert_eq!(ctx.dk_scene, 123);
        assert_eq!(ctx.dk_factor, 456);
        assert_eq!(ctx.dk_function, 789);
        assert!(ctx.is_valid());
    }

    #[test]
    fn test_parse_missing_header() {
        assert!(DkContext::parse_header("").is_err());
    }

    #[test]
    fn test_parse_missing_dimension() {
        let r = DkContext::parse_header("scene=123,factor=456");
        assert!(r.is_err());
    }

    #[test]
    fn test_invalid_value() {
        assert!(DkContext::parse_header("scene=abc,factor=456,function=789").is_err());
    }

    #[test]
    fn test_zero_values() {
        let ctx = DkContext::parse_header("scene=0,factor=456,function=789").unwrap();
        assert!(!ctx.is_valid());
    }

    // ── 实体声明回退（from_declared）──

    #[test]
    fn test_from_declared_all_some() {
        let ctx = DkContext::from_declared(Some(1), Some(2), Some(3)).unwrap();
        assert_eq!(
            ctx,
            DkContext {
                dk_scene: 1,
                dk_factor: 2,
                dk_function: 3,
            }
        );
    }

    #[test]
    fn test_from_declared_partial_declaration_is_none() {
        // 只声明 scene → 无完整坐标，回退不成立（保持 NULL）
        assert!(DkContext::from_declared(Some(1), None, None).is_none());
        assert!(DkContext::from_declared(Some(1), Some(2), None).is_none());
        assert!(DkContext::from_declared(None, None, None).is_none());
    }

    #[test]
    fn test_from_declared_non_positive_is_none() {
        assert!(DkContext::from_declared(Some(0), Some(2), Some(3)).is_none());
        assert!(DkContext::from_declared(Some(-1), Some(2), Some(3)).is_none());
    }
}
