# AliothStudio — SSO Backend Docker Image
# Build context: project root
#
# SAML 2.0 已移除（remove-saml-sso）：xmlsec1 源码构建不再需要。
# builder 仅保留 openssl 系构建依赖（webauthn-rs → openssl-sys 需要 pkg-config/libssl-dev）。

FROM rust:1.96-slim AS builder

WORKDIR /usr/src/app

RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# sqlx 宏离线模式（对应宿主 .cargo/config.toml 的 SQLX_OFFLINE=true；
# 离线数据在 SSO/backend/.sqlx，随 COPY . . 进入）
ENV SQLX_OFFLINE=true

COPY . .

RUN cargo build --release -p gateway-sso

FROM debian:trixie-slim

RUN apt-get update && \
    apt-get install -y ca-certificates libpq5 && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/app/target/release/gateway-sso /usr/local/bin/gateway-sso

EXPOSE 9002

ENV RUST_LOG=info
ENV SERVER_ADDR=0.0.0.0:9002

CMD ["gateway-sso"]
