use alioth_service_language::seed::seed_languages;
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

    seed_languages(&pool).await.expect("seed languages");
    println!("✅ Seeded language packs");
}
