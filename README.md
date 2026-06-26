# poker-hands-storage

Standalone Rust storage and query service for Preflop Storage range data.

V1 follows the current `preflop-storage` Range Strata Binary contract:

- `manifest.json` with `format = "PFSP"` and `version = 1`
- `meta.db`
- `ranges_{strategy}_{player_count}max_{depth_bb}BB.idx`
- `ranges_{strategy}_{player_count}max_{depth_bb}BB.bin`

The HTTP runtime remains read-only. The same binary also provides an offline
`build` command that converts the legacy SQLite range DB into the V1 files.

## Service module layout

`crates/service/src` is organized by business area:

- `domain`: action schemas, dimensions, and hole-card parsing.
- `storage`: manifest, metadata DB, and dynamically loaded SQLite access.
- `range_store_builder`: SQLite source to PFSP/PFXI binary store build flow.
- `query`: hand query service and dimension handle pool.
- `http` and `routes`: Axum server setup, OpenAPI, validation, and handlers.
- `scripts`: CLI command parsing and command entry points.
- `verification`: standalone and source-cross verification reports.

Service integration tests live under `crates/service/tests` and use explicit
Cargo targets with `<source-file>.test.rs` filenames.

## Build data

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- build `
  --source-db C:\path\to\range.db `
  --out-dir data\range-strata `
  --dimension default:6:100 `
  --overwrite
```

Repeat `--dimension` to select multiple dimensions. Omit it to build all
dimensions. `--max-concrete-lines` is intended for smoke fixtures.

## Query smoke test

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- query `
  --data-dir data\range-strata `
  --player-count 6 `
  --depth-bb 100 `
  --concrete-line-id 1 `
  --hole-cards AA `
  --verify-checksum
```

SQLite is loaded dynamically. Set `PHS_SQLITE3_LIB` when it is not available as
`sqlite3.dll`, `libsqlite3.so.0`, `libsqlite3.so`, or `libsqlite3.dylib`.

## Verify Range Strata output

Standalone verification checks `manifest.json`, `meta.db`, `.idx`, `.bin`,
index-pack cross references, and optional pack CRC32C checksums without reading
the source SQLite DB.

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify `
  --mode standalone `
  --dir data\range-strata `
  --verify-checksum
```

Cross verification first runs standalone checks, then compares source SQLite
rows against decoded binary packs using float32 bit-exact frequency and hand EV
semantics.

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify `
  --mode cross `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --sample-size 10000 `
  --verify-checksum
```

Reports default to `reports/range-strata-verify-standalone.json/.md` and
`reports/range-strata-verify-cross.json/.md`. Use `--sample-size 0` for a full
source scan in cross mode.

## Run the HTTP service

```powershell
$env:PHS_DATA_DIR = "data\smoke"
$env:PHS_META_DB = "data\smoke\meta.db"
$env:PHS_PREWARM = "default:6:100"
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- serve
```

The service exposes:

- `GET /swagger`
- `GET /api-docs/openapi.json`
- `GET /health`
- `GET /ready`
- `POST /query`
- `POST /batch`
- `POST /prewarm`
- `POST /concrete-lines`
- `POST /drill-scenario-lines`

Configuration:

| Variable | Default |
| --- | --- |
| `PHS_BIND` | `0.0.0.0:8080` |
| `PHS_DATA_DIR` | `/data` |
| `PHS_META_DB` | `${PHS_DATA_DIR}/meta.db` |
| `PHS_MAX_OPEN_HANDLES` | `3` |
| `PHS_VERIFY_CHECKSUMS` | `false` |
| `PHS_PREWARM` | empty |
| `RUST_LOG` | `info` |

## API documentation and validation

Scalar API Reference is available from the running service:

```text
http://127.0.0.1:8080/swagger
```

The raw OpenAPI document is available at:

```text
http://127.0.0.1:8080/api-docs/openapi.json
```

Request bodies are validated before query execution. Validation failures use the
standard JSON error shape:

```json
{
  "code": "INVALID_ARGUMENT",
  "message": "request validation failed",
  "details": {
    "fields": [
      { "path": "concrete_line_id", "message": "must be greater than 0" }
    ]
  }
}
```

## Run with Docker

The default compose setup runs the HTTP service against the checked-in smoke
fixture. It mounts `./data/smoke` as `/data:ro`, enables checksum verification,
and prewarms `default:6:100`.

```powershell
docker compose up --build
```

Health and readiness checks:

```powershell
Invoke-RestMethod http://127.0.0.1:8080/health
Invoke-RestMethod http://127.0.0.1:8080/ready
```

Query smoke:

```powershell
$body = @{
  strategy = "default"
  player_count = 6
  depth_bb = 100
  concrete_line_id = 1
  hole_cards = "AA"
} | ConvertTo-Json

Invoke-RestMethod `
  -Uri http://127.0.0.1:8080/query `
  -Method Post `
  -ContentType "application/json" `
  -Body $body
```

For full data, mount a directory containing `manifest.json`, `meta.db`, and the
matching `.idx/.bin` files to `/data:ro`. The runtime image includes
`libsqlite3.so.0` for the dynamic SQLite loader.
