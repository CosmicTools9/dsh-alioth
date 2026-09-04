use gateway_sso::auth::password;
use std::env;

fn main() {
    let password = env::args()
        .nth(1)
        .or_else(|| env::var("PASSWORD").ok())
        .unwrap_or_else(|| {
            eprintln!("用法: hash_password <password>");
            eprintln!("或者设置 PASSWORD 环境变量");
            std::process::exit(1);
        });
    match password::hash_password(&password) {
        Ok(hash) => {
            println!("Password: {}", password);
            println!("Hash: {}", hash);
        }
        Err(e) => {
            eprintln!("Failed to hash password: {}", e);
        }
    }
}
