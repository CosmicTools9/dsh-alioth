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

    let server = gateway_sso::build_server(config).await?;
    server.await
}
