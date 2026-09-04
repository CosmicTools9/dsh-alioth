use gateway_sso::auth::password::hash_password;

#[test]
fn gen_admin_hash() {
    let hash = hash_password("admin123").unwrap();
    println!("HASH: {}", hash);
}
