# syntax=docker/dockerfile:1.6

ARG RUST_VERSION=1.84

FROM rust:${RUST_VERSION} AS chef
WORKDIR /app
RUN cargo install cargo-chef --locked
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM rust:${RUST_VERSION} AS builder
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev clang && rm -rf /var/lib/apt/lists/*
ENV SQLX_OFFLINE=true
COPY --from=chef /usr/local/cargo/bin/cargo-chef /usr/local/cargo/bin/cargo-chef
COPY --from=chef /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo build --release -p api -p collector

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
ENV RUST_LOG=info
WORKDIR /srv/app
COPY --from=builder /app/target/release/api /usr/local/bin/api
COPY --from=builder /app/target/release/collector /usr/local/bin/collector

CMD ["/usr/local/bin/api"]
