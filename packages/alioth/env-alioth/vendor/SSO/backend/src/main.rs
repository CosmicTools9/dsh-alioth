/// SSO 独立进程入口
///
/// 实现了完整的独立部署。当嵌入 Gateway 时，Gateway 直接调用
/// `gateway_sso::configure_routes` 而非启动此进程。
use gateway_sso::Config;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("[SSO PANIC] {:?}", info);
        let backtrace = std::backtrace::Backtrace::force_capture();
        eprintln!("{:?}", backtrace);
    }));

    eprintln!("Starting AliothStudio SSO Service...");

    let config = Config::from_env().map_err(|e| std::io::Error::other(e.to_string()))?;

    let mut logger_builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(&config.log_level),
    );
    logger_builder.format_timestamp_millis();
    logger_builder.init();
    log::info!("SSO config loaded: server_addr={}", config.server_addr);

    // 系统配置凭证解密密钥（与 Gateway `main.rs` 同契约）：邮箱口令等 DB 内 `enc:` 字段
    // 依赖它解密。缺省时解密不可用（非 `enc:` 的明文凭据不受影响）。
    match std::env::var("SYSTEM_CONFIG_ENC_KEY") {
        Ok(enc_key) => match system_config::crypto::init_encryption(&enc_key) {
            Ok(()) => log::info!("System-config encryption initialized"),
            Err(e) => log::warn!("Failed to initialize system-config encryption: {e}"),
        },
        Err(_) => log::warn!(
            "SYSTEM_CONFIG_ENC_KEY not set, system-config credentials will not be decrypted"
        ),
    }

    let server = gateway_sso::build_server(config).await?;
    server.await
}
