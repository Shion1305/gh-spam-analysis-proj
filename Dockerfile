# syntax=docker/dockerfile:1.6

ARG RUST_VERSION=1.84

FROM rust:${RUST_VERSION} AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev clang && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml .
COPY Cargo.lock .
COPY crates crates
COPY migrations migrations
COPY .config .config
COPY .sqlx .sqlx

ENV SQLX_OFFLINE=true
RUN cargo build --release -p api -p collector

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*
ENV RUST_LOG=info
WORKDIR /srv/app
COPY --from=builder /app/target/release/api /usr/local/bin/api
COPY --from=builder /app/target/release/collector /usr/local/bin/collector

CMD ["/usr/local/bin/api"]
