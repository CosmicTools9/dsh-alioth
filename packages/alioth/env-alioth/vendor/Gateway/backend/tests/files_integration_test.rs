//! Files 公共服务集成测试（fix-file-storage-local-auth）
//!
//! 使用 aliothstudio_test 数据库，自建自清 fixture（负 ID）。
//! 覆盖：
//! - 全链路：上传（HTTP multipart）→ 列表/下载/删除（handler 直调）
//! - 行级鉴权矩阵：创建者可读写删 / ak_access_user 命中可读 / 无关用户 404
//! - namespace 隔离：跨 namespace 下载 404 / 列表过滤 / 非法 header 400
//! - 上传约束：10MB 上限、扩展名白名单、文件名净化（400）

use ::common::context::RequestContext;
use ::common::testing::connect_test_db;
use actix_web::{test, web, App, HttpMessage, HttpRequest, HttpResponse};
use alioth_gateway::api::files::{download_file, list_files, FilesState};
use sqlx::PgPool;

const USER_A: i64 = -9011;
const USER_B: i64 = -9012;

fn req_with_user(user_id: i64, ns: &str) -> HttpRequest {
    let req = test::TestRequest::default()
        .insert_header(("X-Namespace", ns))
        .to_http_request();
    req.extensions_mut().insert(RequestContext::new(
        user_id,
        format!("user{}@test.local", user_id),
    ));
    req
}

/// 构造 FilesState（本地后端，活库 oss_config 兜底）。
async fn setup_state(pool: PgPool) -> FilesState {
    FilesState::from_live_db(pool).await
}

/// 经 HTTP 层上传（multipart），返回响应。
async fn http_upload(
    state: &FilesState,
    user: i64,
    ns: &str,
    filename: &str,
    content: &[u8],
    extra: &[(&str, &str)],
) -> HttpResponse {
    let boundary = "----files-it-boundary";
    let mut body = actix_web::web::BytesMut::new();
    for (k, v) in extra {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")
                .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let req = test::TestRequest::post()
        .uri("/files")
        .insert_header(("X-Namespace", ns))
        .insert_header((
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        ))
        .set_payload(body.freeze())
        .to_request();
    req.extensions_mut()
        .insert(RequestContext::new(user, format!("u{user}@test.local")));

    let app = test::init_service(App::new().app_data(web::Data::new(state.clone())).service(
        web::scope("/files").route("", web::post().to(alioth_gateway::api::files::upload_file)),
    ))
    .await;
    test::call_service(&app, req).await.into()
}

async fn json_body(resp: HttpResponse) -> serde_json::Value {
    let body = actix_web::body::to_bytes(resp.into_body())
        .await
        .expect("body");
    serde_json::from_slice(&body).expect("json")
}

/// 清理 fixture：URL 链 + 全部 kind 表文件行软删 + 磁盘目录。
async fn cleanup(pool: &PgPool, file_ids: &[i64]) {
    for id in file_ids {
        let _ = sqlx::query(
            r#"UPDATE isahl."zc_id_info-url" SET deleted_at = NOW()
               WHERE id IN (SELECT s.fk_address FROM isahl."zc_id_stor-plc-url" s
                            JOIN isahl."zc_id_file_rr_url" rr ON rr.ref_right = s.id
                            WHERE rr.ref_left = $1)"#,
        )
        .bind(id)
        .execute(pool)
        .await;
        let _ = sqlx::query(
            r#"UPDATE isahl."zc_id_stor-plc-url" SET deleted_at = NOW()
               WHERE id IN (SELECT rr.ref_right FROM isahl."zc_id_file_rr_url" rr
                            WHERE rr.ref_left = $1)"#,
        )
        .bind(id)
        .execute(pool)
        .await;
        let _ = sqlx::query(
            r#"UPDATE isahl."zc_id_file_rr_url" SET deleted_at = NOW() WHERE ref_left = $1"#,
        )
        .bind(id)
        .execute(pool)
        .await;
        // 文件行：全部 5 张 kind 表（kind 由存储链解析，清理须全覆盖）
        for t in [
            "zc_id_file-document",
            "zc_id_file-image",
            "zc_id_file-avatar",
            "zc_id_file-package",
            "zc_id_file-ver_ctrl",
        ] {
            let sql = format!(r#"UPDATE isahl."{t}" SET deleted_at = NOW() WHERE id = $1"#);
            let _ = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(id)
                .execute(pool)
                .await;
        }
        // 磁盘字节：整目录清理（覆盖改名后路径）
        for kind in ["document", "image", "avatar", "package", "ver_ctrl"] {
            let _ = std::fs::remove_dir_all(format!("./data/local-files/Alioth/{kind}/{id}"));
        }
    }
}

// ── 全链路 ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn full_roundtrip_upload_download_list_delete() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    // 上传
    let resp = http_upload(&state, USER_A, "Alioth", "test.txt", b"hello files", &[]).await;
    assert!(resp.status().is_success(), "上传应成功: {}", resp.status());
    let up: serde_json::Value = json_body(resp).await;
    let file_id: i64 = up["id"].as_str().unwrap().parse().unwrap();
    assert_eq!(up["fileName"], "test.txt");
    assert_eq!(up["fileSize"], 11);

    // 下载（创建者）
    let resp = download_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("download");
    assert!(
        resp.status().is_success(),
        "下载应成功，got {}",
        resp.status()
    );
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"hello files");

    // 列表（含该文件）
    let resp = list_files(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Query::from_query("page=1&pageSize=20").unwrap(),
    )
    .await
    .unwrap_or_else(|e| panic!("list failed: {e:?}"));
    let list: serde_json::Value = json_body(resp).await;
    eprintln!("LIST RESP: {}", list);
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"].as_str() == Some(&file_id.to_string())),
        "列表应含刚上传文件"
    );

    // 删除（创建者）
    let resp = alioth_gateway::api::files::delete_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("delete");
    assert!(resp.status().is_success());

    // 删除后下载 → 404（handler 经 Err(NotFound) 表达不可访问）
    let resp = download_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state),
        web::Path::from(file_id),
    )
    .await;
    match resp {
        Ok(r) => assert_eq!(r.status(), actix_web::http::StatusCode::NOT_FOUND),
        Err(e) => assert!(
            e.to_string().contains("文件不存在") || e.to_string().contains("File"),
            "删除后下载应 404/NotFound: {e}"
        ),
    }

    cleanup(&pool, &[file_id]).await;
}

// ── 行级鉴权矩阵 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn row_auth_matrix() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    // A 上传（无授权列）
    let up =
        json_body(http_upload(&state, USER_A, "Alioth", "private.txt", b"secret", &[]).await).await;
    let file_id: i64 = up["id"].as_str().unwrap().parse().unwrap();

    // B 下载 → 404（行级授权拒绝）
    let resp = download_file(
        req_with_user(USER_B, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await;
    if let Ok(r) = resp {
        assert_eq!(
            r.status(),
            actix_web::http::StatusCode::NOT_FOUND,
            "未授权用户应 404"
        );
    } // Err：404/NotFound 语义

    // A 上传声明 akAccessUsers=[B]
    let up2 = json_body(
        http_upload(
            &state,
            USER_A,
            "Alioth",
            "shared.txt",
            b"shared",
            &[("akAccessUsers", &USER_B.to_string())],
        )
        .await,
    )
    .await;
    let file_id2: i64 = up2["id"].as_str().unwrap().parse().unwrap();

    // B 下载授权文件 → 200
    let resp = download_file(
        req_with_user(USER_B, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id2),
    )
    .await
    .expect("download shared");
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::OK,
        "ak_access_user 命中应可读"
    );

    // B 删除 A 的文件 → 404（行级过滤：非创建者且未授权删除）
    let resp = alioth_gateway::api::files::delete_file(
        req_with_user(USER_B, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await;
    if let Ok(r) = resp {
        assert_eq!(
            r.status(),
            actix_web::http::StatusCode::NOT_FOUND,
            "B 不能删 A 的文件"
        );
    } // Err：404/NotFound 语义

    cleanup(&pool, &[file_id, file_id2]).await;
}

// ── namespace 隔离 ─────────────────────────────────────────────────────────

#[tokio::test]
async fn namespace_isolation() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    let up =
        json_body(http_upload(&state, USER_A, "Alioth", "ns-test.txt", b"ns", &[]).await).await;
    let file_id: i64 = up["id"].as_str().unwrap().parse().unwrap();

    // 跨 namespace 下载 → 404（链校验）
    let resp = download_file(
        req_with_user(USER_A, "WZ"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await;
    if let Ok(r) = resp {
        assert_eq!(
            r.status(),
            actix_web::http::StatusCode::NOT_FOUND,
            "跨 namespace 应 404"
        );
    } // Err：404/NotFound 语义

    // WZ 列表不含该文件（namespace 过滤）
    let resp = list_files(
        req_with_user(USER_A, "WZ"),
        web::Data::new(state.clone()),
        web::Query::from_query("page=1&pageSize=20").unwrap(),
    )
    .await
    .expect("list wz");
    let list: serde_json::Value = json_body(resp).await;
    assert!(
        !list["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"].as_str() == Some(&file_id.to_string())),
        "WZ 列表不应含 Alioth 文件"
    );

    cleanup(&pool, &[file_id]).await;
}

// ── 上传约束 ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn upload_limits_enforced() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    // 11MB 超限 → 400
    let big = vec![0u8; 11 * 1024 * 1024];
    let resp = http_upload(&state, USER_A, "Alioth", "big.bin", &big, &[]).await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "11MB 应 400"
    );

    // .exe 白名单外 → 400
    let resp = http_upload(&state, USER_A, "Alioth", "evil.exe", b"x", &[]).await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        ".exe 应 400"
    );

    // 路径穿越文件名 → 400
    let resp = http_upload(&state, USER_A, "Alioth", "../../etc/passwd", b"x", &[]).await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "路径穿越应 400"
    );

    // 缺 X-Namespace → 400（handler 直调可覆盖：namespace 校验在 extract_namespace）
    let req = test::TestRequest::default().to_http_request();
    req.extensions_mut()
        .insert(RequestContext::new(USER_A, "a@test.local"));
    let resp = download_file(
        req,
        web::Data::new(state.clone()),
        web::Path::from(-9999i64),
    )
    .await;
    assert!(resp.is_err(), "缺 namespace 应报错");
}

// ── 非 document kind 全链路（bug① 回归）──────────────────────────────────

#[tokio::test]
async fn non_document_kind_roundtrip() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    // 合法 PNG 头 + 内容
    let png: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
    let resp = http_upload(
        &state,
        USER_A,
        "Alioth",
        "photo.png",
        png,
        &[("tableKind", "image")],
    )
    .await;
    assert!(
        resp.status().is_success(),
        "image 上传应成功: {}",
        resp.status()
    );
    let up = json_body(resp).await;
    let file_id: i64 = up["id"].as_str().unwrap().parse().unwrap();

    // 下载（image 表路由，bug① 修复点）→ 原字节
    let resp = download_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("download image");
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::OK,
        "image 下载应 200"
    );
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], png);

    // 列表（tableKind=image）→ 含该文件
    let resp = list_files(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Query::from_query("page=1&pageSize=20&tableKind=image").unwrap(),
    )
    .await
    .expect("list image");
    let list = json_body(resp).await;
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"].as_str() == Some(&file_id.to_string())),
        "image 列表应含该文件"
    );

    // 删除（image 表路由）→ 成功；再下载 → 404
    let resp = alioth_gateway::api::files::delete_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("delete image");
    assert!(resp.status().is_success(), "image 删除应成功");
    let resp = download_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state),
        web::Path::from(file_id),
    )
    .await;
    match resp {
        Ok(r) => assert_eq!(r.status(), actix_web::http::StatusCode::NOT_FOUND),
        Err(e) => assert!(
            e.to_string().contains("文件不存在") || e.to_string().contains("File"),
            "删除后下载应 404/NotFound: {e}"
        ),
    }

    cleanup(&pool, &[file_id]).await;
}

// ── checksum 闭环 + 条件请求 ──────────────────────────────────────────────

#[tokio::test]
async fn checksum_and_conditional_download() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    let up = json_body(http_upload(&state, USER_A, "Alioth", "sum.txt", b"hello files", &[]).await)
        .await;
    let file_id: i64 = up["id"].as_str().unwrap().parse().unwrap();
    let upload_checksum = up["checksum"]
        .as_str()
        .expect("上传响应应含 checksum")
        .to_string();
    assert!(!upload_checksum.is_empty());

    // 列表条目含 checksum
    let resp = list_files(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Query::from_query("page=1&pageSize=20").unwrap(),
    )
    .await
    .expect("list");
    let list = json_body(resp).await;
    let item = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"].as_str() == Some(&file_id.to_string()))
        .expect("列表含文件");
    assert_eq!(item["checksum"].as_str().unwrap(), upload_checksum);

    // 下载响应头：ETag = "sha256:{checksum}" + X-Checksum-Sha256 + Last-Modified
    let resp = download_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("download");
    let etag = resp
        .headers()
        .get(actix_web::http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag 头")
        .to_string();
    let xcs = resp
        .headers()
        .get("X-Checksum-Sha256")
        .and_then(|v| v.to_str().ok())
        .expect("X-Checksum-Sha256 头");
    assert_eq!(etag, format!("\"sha256:{upload_checksum}\""));
    assert_eq!(xcs, upload_checksum);
    assert!(resp
        .headers()
        .contains_key(actix_web::http::header::LAST_MODIFIED));
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"hello files");

    // If-None-Match=ETag → 304（空体）
    let req = test::TestRequest::default()
        .insert_header(("X-Namespace", "Alioth"))
        .insert_header((actix_web::http::header::IF_NONE_MATCH, etag.clone()))
        .to_http_request();
    req.extensions_mut()
        .insert(RequestContext::new(USER_A, "u@test.local"));
    let resp = download_file(req, web::Data::new(state.clone()), web::Path::from(file_id))
        .await
        .expect("download 304");
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::NOT_MODIFIED,
        "If-None-Match 应 304"
    );

    // 篡改磁盘字节 → 下载必须失败（ChecksumMismatch）
    let disk = format!("./data/local-files/Alioth/document/{file_id}/sum.txt");
    std::fs::write(&disk, b"tampered!!").unwrap();
    let resp = download_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await;
    assert!(resp.is_err(), "篡改后下载必须失败");

    // 恢复字节 → 下载恢复 200
    std::fs::write(&disk, b"hello files").unwrap();
    let resp = download_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("download after restore");
    assert_eq!(resp.status(), actix_web::http::StatusCode::OK);

    cleanup(&pool, &[file_id]).await;
}

// ── Range 语义 ────────────────────────────────────────────────────────────

#[tokio::test]
async fn range_download_semantics() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    // "hello world" = 11 字节
    let up =
        json_body(http_upload(&state, USER_A, "Alioth", "range.txt", b"hello world", &[]).await)
            .await;
    let file_id: i64 = up["id"].as_str().unwrap().parse().unwrap();

    // bytes=0-4 → 206 + "hello"
    let resp = range_download(&state, file_id, "bytes=0-4")
        .await
        .expect("range");
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::PARTIAL_CONTENT,
        "0-4 应 206"
    );
    assert_eq!(
        resp.headers()
            .get(actix_web::http::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .unwrap(),
        "bytes 0-4/11"
    );
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"hello");

    // bytes=6- → 206 "world"
    let resp = range_download(&state, file_id, "bytes=6-")
        .await
        .expect("range open");
    assert_eq!(resp.status(), actix_web::http::StatusCode::PARTIAL_CONTENT);
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"world");

    // bytes=-5 → 206 最后 5 字节 "world"
    let resp = range_download(&state, file_id, "bytes=-5")
        .await
        .expect("range suffix");
    assert_eq!(resp.status(), actix_web::http::StatusCode::PARTIAL_CONTENT);
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"world");

    // bytes=50-60（越界）→ 416 + Content-Range: bytes */11
    let resp = range_download(&state, file_id, "bytes=50-60")
        .await
        .expect("range oob");
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::RANGE_NOT_SATISFIABLE,
        "越界应 416"
    );
    assert_eq!(
        resp.headers()
            .get(actix_web::http::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .unwrap(),
        "bytes */11"
    );

    // 多区间 → 忽略 → 200 全量
    let resp = range_download(&state, file_id, "bytes=0-3,5-9")
        .await
        .expect("range multi");
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::OK,
        "多区间应忽略 → 200"
    );
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"hello world");

    cleanup(&pool, &[file_id]).await;
}

async fn range_download(
    state: &FilesState,
    file_id: i64,
    range: &str,
) -> Result<HttpResponse, ::common::error::AliothError> {
    let req = test::TestRequest::default()
        .insert_header(("X-Namespace", "Alioth"))
        .insert_header((actix_web::http::header::RANGE, range))
        .to_http_request();
    req.extensions_mut()
        .insert(RequestContext::new(USER_A, "u@test.local"));
    download_file(req, web::Data::new(state.clone()), web::Path::from(file_id)).await
}

// ── PUT 更新端点 ──────────────────────────────────────────────────────────

#[tokio::test]
async fn update_rename_replace_share() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    let up =
        json_body(http_upload(&state, USER_A, "Alioth", "old.txt", b"old bytes", &[]).await).await;
    let file_id: i64 = up["id"].as_str().unwrap().parse().unwrap();
    let old_checksum = up["checksum"].as_str().unwrap().to_string();

    // 改名 → 下载新名 200 + 旧磁盘键消失
    let resp = http_update(
        &state,
        USER_A,
        "Alioth",
        file_id,
        None,
        &[("fileName", "renamed.txt")],
    )
    .await;
    assert!(
        resp.status().is_success(),
        "PUT 改名应成功: {}",
        resp.status()
    );
    let resp = download_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("download renamed");
    assert_eq!(resp.status(), actix_web::http::StatusCode::OK);
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"old bytes");
    assert!(
        !std::path::Path::new(&format!(
            "./data/local-files/Alioth/document/{file_id}/old.txt"
        ))
        .exists(),
        "旧存储键应消失"
    );
    assert!(
        std::path::Path::new(&format!(
            "./data/local-files/Alioth/document/{file_id}/renamed.txt"
        ))
        .exists(),
        "新存储键应存在"
    );

    // 替换字节 → checksum 变化 + size 变化
    let resp = http_update(
        &state,
        USER_A,
        "Alioth",
        file_id,
        Some(("new.txt", b"new bytes longer".to_vec())),
        &[],
    )
    .await;
    assert!(
        resp.status().is_success(),
        "PUT 替换应成功: {}",
        resp.status()
    );
    let up2 = json_body(resp).await;
    let new_checksum = up2["checksum"]
        .as_str()
        .expect("替换后应返回 checksum")
        .to_string();
    assert_ne!(new_checksum, old_checksum, "替换后 checksum 必须变化");
    assert_eq!(up2["fileSize"], 16);
    let resp = download_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("download replaced");
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&body[..], b"new bytes longer");

    // 增 akAccessUsers=[B] → B 可下载
    let resp = http_update(
        &state,
        USER_A,
        "Alioth",
        file_id,
        None,
        &[("akAccessUsers", &USER_B.to_string())],
    )
    .await;
    assert!(resp.status().is_success(), "PUT 授权应成功");
    let resp = download_file(
        req_with_user(USER_B, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("B download");
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::OK,
        "授权后 B 应可下载"
    );

    // 空 PUT（无字段）→ 400
    let resp = http_update(&state, USER_A, "Alioth", file_id, None, &[]).await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "空 PUT 应 400"
    );

    cleanup(&pool, &[file_id]).await;
}

/// 经 HTTP 层 PUT 更新（multipart），返回响应。
async fn http_update(
    state: &FilesState,
    user: i64,
    ns: &str,
    file_id: i64,
    file: Option<(&str, Vec<u8>)>,
    extra: &[(&str, &str)],
) -> HttpResponse {
    let boundary = "----files-update-boundary";
    let mut body = actix_web::web::BytesMut::new();
    for (k, v) in extra {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")
                .as_bytes(),
        );
    }
    if let Some((fname, content)) = file {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{fname}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let req = test::TestRequest::put()
        .uri(&format!("/files/{file_id}"))
        .insert_header(("X-Namespace", ns))
        .insert_header((
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        ))
        .set_payload(body.freeze())
        .to_request();
    req.extensions_mut()
        .insert(RequestContext::new(user, format!("u{user}@test.local")));

    let app = test::init_service(App::new().app_data(web::Data::new(state.clone())).service(
        web::scope("/files").route(
            "/{id}",
            web::put().to(alioth_gateway::api::files::update_file),
        ),
    ))
    .await;
    test::call_service(&app, req).await.into()
}

// ── 列表 total + dk 过滤 ──────────────────────────────────────────────────

#[tokio::test]
async fn list_total_and_dk_filter() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    // 基线：历史残留行（失败 run 遗留）→ 断言用相对增量，不依赖绝对计数
    let resp = list_files(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Query::from_query("page=1&pageSize=20").unwrap(),
    )
    .await
    .expect("list baseline");
    let baseline = json_body(resp).await["total"].as_i64().unwrap();

    let a = json_body(http_upload(&state, USER_A, "Alioth", "a.txt", b"aaa", &[]).await).await;
    let b = json_body(http_upload(&state, USER_A, "Alioth", "b.txt", b"bbb", &[]).await).await;
    let id_a: i64 = a["id"].as_str().unwrap().parse().unwrap();
    let id_b: i64 = b["id"].as_str().unwrap().parse().unwrap();

    // total = 基线 + 2
    let resp = list_files(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Query::from_query("page=1&pageSize=20").unwrap(),
    )
    .await
    .expect("list");
    let list = json_body(resp).await;
    assert_eq!(list["total"], baseline + 2, "total 应为基线+2");

    // 直接 SQL 给 a.txt 挂 dk_scene（HTTP 上传暂不支持 dk 字段）
    sqlx::query(r#"UPDATE isahl."zc_id_file-document" SET dk_scene = -991 WHERE id = $1"#)
        .bind(id_a)
        .execute(&pool)
        .await
        .expect("set dk_scene");

    // scene 过滤 → 仅 a，total=1
    let resp = list_files(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Query::from_query("page=1&pageSize=20&scene=-991").unwrap(),
    )
    .await
    .expect("list scene");
    let list = json_body(resp).await;
    assert_eq!(list["total"], 1);
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id_a.to_string());
    assert_ne!(items[0]["id"], id_b.to_string());

    // 无命中 → total=0
    let resp = list_files(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Query::from_query("page=1&pageSize=20&scene=-999").unwrap(),
    )
    .await
    .expect("list scene miss");
    let list = json_body(resp).await;
    assert_eq!(list["total"], 0);

    cleanup(&pool, &[id_a, id_b]).await;
}

// ── kind↔扩展名 + magic bytes 拒绝 ───────────────────────────────────────

#[tokio::test]
async fn kind_and_magic_rejection() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    let png: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3];
    let zip: &[u8] = b"PK\x03\x04zip-content";

    // image 表 + .docx → 400（kind 扩展名不符）
    let resp = http_upload(
        &state,
        USER_A,
        "Alioth",
        "doc.docx",
        png,
        &[("tableKind", "image")],
    )
    .await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "image 表存 docx 应 400"
    );

    // image 表 + .png 但内容为文本 → 400（magic 不符）
    let resp = http_upload(
        &state,
        USER_A,
        "Alioth",
        "fake.png",
        b"plain text",
        &[("tableKind", "image")],
    )
    .await;
    assert_eq!(
        resp.status(),
        actix_web::http::StatusCode::BAD_REQUEST,
        "png 内容为文本应 400"
    );

    // package 表 + zip 内容 → 200
    let resp = http_upload(
        &state,
        USER_A,
        "Alioth",
        "bundle.zip",
        zip,
        &[("tableKind", "package")],
    )
    .await;
    assert!(
        resp.status().is_success(),
        "package zip 应成功: {}",
        resp.status()
    );
    let up = json_body(resp).await;
    let id_zip: i64 = up["id"].as_str().unwrap().parse().unwrap();

    // document 表 + png 内容（document 全量白名单，magic 校验通过）→ 200
    let resp = http_upload(&state, USER_A, "Alioth", "img.png", png, &[]).await;
    assert!(
        resp.status().is_success(),
        "document png 应成功: {}",
        resp.status()
    );
    let up = json_body(resp).await;
    let id_png: i64 = up["id"].as_str().unwrap().parse().unwrap();

    cleanup(&pool, &[id_zip, id_png]).await;
}

// ── presigned（local → 代理回退）─────────────────────────────────────────

#[tokio::test]
async fn presigned_local_falls_back_to_proxy() {
    let pool = connect_test_db().await;
    let state = setup_state(pool.clone()).await;

    let up = json_body(http_upload(&state, USER_A, "Alioth", "p.txt", b"p", &[]).await).await;
    let file_id: i64 = up["id"].as_str().unwrap().parse().unwrap();

    let resp = alioth_gateway::api::files::presigned_file(
        req_with_user(USER_A, "Alioth"),
        web::Data::new(state.clone()),
        web::Path::from(file_id),
    )
    .await
    .expect("presigned");
    assert!(resp.status().is_success());
    let body = json_body(resp).await;
    assert_eq!(body["proxy"], true, "local 后端应回退代理");
    assert_eq!(body["url"], format!("/api/files/{file_id}"));

    cleanup(&pool, &[file_id]).await;
}
