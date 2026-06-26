# Rust Verifier 7a-7b Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Rust `verify --mode standalone` and `verify --mode cross` commands that match the upstream Range Strata Binary verifier semantics.

**Architecture:** `range-store-core` provides traversal and full-decode primitives. `poker-hands-storage-service` implements verifier modules for reports, precision, catalog checks, standalone checks, source cross-checks, and CLI routing.

**Tech Stack:** Rust 2021, existing mmap readers, existing dynamic SQLite loader, serde/serde_json, x86_64-pc-windows-msvc target.

---

### Task 1: Core Traversal And Full Pack Decode

**Files:**
- Modify: `crates/range-store-core/src/types.rs`
- Modify: `crates/range-store-core/src/lib.rs`
- Modify: `crates/range-store-core/src/idx_reader.rs`
- Modify: `crates/range-store-core/src/bin_reader.rs`
- Modify: `crates/range-store-core/src/pack_codec.rs`
- Create: `crates/range-store-core/tests/traversal_and_decode.rs`

- [ ] **Step 1: Write failing core traversal tests**

Add integration tests in `crates/range-store-core/tests/traversal_and_decode.rs`:

```rust
#[test]
fn test_record_at_and_records_iterate_in_file_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let records = vec![
        IdxRecord { concrete_line_id: 10, action_schema_id: 1, hand_count: 2, offset: 16, byte_length: 42, checksum: 100 },
        IdxRecord { concrete_line_id: 20, action_schema_id: 2, hand_count: 3, offset: 58, byte_length: 87, checksum: 200 },
    ];
    let path = make_test_idx(dir.path(), "test.idx", &records);
    let reader = IdxReader::open(&path).unwrap();

    assert_eq!(reader.record_at(0).unwrap().concrete_line_id, 10);
    assert_eq!(reader.record_at(1).unwrap().concrete_line_id, 20);
    assert!(reader.record_at(2).is_none());
    assert_eq!(
        reader.records().map(|record| record.concrete_line_id).collect::<Vec<_>>(),
        vec![10, 20]
    );
}
```

Cover `BinReader::file_len` in the same integration test file:

```rust
assert_eq!(reader.file_len(), PFSP_HEADER_SIZE + 100);
```

Add full decode tests in the same integration test file:

```rust
#[test]
fn test_decode_full_pack_includes_unset_cells() {
    let mut pack = make_test_pack(&[0, 2], 2, &[0.5, 1.0, 0.25, 2.0, 0.75, 3.0, 0.0, 4.0]);
    pack[2..6].copy_from_slice(&1u32.to_le_bytes());
    pack[6..10].copy_from_slice(&2u32.to_le_bytes());

    let decoded = decode_pack(&pack, 2, 2).unwrap();

    assert_eq!(decoded.hand_ids, vec![0, 2]);
    assert_eq!(decoded.action_masks, vec![1, 2]);
    assert_eq!(decoded.cells.len(), 4);
    assert!(decoded.cells[0].exists);
    assert!(!decoded.cells[1].exists);
    assert!(!decoded.cells[2].exists);
    assert!(decoded.cells[3].exists);
}
```

- [ ] **Step 2: Run tests and verify red**

Run:

```text
cargo test -p range-store-core --target x86_64-pc-windows-msvc
```

Expected: compile fails with missing `record_at`, `records`, `file_len`, `decode_pack`, and decoded type symbols.

- [ ] **Step 3: Implement minimal core APIs**

Add public decoded types:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedPackCell {
    pub hand_id: u8,
    pub action_id: u32,
    pub exists: bool,
    pub frequency: f64,
    pub hand_ev: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedPack {
    pub hand_ids: Vec<u8>,
    pub action_masks: Vec<u32>,
    pub cells: Vec<DecodedPackCell>,
}
```

Implement:

```rust
pub fn record_at(&self, index: u32) -> Option<IdxRecord>
pub fn records(&self) -> impl Iterator<Item = IdxRecord> + '_
pub fn file_len(&self) -> usize
pub fn decode_pack(pack: &[u8], hand_count: u16, action_count: u16) -> Result<DecodedPack, String>
```

- [ ] **Step 4: Verify green**

Run:

```text
cargo test -p range-store-core --target x86_64-pc-windows-msvc
```

Expected: all range-store-core tests pass.

### Task 2: Report And Precision Modules

**Files:**
- Create: `crates/service/src/verifier/mod.rs`
- Create: `crates/service/src/verifier/report.rs`
- Create: `crates/service/src/verifier/precision.rs`
- Create: `crates/service/tests/verifier_precision_report.rs`
- Modify: `crates/service/src/lib.rs`

- [ ] **Step 1: Write failing precision/report tests**

Add integration tests covering:

```rust
assert!(check_float32_round_trip(0.1, f32::from_bits(0x3dcccccd) as f64).ok);
assert!(!check_float32_round_trip(0.5000000596046448, 0.5).ok);
assert!(!check_float32_round_trip(-0.0, 0.0).ok);
assert!(check_nullable_float32_round_trip(None, None).ok);
assert!(!check_nullable_float32_round_trip(None, Some(0.0)).ok);
```

Add a report test that creates an empty standalone report and asserts:

```rust
assert_eq!(report.mode, VerifyMode::Standalone);
assert!(report.totals.manifest_ok);
assert!(render_markdown(&report).contains("Range Strata Binary Integrity Report"));
```

- [ ] **Step 2: Run tests and verify red**

Run:

```text
cargo test -p poker-hands-storage-service verifier --target x86_64-pc-windows-msvc
```

Expected: compile fails because verifier modules do not exist.

- [ ] **Step 3: Implement precision and report**

Implement:

- `Float32RoundTripCheck`
- `NullableFloat32RoundTripCheck`
- `Float32PrecisionStatsAccumulator`
- `VerifyFailure`
- `DimensionVerifyDetail`
- `RangeStrataVerifyReport`
- JSON writer using `serde_json::to_string_pretty`
- Markdown renderer using simple local table formatting

Use serde camelCase to match upstream JSON where applicable.

- [ ] **Step 4: Verify green**

Run:

```text
cargo test -p poker-hands-storage-service verifier --target x86_64-pc-windows-msvc
```

Expected: verifier precision/report tests pass.

### Task 3: Standalone Verifier

**Files:**
- Create: `crates/service/src/verifier/catalog.rs`
- Create: `crates/service/src/verifier/standalone.rs`
- Create: `crates/service/tests/common/mod.rs`
- Create: `crates/service/tests/verifier_standalone.rs`
- Modify: `crates/service/src/verifier/mod.rs`

- [ ] **Step 1: Write failing standalone tests**

Use a `tests/common` helper based on the existing builder fixture pattern:

```rust
fn build_verify_fixture(root: &Path) -> (PathBuf, PathBuf) {
    // Create source.db with one range_data table, concrete_lines table, and drill table.
    // Run build_store into root/output.
    // Return (source_path, output_path).
}
```

Tests:

- clean output has zero failures
- missing manifest sets `manifestOk = false`
- corrupt `.idx` magic produces `INVALID_MAGIC`
- corrupt `.bin` header produces pack-header failure
- bad action schema checksum produces catalog `CHECKSUM_MISMATCH`
- swapped concrete line IDs produces `OUT_OF_ORDER`
- mutated record byte length produces `PACK_SIZE_MISMATCH`

- [ ] **Step 2: Run tests and verify red**

Run:

```text
cargo test -p poker-hands-storage-service standalone --target x86_64-pc-windows-msvc
```

Expected: compile fails or tests fail because standalone verifier is missing.

- [ ] **Step 3: Implement standalone checks**

Implement:

```rust
pub struct StandaloneVerifyOptions {
    pub dir: PathBuf,
    pub verify_checksums: bool,
    pub out_path: Option<PathBuf>,
    pub md_path: Option<PathBuf>,
}

pub fn run_standalone_verify(options: &StandaloneVerifyOptions) -> RangeStrataVerifyReport
```

Layers:

- file-existence
- manifest
- catalog
- index-header
- pack-header
- index-pack-cross

- [ ] **Step 4: Verify green**

Run:

```text
cargo test -p poker-hands-storage-service standalone --target x86_64-pc-windows-msvc
```

Expected: standalone tests pass.

### Task 4: Cross Verifier

**Files:**
- Create: `crates/service/src/verifier/source_cross.rs`
- Create: `crates/service/tests/verifier_source_cross.rs`
- Modify: `crates/service/src/verifier/mod.rs`

- [ ] **Step 1: Write failing cross tests**

Add integration tests:

```rust
#[test]
fn cross_verify_sample_passes_clean_fixture()
#[test]
fn cross_verify_full_detects_float32_mismatch_inside_legacy_tolerance()
#[test]
fn cross_verify_full_counts_extra_binary_cells()
```

The float32 mismatch test mutates source after build:

```sql
UPDATE range_data_default_6max_100BB
SET frequency = 0.5000000596046448
WHERE concrete_line_id = 2 AND hole_cards = 'AKs';
```

- [ ] **Step 2: Run tests and verify red**

Run:

```text
cargo test -p poker-hands-storage-service source_cross --target x86_64-pc-windows-msvc
```

Expected: compile fails or tests fail because cross verifier is missing.

- [ ] **Step 3: Implement source cross-check**

Implement:

```rust
pub struct CrossVerifyOptions {
    pub dir: PathBuf,
    pub source_db: PathBuf,
    pub sample_size: usize,
    pub max_failures: usize,
    pub verify_checksums: bool,
    pub out_path: Option<PathBuf>,
    pub md_path: Option<PathBuf>,
}

pub fn run_cross_verify(options: &CrossVerifyOptions) -> RangeStrataVerifyReport
```

Use deterministic sampling by stable hash, not `ORDER BY random()`.

Compare:

- idx record exists
- action schema exists
- pack CRC when requested
- source hand maps to pack hand
- source action maps to schema action
- decoded cell exists
- frequency and hand EV are float32 bit-exact
- extra binary cells in full mode

- [ ] **Step 4: Verify green**

Run:

```text
cargo test -p poker-hands-storage-service source_cross --target x86_64-pc-windows-msvc
```

Expected: cross tests pass.

### Task 5: CLI, Docs, And Validation

**Files:**
- Create: `crates/service/src/verifier/cli_args.rs`
- Create: `crates/service/tests/verify_cli_args.rs`
- Modify: `crates/service/src/main.rs`
- Modify: `README.md`
- Modify: `docs/progress.md`

- [ ] **Step 1: Add failing CLI parse tests or command helper tests**

Extract verify arg parsing into a testable library module:

```rust
pub fn parse_verify_args(args: Vec<String>) -> Result<VerifyCommand, AppError>
```

Tests:

- standalone defaults report paths
- cross requires `--source`
- invalid `--mode` is rejected
- `--sample-size 0` is accepted

- [ ] **Step 2: Run tests and verify red**

Run:

```text
cargo test -p poker-hands-storage-service verify_args --target x86_64-pc-windows-msvc
```

Expected: tests fail before parser exists.

- [ ] **Step 3: Wire CLI**

Add `verify` to the main command router. Summary output should include:

- dimensions
- manifest/catalog OK
- index files OK
- pack files OK
- index-pack failures
- cross source records checked/failed for cross mode
- total failures

Set process exit code to 1 when report failure rules indicate failure.

- [ ] **Step 4: Update docs**

Update `README.md` with Rust verifier examples and replace the statement that authoritative standalone verifier remains upstream.

Update `docs/progress.md` to note that Rust verifier design/implementation exists after code is complete.

- [ ] **Step 5: Full validation**

Run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

Then run smoke checks:

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify --mode standalone --dir data/range-strata --verify-checksum
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify --mode cross --dir data/range-strata --source data/sqlite/range.db --sample-size 10000 --verify-checksum
```

Expected: validation commands pass and both verifier smoke commands write JSON/Markdown reports. Cross smoke may take longer than unit tests because it samples the real source DB.
