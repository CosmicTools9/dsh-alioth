#[cfg(test)]
mod quick_pwd_test {
    use gateway_sso::auth::password;

    #[test]
    fn test_verify_hash() {
        let hash = "$argon2id$v=19$m=19456,t=2,p=1$bL9dfsUhrAw/hAiXqCogVg$V1J5k18jf5ReKGJwn6uKqGHEY4MXDZue+QBh0HEHiFc";
        println!("Testing password verification...");
        println!("Hash: {}", &hash[..60]);
        let result = password::verify_password("1111", hash);
        match &result {
            Ok(Some(h)) => println!("PASS: verified, hash={}", &h[..60]),
            Ok(None) => println!("FAIL: password mismatch"),
            Err(e) => println!("ERROR: {:?}", e),
        }
        assert!(result.is_ok(), "verify should succeed");
    }
}
