# poker-hands-storage

Standalone Rust storage and query service for Preflop Storage range data.

V1 follows the current `preflop-storage` Range Strata Binary contract:

- `manifest.json` with `format = "PFSP"` and `version = 1`
- `meta.db`
- `ranges_{strategy}_{player_count}max_{depth_bb}BB.idx`
- `ranges_{strategy}_{player_count}max_{depth_bb}BB.bin`

The HTTP runtime remains read-only. The same binary also provides an offline
`build` command that converts the legacy SQLite range DB into the V1 files.

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

The authoritative standalone verifier remains in `preflop-storage`.

## Run the HTTP service

```powershell
$env:PHS_DATA_DIR = "data\smoke"
$env:PHS_META_DB = "data\smoke\meta.db"
$env:PHS_PREWARM = "default:6:100"
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- serve
```

The service exposes:

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
