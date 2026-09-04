use alioth_service_status::seed::seed_statuses;
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

    let ids = seed_statuses(&pool).await.expect("seed statuses");
    println!("✅ Seeded {} statuses", ids.len());
}
