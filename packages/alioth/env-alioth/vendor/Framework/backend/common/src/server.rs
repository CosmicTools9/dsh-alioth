//! 服务器启动基础设施：监听地址绑定错误的统一增强。

use std::io;

/// 将 socket 绑定失败错误增强为包含监听地址的明确错误。
///
/// actix `HttpServer::bind` 直接 `?` 传播时，`AddrInUse`（os error 48）只会显示
/// "Address already in use"，不含被占用的地址，无法定位冲突进程。本函数统一补上
/// 地址上下文与排查指引，其余绑定失败（权限不足、地址格式非法等）同样带上地址。
pub fn bind_error(addr: &str, err: io::Error) -> io::Error {
    let kind = err.kind();
    let msg = if kind == io::ErrorKind::AddrInUse {
        let hint = addr
            .rsplit_once(':')
            .filter(|(_, p)| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            .map(|(_, port)| format!("可用 `lsof -nP -iTCP:{port} -sTCP:LISTEN` 查看占用进程；"))
            .unwrap_or_default();
        format!(
            "监听地址 {addr} 已被占用（Address already in use, os error 48）。\
             可能原因：同名服务实例已在运行，或该端口被其他进程占用。\
             {hint}或更换监听端口后重试。原始错误：{err}"
        )
    } else {
        format!("绑定监听地址 {addr} 失败：{err}")
    };
    io::Error::new(kind, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用真实 OS 错误复现：占住一个端口再绑同端口，得到原生 `AddrInUse`，
    /// 断言增强后信息包含被占用地址与排查指引（不再只有 "Address already in use"）。
    #[test]
    fn addr_in_use_error_includes_bound_address() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let raw = std::net::TcpListener::bind(addr).unwrap_err();
        assert_eq!(raw.kind(), io::ErrorKind::AddrInUse);

        let enriched = bind_error(&addr.to_string(), raw);
        let msg = enriched.to_string();
        assert!(
            msg.contains(&addr.to_string()),
            "错误信息应包含被占用地址 {addr}: {msg}"
        );
        assert!(msg.contains("lsof"), "错误信息应含 lsof 排查指引: {msg}");
    }

    /// 非 AddrInUse 的绑定失败（如权限不足）同样带上地址上下文。
    #[test]
    fn non_addr_in_use_error_still_includes_address() {
        let raw = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");
        let enriched = bind_error("127.0.0.1:4949", raw);
        let msg = enriched.to_string();
        assert!(msg.contains("127.0.0.1:4949"), "应包含地址: {msg}");
        assert_eq!(enriched.kind(), io::ErrorKind::PermissionDenied);
    }
}
