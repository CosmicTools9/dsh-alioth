use alioth_service_version::seed::seed_versions;
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

    let inserted = seed_versions(&pool).await.expect("seed versions");
    println!("✅ Seeded {} versions", inserted);
}
