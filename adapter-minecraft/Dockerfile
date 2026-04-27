# syntax=docker/dockerfile:1.7
# adapter-minecraft 멀티스테이지 빌드.

FROM rust:1-bookworm AS builder

WORKDIR /build

COPY Cargo.toml ./
RUN sed -i '/player-launcher/d' Cargo.toml

COPY shared shared
COPY adapter-minecraft adapter-minecraft
COPY gateway gateway

RUN cargo build --release -p pengport-adapter-minecraft \
 && strip target/release/adapter-minecraft


FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/adapter-minecraft /usr/local/bin/adapter-minecraft

ENV RUST_LOG=info
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/adapter-minecraft"]
