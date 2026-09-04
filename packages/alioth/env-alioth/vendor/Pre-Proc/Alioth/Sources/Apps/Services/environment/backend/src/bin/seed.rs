//! 运行环境开发种子数据运行器
//!
//! 用法：
//!   DATABASE_URL=postgresql://isahl@localhost:5432/alioth cargo run -p alioth-service-environment --bin seed

use sqlx::PgPool;
use std::env;

#[tokio::main]
async fn main() {
    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set — e.g. postgresql://isahl@localhost:5432/alioth");

    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to database");

    let inserted = alioth_service_environment::seed::seed_environments(&pool)
        .await
        .expect("Failed to seed environments");

    println!("✅ Seeded {} environments", inserted);
}
