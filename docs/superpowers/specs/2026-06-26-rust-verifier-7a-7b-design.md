# Rust Verifier 7a-7b Design

## Scope

This design covers the Rust migration of the upstream Range Strata Binary verifier:

- `verify --mode standalone`
- `verify --mode cross`
- low-level `.idx` / `.bin` / pack traversal APIs required by both modes
- JSON and Markdown integrity reports compatible with the upstream report shape

Benchmark work is intentionally out of scope for this design. The verifier APIs should be reusable by the future benchmark implementation, especially workload validation and source DB result comparison, but benchmark CLI commands and performance reports will be designed separately.

## Upstream Reference

The behavior is based on the sibling repository at `E:\idea_project\preflop-storage`, especially:

- `src/range-strata-binary/cli/verify.ts`
- `src/range-strata-binary/integrity/self-check.ts`
- `src/range-strata-binary/integrity/cross-check.ts`
- `src/range-strata-binary/integrity/checks/*.ts`
- `src/precision/float32.ts`

The current Rust repository already has matching PFSP/PFXI readers and a SQLite dynamic loader, but the Rust CLI currently only supports `build`, `query`, and `serve`.

## CLI Contract

Add a new command to `poker-hands-storage-service`:

```text
cargo run -p poker-hands-storage-service -- verify \
  --mode standalone \
  --dir data/range-strata \
  --verify-checksum
```

```text
cargo run -p poker-hands-storage-service -- verify \
  --mode cross \
  --dir data/range-strata \
  --source data/sqlite/range.db \
  --sample-size 10000 \
  --verify-checksum
```

Options:

| Option | Mode | Default | Meaning |
| --- | --- | --- | --- |
| `--mode standalone|cross` | both | `standalone` | Verification mode |
| `--dir <path>` | both | required in Rust CLI | Range Strata Binary output directory |
| `--source <path>` | cross | required | Source SQLite DB |
| `--verify-checksum` | both | false | Verify CRC32C for pack payloads |
| `--sample-size <n>` | cross | `10000` | Source rows sampled across dimensions; `0` means full scan |
| `--max-failures <n>` | cross | `50` | Cap stored failure details |
| `--out <path>` | both | mode-specific report path | JSON report |
| `--md <path>` | both | mode-specific report path | Markdown report |

Default report paths:

- standalone JSON: `reports/range-strata-verify-standalone.json`
- standalone Markdown: `reports/range-strata-verify-standalone.md`
- cross JSON: `reports/range-strata-verify-cross.json`
- cross Markdown: `reports/range-strata-verify-cross.md`

Exit code is non-zero when any verification layer fails. In cross mode, `failedSourceRecords > 0` or `extraBinaryRecords > 0` is also a failure.

## Core Reader API

`range-store-core` should stay format-focused and independent from source SQLite or report concerns.

Add APIs:

- `IdxReader::record_at(index: u32) -> Option<IdxRecord>`
- `IdxReader::records() -> impl Iterator<Item = IdxRecord> + '_`
- `BinReader::file_len() -> usize`
- `pack_codec::decode_pack(pack: &[u8], hand_count: u16, action_count: u16) -> Result<DecodedPack, String>`

New decoded types:

```rust
pub struct DecodedPack {
    pub hand_ids: Vec<u8>,
    pub action_masks: Vec<u32>,
    pub cells: Vec<DecodedPackCell>,
}

pub struct DecodedPackCell {
    pub hand_id: u8,
    pub action_id: u32,
    pub exists: bool,
    pub frequency: f64,
    pub hand_ev: Option<f64>,
}
```

The existing hot-path `DimensionReader::query` and `decode_pack_for_hand` remain unchanged. Full decode exists for verification, where correctness and explainability matter more than per-query allocation cost.

## Standalone Verifier

Standalone mode proves that a binary output directory is internally consistent without using the source DB.

Checks:

1. `manifest.json`
   - file exists
   - JSON parses
   - `format == "PFSP"` and `version == 1`
   - successful dimensions have `idxFile` and `binFile`

2. file existence
   - `meta.db`
   - all `.idx` / `.bin` files for successful dimensions
   - failed manifest dimensions are skipped for per-dimension file requirements

3. catalog (`meta.db`)
   - required tables: `build_info`, `action_schemas`, `dimension_action_schemas`
   - `build_info` contains `built_at` and `source_checksum`
   - each `action_schemas` row has `1..=32` actions
   - `action_blob.len() == action_count * 9`
   - stored checksum equals `crc32c(action_blob)`
   - `schema_key` equals hex of `action_blob`
   - expected drill and concrete line metadata tables exist

4. index headers and records
   - PFXI magic, version 1, 16-byte header
   - file is large enough for declared record count
   - `concrete_line_id` is strictly increasing
   - `hand_count <= 169`
   - `action_schema_id` exists in catalog

5. pack headers
   - PFSP magic, version 1, little-endian, float32, sparse hand-major v1, no compression, 16-byte header

6. index-pack cross checks
   - idx record offset is outside the `.bin` header
   - `offset + byte_length <= bin file length`
   - `byte_length == hand_count * (5 + action_count * 8)`
   - optional CRC32C pack checksum
   - pack hand IDs are in `0..=168` and strictly increasing

Standalone should continue collecting failures when possible, rather than stopping after the first issue. Fatal manifest read/parse errors produce a report with limited totals.

## Cross Verifier

Cross mode first runs standalone verification, then compares source SQLite rows against binary packs.

Source discovery uses the same naming rules as builder:

- `range_data_{strategy}_{player_count}max_{depth_bb}BB`
- matching `concrete_lines_{strategy}_{player_count}max_{depth_bb}BB`

Only dimensions present in the source DB and not marked failed in the manifest are checked.

Sampling:

- `--sample-size 0` scans all source rows ordered by `concrete_line_id, hole_cards, action_name`.
- `--sample-size > 0` allocates a deterministic per-dimension quota proportional to row counts.
- Unlike upstream `ORDER BY random()`, Rust should use deterministic sampling so repeated verifier runs are reproducible. The selection can use a stable integer hash over `(dimension, concrete_line_id, hole_cards, action_name, action_size, amount_bb)` and take the lowest N rows per dimension quota.

Per source row:

1. Find the `.idx` record for `concrete_line_id`.
2. Load the action schema referenced by the record.
3. Read and optionally CRC-check the pack.
4. Full-decode the pack.
5. Convert source `hole_cards` to `hand_id`.
6. Find a matching schema action by normalized action name, `action_size`, and `amount_bb`.
7. Verify the decoded cell exists.
8. Compare `frequency` and `hand_ev` using float32 bit-exact policy.

Full mode additionally counts extra binary cells that exist in the decoded pack but have no matching source row.

Failure reasons mirror upstream where practical:

- `PACK_NOT_FOUND_IN_IDX`
- `CHECKSUM_MISMATCH`
- `UNKNOWN_HAND`
- `HAND_NOT_FOUND_IN_PACK`
- `ACTION_NOT_FOUND_IN_SCHEMA`
- `ACTION_CELL_NOT_SET`
- `FREQUENCY_FLOAT32_MISMATCH`
- `HAND_EV_NULL_MISMATCH`
- `HAND_EV_FLOAT32_MISMATCH`

## Float32 Precision Policy

Numeric source values are not compared as f64. The expected binary value is the exact IEEE-754 float32 representation of the source value.

Rules:

- `frequency`: source f64 -> expected f32 bits -> decoded f32 bits
- `hand_ev`: `NULL` must decode as `None`; non-null source follows the same f32 bit rule
- signed zero is significant
- values inside the legacy tolerance still fail if they land on different f32 bits

The report includes:

- checked value counts
- null counts for nullable fields
- bit-exact counts
- mismatch counts
- max / p95 / p99 quantization absolute error
- max implementation absolute error
- top quantization error samples

## Report Shape

The JSON report keeps the upstream top-level shape:

- `generatedAt`
- `mode`
- `directory`
- `sourceDbPath` for cross mode
- `verifyChecksums`
- `tolerances`
- `precisionPolicy`
- optional `precision`
- cross options such as `sampleSize` and `maxFailures`
- `totals`
- `dimensions`
- `failures`
- `repairSuggestions`

Markdown output mirrors the upstream report sections:

- Summary
- Precision Policy
- Float32 Quantization for cross mode
- Largest Float32 Quantization Errors when present
- Dimensions
- Failures
- Repair Suggestions

Failure details are capped in JSON and Markdown to avoid producing unusably large reports on corrupted data.

## Module Layout

Proposed Rust files:

```text
crates/range-store-core/src/
  idx_reader.rs          # record_at / records traversal
  bin_reader.rs          # file_len
  pack_codec.rs          # full pack decode
  types.rs               # DecodedPack types

crates/service/src/verifier/
  mod.rs                 # public verifier API and options
  report.rs              # report structs + JSON/Markdown write helpers
  precision.rs           # float32 bit-exact helpers and stats
  catalog.rs             # meta.db catalog loading/checks
  cli_args.rs            # verify CLI argument parsing
  standalone.rs          # standalone orchestration
  source_cross.rs        # source DB cross check
```

`main.rs` remains the CLI router. Verifier argument parsing lives in `verifier::cli_args` so it can be covered by integration tests while keeping dependencies minimal.

## Testing Strategy

New verifier/core coverage should live in crate-level `tests/` directories, with shared service fixtures under `crates/service/tests/common`.

Core integration tests:

- record traversal returns records in file order
- out-of-range `record_at` returns `None`
- `file_len` reflects mmap file length
- full pack decode includes both set and unset cells
- full pack decode rejects invalid byte lengths

Service integration tests:

- clean fixture passes standalone
- missing/corrupt manifest fails with report
- missing `.idx` / corrupt `.idx` magic fails
- corrupt `.bin` header fails
- bad action schema checksum fails
- idx out-of-order fails
- pack size mismatch is counted as index-pack failure
- clean fixture passes cross
- source value changed to a different float32 inside legacy tolerance fails cross
- nullable `hand_ev` mismatch fails cross

Validation commands:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

Smoke commands after implementation:

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify --mode standalone --dir data/range-strata --verify-checksum
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify --mode cross --dir data/range-strata --source data/sqlite/range.db --sample-size 10000 --verify-checksum
```

## Risks And Decisions

- Deterministic sampling intentionally differs from upstream `ORDER BY random()` to make Rust verification reproducible. The report records the requested `sampleSize` and `maxFailures` in cross mode.
- Cross full scan may be expensive on the full 9max data. The default remains sampled verification.
- No hard performance threshold belongs in verifier. Benchmark commands will own performance observability.
- The verifier must not require GNU target or static SQLite linkage. It continues using the existing `libloading` SQLite wrapper and the default `x86_64-pc-windows-msvc` target.
