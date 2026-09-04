use gateway_sso::auth::password;

fn main() {
    let pw = "isahl123";

    // 当前标准算法：先 hash 再 verify（替代已移除的 aes256gcm 遗留格式）
    let hash = match password::hash_password(pw) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("❌ Failed to hash password: {}", e);
            return;
        }
    };

    println!("Testing password verification (standard argon2id)...");
    println!("Password: {}", pw);
    println!("Hash: {}", hash);

    match password::verify_password(pw, &hash) {
        Ok(Some(_)) => println!("✅ Password verified successfully!"),
        Ok(None) => println!("❌ Password verification returned None"),
        Err(e) => println!("❌ Password verification failed: {}", e),
    }

    // 错误密码应被拒绝
    match password::verify_password("wrong-password", &hash) {
        Ok(Some(_)) => println!("❌ Wrong password was incorrectly accepted!"),
        Ok(None) => println!("✅ Wrong password correctly rejected"),
        Err(_) => println!("✅ Wrong password correctly rejected"),
    }
}
