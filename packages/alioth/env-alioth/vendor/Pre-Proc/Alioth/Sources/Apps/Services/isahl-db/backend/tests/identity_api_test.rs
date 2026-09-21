//! alioth-service-isahl-db 集成测试
//!
//! 验证 entity 因子的 Identity CRUD。

use common::testing::{connect_test_db, setup_test_schema_light};
use crud::AliothRepository;

/// 清场 zc_id_lifecycle：动态收集全部入向外键的引用集（wz_fssc.bill_check_ext /
/// carrier_bill_audit 等扩展表随业务演进持续新增——硬编码清单必腐），只删未被引用行。
async fn clean_lifecycle(pool: &sqlx::PgPool) {
    sqlx::query(
        r#"DO $$
DECLARE
    r record;
BEGIN
    CREATE TEMP TABLE IF NOT EXISTS _keep_lifecycle_ids(id bigint PRIMARY KEY);
    TRUNCATE _keep_lifecycle_ids;
    FOR r IN
        SELECT c.conrelid::regclass::text AS tbl, a.attname AS col
        FROM pg_constraint c
        JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = ANY (c.conkey)
        WHERE c.contype = 'f' AND c.confrelid IN (
            SELECT t.oid FROM pg_class t
            WHERE t.oid = 'isahl.zc_id_lifecycle'::regclass
               OR t.oid IN (
                   WITH RECURSIVE inh AS (
                       SELECT inhrelid FROM pg_inherits
                       WHERE inhparent = 'isahl.zc_id_lifecycle'::regclass
                       UNION ALL
                       SELECT i.inhrelid FROM pg_inherits i JOIN inh ON i.inhparent = inh.inhrelid
                   )
                   SELECT inhrelid FROM inh
               )
        )
    LOOP
        EXECUTE format(
            'INSERT INTO _keep_lifecycle_ids SELECT DISTINCT s.%I FROM %s s WHERE s.%I IS NOT NULL ON CONFLICT DO NOTHING',
            r.col, r.tbl, r.col
        );
    END LOOP;
    -- 叶表定向：本测试族自持 zc_id_orga-non-banking-legal（身份叶表）——
    -- 从族根（zc_id_lifecycle）删会连坐整族后代（law/stan-*/unit 等千表），MUST NOT。
    DELETE FROM isahl."zc_id_orga-non-banking-legal" t
     WHERE NOT EXISTS (SELECT 1 FROM _keep_lifecycle_ids k WHERE k.id = t.id);
    DROP TABLE _keep_lifecycle_ids;
END $$"#,
    )
    .execute(pool)
    .await
    .expect("clean lifecycle");
}

#[tokio::test]
async fn identity_list_with_rls() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    clean_lifecycle(&pool).await;

    let first = alioth_service_isahl_db::seed::seed_identities(&pool)
        .await
        .unwrap();
    // 共享测试库残留（他轮/他用例）：断言种子自身两行在册 + 幂等，不依赖全表精确计数
    assert!(first <= 2, "至多新增 2 行，实得 {first}");

    let repo = identity_org::repository::IdentityRepository::from(pool.clone());

    let query = common::data::ListQuery {
        page: 1,
        page_size: 10,
        filter_field: None,
        filter_op: None,
        filter_value: None,
        sort_field: None,
        sort_order: None,
    };

    let all = repo.list_with_rls(&query, None, None).await.unwrap();
    let seeded: Vec<_> = all
        .items
        .iter()
        .filter(|i| {
            i.code.as_deref() == Some("alioth-platform") || i.code.as_deref() == Some("demo-vendor")
        })
        .collect();
    assert_eq!(
        seeded.len(),
        2,
        "两个种子身份均应在列（total={}）",
        all.total
    );

    let visible_id = all.items[0].id;
    let filtered = repo
        .list_with_rls(&query, Some(&[visible_id]), None)
        .await
        .unwrap();
    assert_eq!(filtered.total, 1, "RLS filter should return 1 identity");
    assert_eq!(filtered.items[0].id, visible_id);
}

#[tokio::test]
async fn identity_list_with_null_notice_record() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    clean_lifecycle(&pool).await;

    // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve(&pool, ("JE", "FJA", "↑_DA"))
            .await
            .expect("resolve zc_id_subjects coords");
    sqlx::query(
        // 裸夹具主体（无业务语义）→ 落叶 zc_id_orga-non-banking-legal（父表禁直写）
        r#"INSERT INTO isahl."zc_id_orga-non-banking-legal" (notice, code, created_by_id, _f_, _t_, dk_scene, dk_factor, dk_function) VALUES (NULL, NULL, 1, '实现', '实例', $1, $2, $3)"#,
    )
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .execute(&pool)
    .await
    .unwrap();

    let repo = identity_org::repository::IdentityRepository::from(pool.clone());
    let query = common::data::ListQuery {
        page: 1,
        page_size: 10,
        filter_field: None,
        filter_op: None,
        filter_value: None,
        sort_field: None,
        sort_order: None,
    };

    let all = repo.list_with_rls(&query, None, None).await.unwrap();
    // 共享测试库残留：定位本用例的裸夹具行（notice/code 皆 NULL），不做全表精确计数
    let bare = all
        .items
        .iter()
        .find(|i| i.code.is_none() && i.name.is_empty())
        .expect("裸夹具身份应在列（notice/code 皆 NULL）");
    assert_eq!(bare.name, "", "null notice should decode as empty name");
    assert_eq!(bare.code, None, "null code should decode as None");
}

#[tokio::test]
async fn identity_crud() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    clean_lifecycle(&pool).await;
    let uid: i64 = 1;

    let repo = identity_org::repository::IdentityRepository::from(pool.clone());
    let created = repo
        .create(
            identity_org::models::CreateIdentityRequest {
                name: "测试身份".to_string(),
                subject_type: "group".to_string(),
                code: Some("TEST-001".to_string()),
                notice: Some("这是一个测试身份".to_string()),
            },
            uid,
        )
        .await
        .unwrap();

    assert_eq!(created.name, "测试身份");
    assert_eq!(created.code.as_deref(), Some("TEST-001"));

    let fetched = repo.get(created.id).await.unwrap().unwrap();
    assert_eq!(fetched.name, "测试身份");
    assert_eq!(fetched.code.as_deref(), Some("TEST-001"));

    let updated = repo
        .update(
            created.id,
            identity_org::models::UpdateIdentityRequest {
                name: Some("更新身份".to_string()),
                code: None,
                notice: None,
                comments: None,
                mdm_codes: None,
                mdm_code: None,
            },
            uid,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.name, "更新身份");
    assert_eq!(updated.code.as_deref(), Some("TEST-001"));

    repo.delete(created.id, uid).await.unwrap();
    assert!(repo.get(created.id).await.unwrap().is_none());
}

#[tokio::test]
async fn identity_seed_idempotent() {
    let pool = connect_test_db().await;
    setup_test_schema_light(&pool).await.unwrap();
    clean_lifecycle(&pool).await;

    let first = alioth_service_isahl_db::seed::seed_identities(&pool)
        .await
        .unwrap();
    let second = alioth_service_isahl_db::seed::seed_identities(&pool)
        .await
        .unwrap();

    assert_eq!(first, 2, "should insert 2 identities on first run");
    assert_eq!(second, 0, "should be idempotent on second run");
}
