//! 运维小工具：为 `isahl_auth.outbound_client.app_secret_enc` 生成密文。
//!
//! 用法：
//! ```bash
//! WZ_FSSC_ENC_KEY=<32字节base64> PLAINTEXT=<明文secret> \
//!   cargo run -q -p outbound-client --example encrypt
//! ```
//! 输出 `enc:<nonce_b64>:<ciphertext_b64>`，可直接写入注册表。

fn main() {
    let plaintext = std::env::var("PLAINTEXT").expect("须设置 PLAINTEXT 环境变量");
    let enc = outbound_client::crypto::encrypt(&plaintext).expect("加密失败");
    println!("{}", enc);
}
