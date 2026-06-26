# syntax=docker/dockerfile:1

ARG RUST_IMAGE=rust:1-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${RUST_IMAGE} AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --release -p poker-hands-storage-service

FROM ${RUNTIME_IMAGE} AS runtime
ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 phs \
    && useradd --uid 10001 --gid phs --home-dir /nonexistent --shell /usr/sbin/nologin --no-create-home phs \
    && mkdir -p /data

COPY --from=builder /app/target/release/poker-hands-storage-service /usr/local/bin/poker-hands-storage-service

ENV PHS_BIND=0.0.0.0:8080 \
    PHS_DATA_DIR=/data \
    PHS_META_DB=/data/meta.db \
    RUST_LOG=info

EXPOSE 8080

USER 10001:10001

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8080/health >/dev/null || exit 1

ENTRYPOINT ["poker-hands-storage-service"]
CMD ["serve"]
