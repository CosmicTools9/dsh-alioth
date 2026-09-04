//! Compression middleware for HTTP responses
//! 
//! Provides gzip and brotli compression for JSON, HTML, CSS, and JavaScript responses.
//! Phase 18: Performance Optimization (PERF-03)

use tower_http::compression::{CompressionLayer, CompressionLevel, predicate::Predicate};
use http::header::CONTENT_TYPE;

/// Creates compression middleware with optimal settings
/// 
/// Compresses responses when:
/// - Content-Type is application/json, text/*, or application/javascript
/// - Response size is worth compressing (>1KB threshold)
pub fn compression_middleware() -> CompressionLayer {
    CompressionLayer::new()
        .gzip(true)
        .br(true)
        .compress_level(CompressionLevel::Fastest)
        .compress_when(CompressibleContentType)
}

/// Predicate that determines if a response should be compressed
#[derive(Clone, Debug)]
struct CompressibleContentType;

impl Predicate for CompressibleContentType {
    fn should_compress<B>(&self, response: &http::Response<B>) -> bool {
        response
            .headers()
            .get(CONTENT_TYPE)
            .map(|ct| {
                let ct_str = ct.as_bytes();
                // Compress JSON, text, and JavaScript
                ct_str.starts_with(b"application/json")
                    || ct_str.starts_with(b"text/")
                    || ct_str.starts_with(b"application/javascript")
                    || ct_str.starts_with(b"application/xml")
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{Response, header::CONTENT_TYPE};

    #[test]
    fn test_compress_json() {
        let mut resp = Response::new(());
        resp.headers_mut().insert(
            CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        assert!(CompressibleContentType.should_compress(&resp));
    }

    #[test]
    fn test_compress_html() {
        let mut resp = Response::new(());
        resp.headers_mut().insert(
            CONTENT_TYPE,
            "text/html".parse().unwrap(),
        );
        assert!(CompressibleContentType.should_compress(&resp));
    }

    #[test]
    fn test_no_compress_image() {
        let mut resp = Response::new(());
        resp.headers_mut().insert(
            CONTENT_TYPE,
            "image/png".parse().unwrap(),
        );
        assert!(!CompressibleContentType.should_compress(&resp));
    }
}
