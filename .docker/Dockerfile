ARG RUST_IMAGE=rust:1-slim-bookworm
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian12

FROM ${RUST_IMAGE} AS builder
WORKDIR /app

COPY .docker/Cargo.service.toml ./Cargo.toml
COPY .docker/Cargo.service.lock ./Cargo.lock
COPY range-store-core ./range-store-core
COPY storage-tools ./storage-tools
COPY service ./service

RUN cargo build --release --locked -p poker-hands-storage-service

FROM ${RUNTIME_IMAGE} AS runtime

# Copy binary from builder
COPY --from=builder /app/target/release/poker-hands-storage-service /usr/local/bin/poker-hands-storage-service

ENV PHS_BIND=0.0.0.0:8080 \
    PHS_DATA_DIR=/data \
    PHS_METADATA_CACHE_BYTES=8388608 \
    PHS_STRATEGY_CACHE_BYTES=67108864 \
    RUST_LOG=info

EXPOSE 8080

USER nonroot

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/poker-hands-storage-service", "healthcheck", "--url", "http://127.0.0.1:8080/health", "--timeout-ms", "2000"]

ENTRYPOINT ["poker-hands-storage-service"]
CMD ["serve"]
