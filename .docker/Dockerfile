ARG RUST_IMAGE=rust:1-slim-bookworm
ARG RUNTIME_IMAGE=gcr.io/distroless/base-debian12

FROM ${RUST_IMAGE} AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --release -p poker-hands-storage-service

# Extract runtime dependencies from a temp debian container
FROM ${RUST_IMAGE} AS deps-extractor
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

FROM ${RUNTIME_IMAGE} AS runtime

# Copy binary from builder
COPY --from=builder /app/target/release/poker-hands-storage-service /usr/local/bin/poker-hands-storage-service

# Copy sqlite3 shared library
COPY --from=deps-extractor /usr/lib/x86_64-linux-gnu/libsqlite3.so.0 /usr/lib/x86_64-linux-gnu/libsqlite3.so.0
COPY --from=deps-extractor /usr/lib/x86_64-linux-gnu/libsqlite3.so.0.8.6 /usr/lib/x86_64-linux-gnu/libsqlite3.so.0.8.6

# Copy CA certificates
COPY --from=deps-extractor /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=deps-extractor /etc/ssl/certs /etc/ssl/certs

# Copy libgcc_s (required by Rust gnu targets on distroless)
COPY --from=deps-extractor /usr/lib/x86_64-linux-gnu/libgcc_s.so.1 /usr/lib/x86_64-linux-gnu/libgcc_s.so.1

# Copy libssl and libcrypto (dependency of ca-certificates)
COPY --from=deps-extractor /usr/lib/x86_64-linux-gnu/libssl.so.3 /usr/lib/x86_64-linux-gnu/libssl.so.3
COPY --from=deps-extractor /usr/lib/x86_64-linux-gnu/libcrypto.so.3 /usr/lib/x86_64-linux-gnu/libcrypto.so.3

ENV PHS_BIND=0.0.0.0:8080 \
    PHS_DATA_DIR=/data \
    PHS_META_DB=/data/meta.db \
    RUST_LOG=info

EXPOSE 8080

USER nonroot

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/poker-hands-storage-service", "healthcheck", "--url", "http://127.0.0.1:8080/health", "--timeout-ms", "2000"]

ENTRYPOINT ["poker-hands-storage-service"]
CMD ["serve"]
