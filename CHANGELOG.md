# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-07-04

### Added

#### Core Storage Engine (`range-store-core`)

- **PFSP binary format** — Custom binary storage for preflop range data with 16-byte header (magic `PFSP`, version 1, little-endian, Float32, sparse hand-major v1, no compression)
- **PFXI index format** — Fixed 16-byte header + 22-byte dense index records per concrete line, enabling O(1) direct lookup by `concrete_line_id`
- **Pack encoding** — Sparse hand-major layout: sorted `hand_ids` (u8), `action_masks` (u32 bitset), interleaved `frequency`/`hand_ev` (f32). Formula: `hand_count * (5 + action_count * 8)` bytes
- **Dense index lookup** — `.idx` records validated as contiguous at open time; lookup via `index = concrete_line_id - first_concrete_line_id` instead of binary search
- **Action schema system** — Deduplicated action definitions stored in `meta.db` (`action_schemas` table), each action serialized as 9 bytes (`action_type` u8 + `action_size` f32 + `amount_bb` f32), supporting 6 action types: fold, call, check, bet, raise, allin
- **Hand ID encoding** — 169-hole-card dictionary based on 13x13 matrix (pairs on diagonal, suited above, offsuit below), mapping normalized codes like `AA`/`AKs`/`AKo` to u8 values 0..168
- **CRC32C checksum** — Per-pack integrity verification for both build-time validation and runtime optional checking
- **mmap-based zero-copy reading** — Both `.idx` and `.bin` files memory-mapped for minimal RAM overhead with OS-managed page faults
- **LRU handle pool** — Thread-safe `DimensionReader` pool with configurable max open handles (`PHS_MAX_OPEN_HANDLES`)
- **StoreQueryService** — Unified query interface supporting single-hand, batch, and hands-by-actions queries with error-tolerant batch responses
- **Hole card parser** — Supports both standardized 169-hand codes (`AA`, `AKs`, `AKo`) and suited two-card input (`AsKh`, `AcAd`), with automatic normalization

#### HTTP Service (`service`)

- **REST API** with 8 endpoints under `/range/*`:
  - `GET /health` — Process liveness check
  - `GET /ready` — Data directory readiness (at least one queryable dimension loaded)
  - `POST /range/drill-scenarios` — Query available abstract lines for a drill scenario
  - `POST /range/concrete-lines` — Resolve abstract/concrete line to `concrete_line_id`
  - `POST /range/hand-strategy` — Query action strategy for a specific hand
  - `POST /range/hand-strategy-batch` — Batch query multiple hands (max 500)
  - `POST /range/hands-by-actions` — Filter hands by action types and frequency threshold
  - `POST /range/prewarm` — Pre-open dimension readers (max 64 dimensions)
- **OpenAPI/Swagger documentation** at `/swagger` and `/api-docs/openapi.json`
- **Structured error codes** — Business error codes (1000, 404, 500, 503) mapped from internal errors with consistent `{code, data, message}` envelope
- **Configuration** via environment variables with sensible defaults

#### Offline Tools (`storage-tools`)

- **Build orchestrator** — Full pipeline: discover dimensions from SQLite `range_data_*` tables, create `meta.db`, encode packs, write `.bin`/`.idx`, generate `manifest.json`, support `--resume` for interrupted builds via `build-state.json`
- **Standalone verification** — Validate `manifest.json`, `meta.db` catalog (action_schemas integrity, CRC32C, schema_key), `.idx` headers and record continuity, `.bin` headers, index-pack cross-references (offset, byte_length, CRC32C, hand_id range/order)
- **Cross verification** — Sampled or full (0 = all) comparison against source SQLite with cell-by-cell action name, size, amount, frequency, and hand_ev matching
- **Float32 bit-exact precision strategy** — `expected_bits == actual_bits` for both frequency and hand_ev, NaN preserved as null
- **Hot benchmark** — Random workload across all 9 dimensions, measuring p50/p95/p99 latency for hand-strategy, batch (sizes 1/5/10/20/50/100), and hands-by-actions
- **Cold benchmark** — Process-cold comparison including service open, dimension prewarm, first query decode, and service close phases
- **Benchmark compare** — Side-by-side Binary vs SQLite with workload compatibility verification and result matching
- **Workload generation** — Deterministic workload JSON with `random` and `abstract-local` modes, seeded for reproducibility

#### Docker Deployment

- **Multi-stage Dockerfile** — Builder (`rust:1-slim-bookworm`) -> deps-extractor (`libsqlite3-0`, CA certs) -> distroless runtime (`gcr.io/distroless/base-debian12`)
- **Service-only build** — `.docker/Cargo.service.toml` excludes `storage-tools` from service image, preventing benchmark/verification changes from invalidating cache
- **Docker Compose** — Local development with health/ready probes, volume mounting, environment variable overrides
- **Kubernetes manifest** — ConfigMap, PVC (read-only), Deployment with resource limits, ClusterIP Service, readiness/liveness probes
- **Security posture** — Read-only filesystem, non-root user, `cap_drop: ALL`, `no-new-privileges`
- **Versioned release workflow** — Timestamped data directories (`2026-07-02T230000Z/`) for safe rollbacks

#### Documentation

- **`README.md`** — Project entrypoint with module responsibilities, setup, common commands, runtime environment variables, and document entrypoints
- **`docs/README.md`** — Document map with authority boundaries, reading paths, module boundaries, and report update rules
- **`docs/roadmap.md`** — Current remaining work, acceptance criteria, and out-of-scope items
- **`docs/native-sdk.md`** — Current Bun/Node native SDK API, build/test commands, and production integration boundary
- **`docs/data-flow-overview.md`** — Complete build-to-runtime data flow with hand_id definition, binary search rationale, and file format quick reference
- **`docs/range-db-binary-storage-design.md`** — Binary format spec, file sizes, dimension breakdown, pack encoding, query flow
- **`docs/api-business-contract.md`** — Full HTTP API contract with request/response bodies, error codes, validation rules, line-transition combination example
- **`docs/data-verification-and-format-validation.md`** — Standalone/cross verify commands, validation checklist, Float32 precision strategy, pre-release checklist
- **`docs/binary-vs-sqlite-benchmark-and-verification-report.md`** — Comprehensive performance comparison: 76% disk savings, 6.4x-36.2x QPS improvement on strategy queries, 100% data integrity across 23.8M records
- **`docs/docker-deployment-guide.md`** — Build, Compose, K8s, smoke queries, prewarm strategy, rollback procedure

#### Agent Support

- **`.agents/SKILL.md`** — Global project instructions for AI coding assistants (compile rules, architecture boundaries, operational workflows)
- **`.agents/references/`** — Progressive disclosure references: `build.md`, `verify.md`, `benchmark.md`, `service.md`
- **Agent Skills protocol** — Compatible with Claude Code, Gemini, and other AI coding clients

### Performance Highlights

| Metric | Result |
|---|---|
| Disk space saved | 76% (1,447 MB SQLite -> 346 MB Binary) |
| Single hand query QPS | 6.4x faster than SQLite |
| Batch query QPS (size 20) | 36.2x faster than SQLite |
| Hands-by-actions QPS | 9.45x faster than SQLite |
| Data integrity | 23.8M records, 0 failures, float32 bit-exact |
| Concurrent reads | mmap + RwLock, lock-free for readers |

### Architecture

Three-crate workspace with clear boundaries:

```
range-store-core    ← shared storage engine (no HTTP, no tools)
    ^
    |--- service    ← HTTP API (axum, OpenAPI)
    |--- storage-tools  ← offline build/verify/benchmark
```

Hybrid storage: `meta.db` (SQLite) for relational metadata, `.idx/.bin` (custom binary) for hot strategy data. Source SQLite `range.db` is build input only, not served at runtime.
