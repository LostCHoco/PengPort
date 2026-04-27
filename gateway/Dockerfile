# syntax=docker/dockerfile:1.7
# pengport-gateway 멀티스테이지 빌드.
# workspace root Cargo.toml 의 player-launcher 멤버만 제거하고
# 전체 소스를 한 번에 빌드한다.

FROM rust:1-bookworm AS builder

WORKDIR /build

COPY Cargo.toml ./
RUN sed -i '/player-launcher/d' Cargo.toml

COPY shared shared
COPY gateway gateway
COPY adapter-minecraft adapter-minecraft

RUN cargo build --release -p pengport-gateway \
 && strip target/release/gateway


FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/gateway /usr/local/bin/gateway

ENV RUST_LOG=info
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/gateway"]
