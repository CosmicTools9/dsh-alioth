use gateway_sso::auth::password;

fn main() {
    println!("Testing argon2id password hashing...");

    let hash1 = password::hash_password("my_password").unwrap();
    let hash2 = password::hash_password("my_password").unwrap();

    println!("Hash 1: {}", hash1);
    println!("Hash 2: {}", hash2);
    println!("Different salts (expected): {}", hash1 != hash2);

    let verify = password::verify_password("my_password", &hash1).unwrap();
    println!("Verify correct: {}", verify.is_some());

    let wrong = password::verify_password("wrong_password", &hash1).unwrap();
    println!("Verify wrong: {}", wrong.is_none());
}
