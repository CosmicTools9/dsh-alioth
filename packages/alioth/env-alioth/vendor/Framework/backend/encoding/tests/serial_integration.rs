use encoding::SerialGenerator;

async fn setup_pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&common::testing::test_database_url())
        .await
        .expect("Failed to connect to test database")
}

#[tokio::test]
async fn test_next_sequence_value() {
    let pool = setup_pool().await;
    let gen = SerialGenerator::new();
    gen.ensure_sequence(&pool, "test_enc_seq", 1).await.unwrap();
    let v1 = gen
        .next_sequence_value(&pool, "test_enc_seq")
        .await
        .unwrap();
    let v2 = gen
        .next_sequence_value(&pool, "test_enc_seq")
        .await
        .unwrap();
    assert_eq!(v2, v1 + 1);
}

#[tokio::test]
async fn test_next_serial() {
    let pool = setup_pool().await;
    let gen = SerialGenerator::new();
    gen.ensure_sequence(&pool, "test_enc_serial", 1)
        .await
        .unwrap();
    let s = gen
        .next_serial(&pool, "test_enc_serial", 6, '0')
        .await
        .unwrap();
    assert_eq!(s.len(), 6);
    assert!(s.chars().all(|c| c.is_ascii_digit()));
}
