use alioth_service_license::seed::seed_licenses;
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

    let inserted = seed_licenses(&pool).await.expect("seed licenses");
    println!("✅ Seeded {} licenses", inserted);
}
