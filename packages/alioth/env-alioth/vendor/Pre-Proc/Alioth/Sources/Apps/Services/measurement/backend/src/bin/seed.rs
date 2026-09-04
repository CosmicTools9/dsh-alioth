use alioth_service_measurement::seed::{seed_currencies_and_rates, seed_standard_units};
use sqlx::PgPool;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://isahl@localhost:5432/alioth".to_string());

    let pool = PgPool::connect(&database_url).await?;

    let inserted_units = seed_standard_units(&pool).await?;
    let (inserted_currencies, inserted_rates) = seed_currencies_and_rates(&pool).await?;

    println!(
        "✅ Seeded {} standard units, {} currencies, {} exchange rates",
        inserted_units, inserted_currencies, inserted_rates
    );
    Ok(())
}
