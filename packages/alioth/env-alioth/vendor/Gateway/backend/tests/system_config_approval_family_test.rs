//! env 配置族（`approval` 分类 → `zc_id_prot-env_config`）集成测试
//! （add-identity-verify-and-approval-config-gui）
//!
//! 覆盖：find_by_code 命中 approval 族（`_f_`/`_t_` 投影）→ update 翻转
//! settings.enabled → 再读回验证；fixture 自建自清（测试库共享，code 幂等键）。

use ::common::testing::connect_test_db;
use alioth_gateway::system_config_repo::SystemConfigRepo;
use system_config::{SystemConfigRepository, UpdateSystemConfigRequest};

const CODE: &str = "approval:auto-approve";

#[tokio::test]
async fn approval_family_roundtrip() {
    let pool = connect_test_db().await;

    // fixture：确保开关行存在且带 provider 键（与模型级种子 §5/5b 同构）
    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE code = $1"#)
        .bind(CODE)
        .execute(&pool)
        .await
        .expect("clear switch row");
    sqlx::query(
        r#"INSERT INTO isahl."zc_id_prot-env_config"
             (notice, code, settings, created_by_id, dk_scene, dk_factor, dk_function)
           VALUES ('注册审批自动通过（test）', $1,
                   jsonb_build_object('enabled', false, 'provider', 'platform'), 1,
                   (SELECT id FROM isahl.zc_id_scene    WHERE code = 'JE'   AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_factor   WHERE code = 'GEC'  AND deleted_at IS NULL),
                   (SELECT id FROM isahl.zc_id_function WHERE code = '↑_DA' AND deleted_at IS NULL))"#,
    )
    .bind(CODE)
    .execute(&pool)
    .await
    .expect("insert switch fixture");

    let repo = SystemConfigRepo::new(pool.clone());

    // 1. find_by_code 命中 approval 族，投影分类/提供商正确
    let cfg = repo
        .find_by_code(CODE)
        .await
        .expect("find_by_code")
        .expect("switch row must be found");
    assert_eq!(
        cfg._f_.as_deref(),
        Some("approval"),
        "分类 MUST 为 approval"
    );
    assert_eq!(
        cfg._t_.as_deref(),
        Some("platform"),
        "provider MUST 投影 platform"
    );
    assert_eq!(cfg.enabled, Some(false), "初始 enabled=false");

    // 2. update 翻转 enabled（GUI 编辑同路径：_f_ 必填，settings 走合并）
    let updated = repo
        .update(
            cfg.id,
            &UpdateSystemConfigRequest {
                notice: None,
                code: None,
                _f_: Some("approval".to_string()),
                _t_: Some("platform".to_string()),
                comments: None,
                credentials: None,
                settings: None,
                enabled: Some(true),
                is_default: None,
                public: None,
                domain_: None,
            },
        )
        .await
        .expect("update")
        .expect("updated row must be returned");
    assert_eq!(
        updated.enabled,
        Some(true),
        "update 后 enabled MUST 为 true"
    );

    // 3. 再读回：settings.enabled 持久化且 provider 键未丢
    let reread = repo
        .find_by_code(CODE)
        .await
        .expect("re-read")
        .expect("switch row must persist");
    assert_eq!(reread.enabled, Some(true), "enabled MUST 持久化");
    assert_eq!(
        reread._t_.as_deref(),
        Some("platform"),
        "update MUST NOT 丢 provider 键"
    );

    // 清理
    sqlx::query(r#"DELETE FROM isahl."zc_id_prot-env_config" WHERE code = $1"#)
        .bind(CODE)
        .execute(&pool)
        .await
        .ok();
}
