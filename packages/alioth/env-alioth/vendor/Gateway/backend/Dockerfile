# AliothStudio — Gateway Backend Docker Image
# Build context: project root
#
# 编译依赖：
#   1. trigger-registry build.rs 在编译期查询 isahl_meta.meta_collections 生成继承图
#      （Framework/backend/trigger-registry/build.rs）。构建时必须提供 BUILD_DATABASE_URL
#      （指向含 isahl_meta 的 PostgreSQL；容器内用 host.docker.internal 访问宿主机）：
#        docker build -f Gateway/backend/Dockerfile -t gateway-backend:dev \
#          --build-arg BUILD_DATABASE_URL=postgres://USER@host.docker.internal:5432/aliothstudio_dev .
#   2. SAML 2.0 已移除（remove-saml-sso）：xmlsec1 源码构建不再需要。
#      builder 仅保留 openssl 系构建依赖（webauthn-rs → openssl-sys 需要 pkg-config/libssl-dev）。

FROM rust:1.96-slim AS builder

WORKDIR /usr/src/app

RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev postgresql-client && \
    rm -rf /var/lib/apt/lists/*

# sqlx 宏离线模式（对应宿主 .cargo/config.toml 的 SQLX_OFFLINE=true；
# 离线数据在 Gateway/backend/.sqlx 与 SSO/backend/.sqlx，随 COPY . . 进入）
ENV SQLX_OFFLINE=true

COPY . .

# 可选 feature 编译（由 build-docker.sh --namespace 传入）
ARG GATEWAY_FEATURE=default
# 编译期 DB（trigger-registry build.rs 读取；不传则回退 DATABASE_URL/本地默认并失败）
ARG BUILD_DATABASE_URL
RUN cargo build --release -p alioth-gateway --no-default-features --features "$GATEWAY_FEATURE"

FROM debian:trixie-slim

RUN apt-get update && \
    apt-get install -y ca-certificates libpq5 && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/app/target/release/alioth-gateway /usr/local/bin/alioth-gateway

EXPOSE 9001

ENV RUST_LOG=info
ENV SERVER_ADDR=0.0.0.0:9001

CMD ["alioth-gateway"]
