//! 单元测试：LocalBackend roundtrip、key 校验（路径穿越拒绝）、namespace 目录隔离、
//! FileManager scheme 路由（未配置后端 → 诚实错误）。

use framework_file_manager::backend::LocalBackend;
use framework_file_manager::backend::StorageBackend;
use framework_file_manager::{FileError, FileManager};

fn tmp_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[tokio::test]
async fn local_roundtrip() {
    let root = tmp_root();
    let backend = LocalBackend::new(root.path(), "/files");
    let key = "Alioth/document/1001/photo.png";

    let put = backend.put(key, b"hello world".to_vec(), "image/png").await;
    assert!(put.is_ok());

    let got = backend.get(key).await.expect("get");
    assert_eq!(got, b"hello world");

    // 落盘路径 = base_path / key（namespace 目录树）
    assert!(root.path().join("Alioth/document/1001/photo.png").exists());

    backend.delete(key).await.expect("delete");
    assert!(!root.path().join(key).exists());
    // 删除后再取 → NotFound
    assert!(matches!(
        backend.get(key).await,
        Err(FileError::NotFound(_))
    ));
}

#[tokio::test]
async fn validate_key_rejects_path_traversal() {
    let root = tmp_root();
    let backend = LocalBackend::new(root.path(), "/files");

    // `..` 穿越
    assert!(matches!(
        backend.put("../../etc/passwd", vec![1], "text/plain").await,
        Err(FileError::InvalidKey(_))
    ));
    // 绝对路径
    assert!(matches!(
        backend.put("/etc/passwd", vec![1], "text/plain").await,
        Err(FileError::InvalidKey(_))
    ));
    // 空 key
    assert!(matches!(
        backend.put("", vec![1], "text/plain").await,
        Err(FileError::InvalidKey(_))
    ));
    // 非法字符（空格）
    assert!(matches!(
        backend.put("Alioth/a b", vec![1], "text/plain").await,
        Err(FileError::InvalidKey(_))
    ));
    // 合法 key 放行
    assert!(backend
        .put("WZ/document/2001/合同.pdf", vec![1], "application/pdf")
        .await
        .is_ok());
}

#[tokio::test]
async fn namespace_directory_isolation() {
    let root = tmp_root();
    let backend = LocalBackend::new(root.path(), "/files");

    backend
        .put("Alioth/a.pdf", b"alioth".to_vec(), "application/pdf")
        .await
        .expect("put alioth");
    backend
        .put("WZ/b.pdf", b"wz".to_vec(), "application/pdf")
        .await
        .expect("put wz");

    // namespace 目录树互不可见：跨 namespace 读取 → NotFound
    assert!(matches!(
        backend.get("WZ/a.pdf").await,
        Err(FileError::NotFound(_))
    ));
    assert!(matches!(
        backend.get("Alioth/b.pdf").await,
        Err(FileError::NotFound(_))
    ));
    // 各自 namespace 内可读
    assert_eq!(backend.get("Alioth/a.pdf").await.unwrap(), b"alioth");
    assert_eq!(backend.get("WZ/b.pdf").await.unwrap(), b"wz");
}

#[tokio::test]
async fn local_get_range_slices_without_full_read() {
    let root = tmp_root();
    let backend = LocalBackend::new(root.path(), "/files");
    let key = "Alioth/document/1/hello.txt";
    backend
        .put(key, b"hello world".to_vec(), "text/plain")
        .await
        .expect("put");

    // [0,4] → 前 5 字节
    assert_eq!(backend.get_range(key, 0, 4).await.unwrap(), b"hello");
    // [6,10] → "world"
    assert_eq!(backend.get_range(key, 6, 10).await.unwrap(), b"world");
    // 越界 end → 裁剪到文件尾
    assert_eq!(backend.get_range(key, 8, 100).await.unwrap(), b"rld");
    // 越界 start → 空切片（调用方以 size 校验 416，此处仅后端不 panic）
    assert!(backend.get_range(key, 100, 200).await.is_ok());
    // 不存在的 key → NotFound
    assert!(matches!(
        backend
            .get_range("Alioth/document/9/missing.txt", 0, 4)
            .await,
        Err(FileError::NotFound(_))
    ));
}

#[tokio::test]
async fn local_move_object_renames() {
    let root = tmp_root();
    let backend = LocalBackend::new(root.path(), "/files");
    let from = "Alioth/document/1/old.txt";
    let to = "Alioth/document/1/new.txt";
    backend
        .put(from, b"payload".to_vec(), "text/plain")
        .await
        .expect("put");

    backend.move_object(from, to).await.expect("move");

    // 新键可读，旧键消失
    assert_eq!(backend.get(to).await.unwrap(), b"payload");
    assert!(matches!(
        backend.get(from).await,
        Err(FileError::NotFound(_))
    ));
    // 磁盘上旧文件已删除
    assert!(!root.path().join(from).exists());
    assert!(root.path().join(to).exists());
    // 移动不存在的键 → 错误（不静默成功）
    assert!(backend
        .move_object("Alioth/x/y.txt", "Alioth/x/z.txt")
        .await
        .is_err());
}

#[tokio::test]
async fn scheme_routing_errors_on_unconfigured() {
    // 仅 local 后端：s3 scheme 路由 → 诚实配置错误（非静默降级）
    let fm = FileManager::local_only("./data/files", "/files");
    assert_eq!(fm.default_scheme(), "local");
    assert!(fm.backend_for_scheme("local").is_ok());

    let err = match fm.backend_for_scheme("s3") {
        Err(e) => e,
        Ok(_) => panic!("s3 未配置必须返回错误"),
    };
    assert!(matches!(err, FileError::Config(_)));
    assert!(
        err.to_string().contains("s3"),
        "错误信息须指明缺失的后端: {err}"
    );
}

#[tokio::test]
async fn default_scheme_must_be_registered() {
    // debug_assert 仅 debug；此处验证构造正常（local_only 自洽）
    let fm = FileManager::local_only("/tmp/x", "/files");
    let backend = fm.default_backend();
    let key = "Alioth/document/1/a.txt";
    backend
        .put(key, vec![1u8], "text/plain")
        .await
        .expect("put");
    assert_eq!(backend.get(key).await.unwrap(), vec![1u8]);
}

// ── S3 离线测试（feature s3）───────────────────────────────────────────────
// S3Backend::new 显式 credentials/region/endpoint（不触发 IMDS 探测）；
// presigned_url 为本地 SigV4 签名（无网络）。put/get/delete 需真实 S3 服务，
// 无测试 infra 不覆盖（REUSE_FIRST/依赖门禁：不引入嵌入式 S3 服务 dev-dependency）。

#[cfg(feature = "s3")]
#[tokio::test]
async fn s3_presigned_uses_cdn_domain_when_configured() {
    use framework_file_manager::backend::StorageBackend;
    use framework_file_manager::backend::{S3Backend, S3Config};

    let cfg = S3Config {
        endpoint: "https://s3.example.com".into(),
        region: "us-east-1".into(),
        bucket: "test-bucket".into(),
        access_key: "test-access-key".into(),
        secret_key: "test-secret-key".into(),
        cdn_domain: Some("https://cdn.example.com".into()),
        force_path_style: false,
    };
    let backend = S3Backend::new(&cfg).await.expect("S3Backend::new 离线构造");
    let url = backend
        .presigned_url("Alioth/document/1/a.pdf", 3600)
        .await
        .expect("presigned");
    // CDN 短路：直接拼 CDN + key，无签名参数
    assert_eq!(url, "https://cdn.example.com/Alioth/document/1/a.pdf");
}

#[cfg(feature = "s3")]
#[tokio::test]
async fn s3_presigned_signs_locally_without_network() {
    use framework_file_manager::backend::StorageBackend;
    use framework_file_manager::backend::{S3Backend, S3Config};

    let cfg = S3Config {
        endpoint: "https://s3.example.com".into(),
        region: "us-east-1".into(),
        bucket: "test-bucket".into(),
        access_key: "test-access-key".into(),
        secret_key: "test-secret-key".into(),
        cdn_domain: None,
        force_path_style: false,
    };
    let backend = S3Backend::new(&cfg).await.expect("S3Backend::new 离线构造");
    let url = backend
        .presigned_url("Alioth/document/1/a.pdf", 3600)
        .await
        .expect("presigned");
    // SigV4 本地签名（无网络）：含签名与凭据参数
    assert!(url.contains("X-Amz-Signature="), "应含签名: {url}");
    assert!(url.contains("X-Amz-Credential="), "应含凭据: {url}");
    assert!(url.contains("X-Amz-Expires=3600"), "应含过期时间: {url}");
}

#[cfg(feature = "s3")]
#[tokio::test]
async fn s3_key_validation_rejects_traversal() {
    use framework_file_manager::backend::{S3Backend, S3Config};

    let cfg = S3Config {
        endpoint: "https://s3.example.com".into(),
        region: "us-east-1".into(),
        bucket: "test-bucket".into(),
        access_key: "test-access-key".into(),
        secret_key: "test-secret-key".into(),
        cdn_domain: None,
        force_path_style: false,
    };
    let backend = S3Backend::new(&cfg).await.expect("S3Backend::new 离线构造");
    // 非法 key 在请求前即被拒绝（无网络）
    assert!(matches!(
        backend.presigned_url("../../etc/passwd", 60).await,
        Err(FileError::InvalidKey(_))
    ));
    assert!(matches!(
        backend.get("../../etc/passwd").await,
        Err(FileError::InvalidKey(_))
    ));
}
