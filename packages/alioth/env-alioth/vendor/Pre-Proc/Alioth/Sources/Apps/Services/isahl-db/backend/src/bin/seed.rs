use alioth_service_isahl_db::seed::seed_identities;
use dotenvy::dotenv;

#[tokio::main]
async fn main() {
    dotenv().ok();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect database");

    let inserted = seed_identities(&pool).await.expect("seed identities");
    println!("✅ Seeded {} identities", inserted);
}
