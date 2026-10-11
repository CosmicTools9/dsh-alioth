//! 生效配置解析（`common::config_resolution`）集成测试
//! （change fix-config-effective-resolution）
//!
//! 覆盖语义与回归（单用例串行，fixture 自建自清）：
//! - GUI 形状写入（无 `public` 键）即被选中——旧 email/sms 消费者按
//!   `public IS NOT FALSE` 过滤会跳过此类行的缺陷回归；
//! - `is_default=true` 胜出（压过更新的显式 false 行）；
//! - `NULLS LAST`：缺 `is_default` 键的行 MUST NOT 压过显式默认行（即使更新）；
//! - `enabled=false` 排除；
//! - `load_by_id` 免 enabled 门（编辑/探测路径）；
//! - storage `list_active` 决胜序（默认行在先）。
//!
//! fixture 时间戳取远期（2099）以稳定压过测试库既有种子行；断言只针对
//! 自建行的**相对序**与命中值，不预设全表唯一。

use ::common::testing::connect_test_db;
use common::config_resolution::{list_active, load_by_id, resolve_active, ConfigFamily};
use sqlx::PgPool;

/// 免解密透传（本测试只验证取行/投影；`enc:` 解密口径由 crypto 单测覆盖）
fn passthrough(s: &str) -> String {
    s.to_string()
}

const CODE_DEFAULT: &str = "test:effective-resolution-default";
const CODE_EXPLICIT_FALSE: &str = "test:effective-resolution-false";
const CODE_NO_KEY: &str = "test:effective-resolution-nokey";
const CODE_DISABLED: &str = "test:effective-resolution-disabled";
const CODE_STORAGE_DEFAULT: &str = "test:effective-resolution-storage-default";
const CODE_STORAGE_ALT: &str = "test:effective-resolution-storage-alt";

async fn clear_fixtures(pool: &PgPool) {
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_prot-llm_config" WHERE code LIKE 'test:effective-resolution%'"#,
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        r#"DELETE FROM isahl."zc_id_prot-oss_config" WHERE code LIKE 'test:effective-resolution%'"#,
    )
    .execute(pool)
    .await
    .ok();
}

async fn insert_llm(pool: &PgPool, code: &str, settings: serde_json::Value, updated_at: &str) {
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_prot-llm_config"
             (notice, code, settings, enc_fields, updated_at, created_by_id,
              dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, '{"api_key": "enc:fixture-key"}'::jsonb, $4::timestamptz, 1,
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'JE'   AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'GEC'  AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↑_DA' AND deleted_at IS NULL))"#,
    )
    .bind(format!("生效解析夹具 {code}"))
    .bind(code)
    .bind(settings)
    .bind(updated_at)
    .execute(pool)
    .await
    .expect("insert llm fixture");
}

async fn insert_storage(pool: &PgPool, code: &str, settings: serde_json::Value, updated_at: &str) {
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_prot-oss_config"
             (notice, code, settings, updated_at, created_by_id,
              dk_scene, dk_factor, dk_function)
           VALUES ($1, $2, $3, $4::timestamptz, 1,
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'JE'   AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'GEC'  AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↑_DA' AND deleted_at IS NULL))"#,
    )
    .bind(format!("生效解析夹具 {code}"))
    .bind(code)
    .bind(settings)
    .bind(updated_at)
    .execute(pool)
    .await
    .expect("insert storage fixture");
}

#[tokio::test]
async fn effective_resolution_semantics() {
    let pool = connect_test_db().await;
    clear_fixtures(&pool).await;

    // ── fixture：GUI 形状（无 public 键）四行 + storage 两行 ──
    // A：显式默认（较旧）；B：显式 false（更新）；C：缺键（最新）；D：禁用（最新，仍被门排除）
    insert_llm(
        &pool,
        CODE_DEFAULT,
        serde_json::json!({"enabled": true, "is_default": true, "provider": "deepseek", "model": "fixture-a"}),
        "2099-01-02T00:00:00Z",
    )
    .await;
    insert_llm(
        &pool,
        CODE_EXPLICIT_FALSE,
        serde_json::json!({"enabled": true, "is_default": false, "provider": "deepseek", "model": "fixture-b"}),
        "2099-01-03T00:00:00Z",
    )
    .await;
    insert_llm(
        &pool,
        CODE_NO_KEY,
        serde_json::json!({"enabled": true, "provider": "deepseek", "model": "fixture-c"}),
        "2099-01-04T00:00:00Z",
    )
    .await;
    insert_llm(
        &pool,
        CODE_DISABLED,
        serde_json::json!({"enabled": false, "is_default": true, "provider": "deepseek", "model": "fixture-d"}),
        "2099-01-05T00:00:00Z",
    )
    .await;

    // ── 1. winner-pick：is_default 胜出 + NULLS LAST + enabled 门 ──
    let winner = resolve_active(&pool, ConfigFamily::Llm, &passthrough)
        .await
        .expect("resolve_active")
        .expect("fixture 行必须可解析");
    assert_eq!(
        winner.code.as_deref(),
        Some(CODE_DEFAULT),
        "显式默认行 MUST 胜出（压过更新的 false 行与缺键行；禁用行被门排除）"
    );
    assert_eq!(winner.provider.as_deref(), Some("deepseek"));
    assert_eq!(winner.settings["model"], serde_json::json!("fixture-a"));
    assert_eq!(
        winner.credentials["api_key"],
        serde_json::json!("enc:fixture-key"),
        "credentials 经注入解密器投影（透传）"
    );

    // ── 2. load_by_id 免 enabled 门（编辑/探测路径按 id 直取）──
    let disabled_id: i64 = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_prot-llm_config" WHERE code = $1 AND deleted_at IS NULL"#,
    )
    .bind(CODE_DISABLED)
    .fetch_one(&pool)
    .await
    .expect("disabled row id");
    let by_id = load_by_id(&pool, ConfigFamily::Llm, disabled_id, &passthrough)
        .await
        .expect("load_by_id")
        .expect("禁用行按 id 仍可取");
    assert_eq!(by_id.settings["model"], serde_json::json!("fixture-d"));

    // ── 3. storage list_active 决胜序（默认行先于显式 false 行）──
    insert_storage(
        &pool,
        CODE_STORAGE_ALT,
        serde_json::json!({"enabled": true, "is_default": false, "provider": "local"}),
        "2099-01-03T00:00:00Z",
    )
    .await;
    insert_storage(
        &pool,
        CODE_STORAGE_DEFAULT,
        serde_json::json!({"enabled": true, "is_default": true, "provider": "local"}),
        "2099-01-02T00:00:00Z",
    )
    .await;
    let rows = list_active(&pool, ConfigFamily::Storage, &passthrough)
        .await
        .expect("list_active");
    let pos_default = rows
        .iter()
        .position(|r| r.code.as_deref() == Some(CODE_STORAGE_DEFAULT))
        .expect("默认行在列表中");
    let pos_alt = rows
        .iter()
        .position(|r| r.code.as_deref() == Some(CODE_STORAGE_ALT))
        .expect("备选行在列表中");
    assert!(
        pos_default < pos_alt,
        "决胜序：is_default 行 MUST 排在显式非默认行之前"
    );

    clear_fixtures(&pool).await;
}
