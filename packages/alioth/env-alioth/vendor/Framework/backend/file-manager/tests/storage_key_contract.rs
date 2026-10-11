// 存储键契约测试（变更：门户写径改用本函数后，键形态成为**跨进程契约**——
// 读端 `repository::resolve_kind` 取第 2 段作 kind、列表/行授权按 `{ns}/%` 过滤，
// 门户 OpenActivity 与平台 Gateway 必须产出同一形态。此前门户自拼 `files/{uid}/{stamp}`，
// 平台侧解不出 kind / 扫不到行 / 找不到字节 ⇒ 本测试把契约钉在框架侧，防回归。
use framework_file_manager::service::key_safe_filename;
use framework_file_manager::{storage_key, FileTableKind};

#[test]
fn storage_key_shape_matches_reader_contract() {
    let k = storage_key("WZ", FileTableKind::Document, 12345, "运输凭证.pdf");
    let segs: Vec<&str> = k.split('/').collect();
    assert_eq!(
        segs.len(),
        4,
        "键 = {{ns}}/{{kind}}/{{file_id}}/{{name}}，实得: {k}"
    );
    assert_eq!(
        segs[0], "WZ",
        "第 1 段 MUST 是 namespace（列表/行授权按 {{ns}}/% 过滤）: {k}"
    );
    assert_eq!(
        segs[1], "document",
        "第 2 段 MUST 是 kind（resolve_kind 取 nth(1)）: {k}"
    );
    assert_eq!(segs[2], "12345", "第 3 段 MUST 是 file_id: {k}");
    assert_eq!(segs[3], key_safe_filename("运输凭证.pdf"));
}

#[test]
fn storage_key_kind_segment_per_table_kind() {
    // 五种 kind 的第 2 段 MUST 均为 `FileTableKind::from_path_segment` 可识别值
    for kind in [
        FileTableKind::Document,
        FileTableKind::Image,
        FileTableKind::Avatar,
        FileTableKind::Package,
        FileTableKind::Versioned,
    ] {
        let seg = storage_key("WZ", kind, 1, "a.bin")
            .split('/')
            .nth(1)
            .map(str::to_string)
            .expect("键含 4 段");
        assert_eq!(
            FileTableKind::from_path_segment(&seg),
            Some(kind),
            "kind 段 `{seg}` 无法被读端反解"
        );
    }
}
