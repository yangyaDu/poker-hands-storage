# Compact LineMatrix V3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a V3 CompactLineMatrix schema with reader optimizations (mmap + LRU + identity binary search + rayon parallel verify) without changing V1/V2 behavior.

**Architecture:** V3 gets its own Protobuf package `zenithstrat.gto.v3`, its own convert and archive modules. V2 archives remain readable by the V2 reader. The V3 form (full vs partial) is decided by Phase 0 `bloatFactor`.

**Tech Stack:** Rust, Prost, SQLite, CRC32C, memmap2, lru, rayon, tempfile integration tests.

## Global Constraints

- V1/V2 files and readers remain backward compatible.
- V3 archive magic is `LM3N`/`LM3X`, format version 3; V3 reader rejects V2 archives and vice versa.
- Bitmap bit order is LSB-first and all padding bits are zero.
- `hand_ev IS NULL` rows are filtered by the source query and never enter the V3 payload (same as V2).
- Phase 0 `bloatFactor` decides Form A (< 1.5) or Form B (> 3.0); 1.5-3.0 requires user review.

---

### Task 1: Phase 0 data exploration

**Files:**
- Create: `storage-tools/src/compact_line_matrix_v3_archive/mod.rs`
- Create: `storage-tools/src/compact_line_matrix_v3_archive/cli.rs`
- Create: `storage-tools/src/compact_line_matrix_v3_archive/analyze.rs`
- Modify: `storage-tools/src/lib.rs`
- Modify: `storage-tools/src/main.rs`
- Modify: `storage-tools/Cargo.toml`

- [ ] Add `compact_line_matrix_v3_archive` module declaration to `lib.rs`.
- [ ] Implement `analyze.rs`: reuse V2 `CompactLineMatrixArchive` reader; per matrix collect `valid_hand_count`, `action_count`, per-action `popcount(action_hand_bitmap)`, projected V3 entries = `valid_hand_count * action_count`, V2 actual entries = `sum popcount(action_hand_bitmap)`.
- [ ] Aggregate to JSON: `matrixCount`, `v2ActionValueCount`, `validHandCount` / `actionCount` / `actionCoverageRatio` percentiles, `projectedV3.bloatFactor` / `estimatedV3DataBytes` / `v2DataBytes`.
- [ ] Implement `cli.rs`: parse `--dir <v2-archive-dir>`.
- [ ] Add `analyze-compact-line-matrix-archive` command to `main.rs` `run()` match block and `print_help()`.
- [ ] Run against production V2 archive to obtain `bloatFactor` and record the decision (Form A / Form B).

### Task 2: V3 Proto schema

**Files:**
- Create: `storage-tools/proto/zenithstrat/gto/v3/compact_matrix.proto`
- Modify: `storage-tools/build.rs`
- Create: `storage-tools/src/compact_line_matrix_v3/proto.rs`
- Create: `storage-tools/src/compact_line_matrix_v3/mod.rs`
- Modify: `storage-tools/src/lib.rs`

- [ ] Define `CompactActionIdentity { action_type, amount_centi_bb, action_size_x10000 }`.
- [ ] Define `CompactActionData { frequency_x10000, ev_x10000, action_hand_bitmap }` with length semantics documented per form.
- [ ] Define `CompactLineMatrixV3 { schema_version=3, hand_encoding, action_identities[], action_data[], valid_hand_bitmap }`.
- [ ] Add `v3_proto` to `compile_protos` in `build.rs`.
- [ ] `proto.rs` includes generated `zenithstrat.gto.v3` code; `mod.rs` declares `convert` + `proto` submodules.
- [ ] Add `pub mod compact_line_matrix_v3` to `lib.rs`.

### Task 3: V3 convert.rs

**Files:**
- Create: `storage-tools/src/compact_line_matrix_v3/convert.rs`
- Test: `storage-tools/tests/compact_line_matrix_v3.test.rs`

- [ ] Reuse V2 helpers (`build_compact_index_map`, `count_bits`, `bit_is_set`, `normalize_action_type`, `quantize_*`) via re-export or copy.
- [ ] Normalize rows (same as V2): filter NULL EV, quantize, deduplicate `(hand_id, action)`.
- [ ] Build `valid_hand_bitmap` + `hand_id_to_global_index` (same as V2).
- [ ] Group by `BTreeMap<ActionKey, Vec<NormalizedRow>>` (auto-sorted).
- [ ] Form A: emit `CompactActionIdentity` + `CompactActionData` with `frequency`/`ev` length = `valid_hand_count`, fill by `global_index`, set bit in `action_hand_bitmap` as validity flag.
- [ ] Form B: emit `CompactActionIdentity` + `CompactActionData` with `frequency`/`ev` length = `popcount(action_hand_bitmap)` (same as V2), retain compact index semantics.
- [ ] Assemble `CompactLineMatrixV3` and validate.
- [ ] `validate_compact_line_matrix_v3`: `schema_version==3`, `hand_encoding==169`, identities non-empty/sorted/unique, `action_data.len()==action_identities.len()`, array length per form, `valid_hand_bitmap` 22 bytes + zero padding.

### Task 4: V3 archive module (mmap + LRU)

**Files:**
- Create: `storage-tools/src/compact_line_matrix_v3_archive/format.rs`
- Modify: `storage-tools/src/compact_line_matrix_v3_archive/mod.rs`
- Modify: `storage-tools/Cargo.toml`

- [ ] `format.rs`: `DATA_MAGIC = b"LM3N"`, `INDEX_MAGIC = b"LM3X"`, `FORMAT_VERSION = 3`, `HEADER_SIZE = 16`, `INDEX_RECORD_SIZE = 16`, file name constants, `IndexRecord`, `write_header`/`read_header`/`write_index_record`/`read_index_record`.
- [ ] Add `memmap2 = "0.9"`, `lru = "0.12"`, `rayon = "1"` to `Cargo.toml`.
- [ ] `CompactLineMatrixV3Archive` struct: `data_mmap`, `index_mmap`, `_data_file`, `_index_file`, `matrix_count`, `cache: Mutex<LruCache<u64, Arc<DecodedCompactLineMatrixV3>>>`.
- [ ] `open(dir)`: read manifest, validate version=3 / payload schema `zenithstrat.gto.v3.CompactLineMatrixV3`; `File::open` + `unsafe Mmap::map` data and index; validate header from mmap slice; validate index file size; create `LruCache` (default capacity 1024).
- [ ] `read_matrix(id)`: LRU hit -> `Arc::clone`; miss -> read IndexRecord from `index_mmap` slice, read payload from `data_mmap` slice, CRC32C verify, protobuf decode, `DecodedCompactLineMatrixV3::new`, wrap `Arc`, insert LRU.
- [ ] `DecodedCompactLineMatrixV3`: `matrix`, `hand_id_to_global_index` (map#1). Form A: no map#2. Form B: `action_local_indices: Vec<i16>` + `action_offsets: Vec<u16>` (flattened).
- [ ] `action_value(action_index, hand_id)`: Form A -> `map#1[hand_id] -> global_index`, check `action_hand_bitmap` validity bit, return `frequency[global_index]`/`ev[global_index]`. Form B -> `map#1 -> global_index`, `offset = action_offsets[action_index]`, `local = action_local_indices[offset + global_index]`, if `local < 0` return None, else `frequency[local]`/`ev[local]`.
- [ ] `action_value_by_identity(action_type, action_size_x10000, amount_centi_bb, hand_id)`: binary-search `action_identities` by `(action_type, action_size_x10000, amount_centi_bb)`, then call `action_value`.

### Task 5: V3 export commands

**Files:**
- Modify: `storage-tools/src/compact_line_matrix_v3_archive/mod.rs`
- Modify: `storage-tools/src/compact_line_matrix_v3_archive/cli.rs`
- Modify: `storage-tools/src/main.rs`

- [ ] `export_compact_line_matrix_v3_archive`: single-dimension export; reuse V2 export flow (dense line IDs, tmp files, atomic rename, metadata DB, manifest with `matrix_schema_version=3`, `payload_schema=zenithstrat.gto.v3.CompactLineMatrixV3`).
- [ ] `export_all_compact_line_matrix_v3_archives`: discover dimensions, export each, verify, emit `storage-comparison.json`.
- [ ] CLI parsing for `export-compact-line-matrix-v3-archive` (`--source-db`, `--out-dir`, `--dimension`, `--overwrite`), `export-all-compact-line-matrix-v3-archives` (`--source-db`, `--out-dir`, `--overwrite`).
- [ ] Add both commands to `main.rs` `run()` match block and `print_help()`.

### Task 6: verify_all parallelization

**Files:**
- Modify: `storage-tools/src/compact_line_matrix_v3_archive/mod.rs`

- [ ] Private `decode_matrix_uncached(id)`: same as `read_matrix` but bypasses LRU (no lock, no insert).
- [ ] `verify_all`: `(1..=matrix_count).into_par_iter().map(decode_matrix_uncached).collect()`; aggregate `action_count` + `action_value_count`.
- [ ] `verify_all_sequential`: serial fallback for tests/debug.
- [ ] `verify-compact-line-matrix-v3-archive` CLI command (`--dir`).

### Task 7: Tests

**Files:**
- Create: `storage-tools/tests/common/mod.rs`
- Create: `storage-tools/tests/compact_line_matrix_v3.test.rs`
- Modify: `storage-tools/tests/line_matrix_export.test.rs`
- Modify: `storage-tools/Cargo.toml`

- [ ] Extract `create_source_fixture` (and helpers) from `line_matrix_export.test.rs` to `tests/common/mod.rs`; update V2 test to import from `common`.
- [ ] Add `compact_line_matrix_v3_test` test target to `Cargo.toml`.
- [ ] V3 proto roundtrip (encode + decode equal).
- [ ] V3 convert: identities sorted and unique (reuse `create_source_fixture`).
- [ ] V3 convert: array lengths correct (Form A = `valid_hand_count`; Form B = `popcount`).
- [ ] V3 archive export + readback (`open` + `read_matrix` + `matrix_count` assertion).
- [ ] V3 `action_value`: None (invalid hand) / None (action absent, Form A via bitmap) / Some.
- [ ] V3 `action_value_by_identity`: binary search hit + miss.
- [ ] V3 LRU cache: two consecutive reads of same id, second is a cache hit.
- [ ] V3 mmap vs `File::open`: both reads return identical results.
- [ ] V2 backward compat: V2 commands still export V2 archives; V2 reader reads V2; V3 reader rejects V2 (magic mismatch).
- [ ] V3 `verify_all` parallel: results match `verify_all_sequential`.
- [ ] Phase 0 analyze: run on V2 archive, assert JSON output structure.

### Task 8: Documentation and final verification

**Files:**
- Modify: `docs/protobuf-line-matrix-export.md`

- [ ] Document V3 schema, identity array, and reader mapping rules.
- [ ] `cargo fmt --all`.
- [ ] `cargo clippy --workspace -- -D warnings`.
- [ ] `cargo build -p poker-hands-storage-tools`.
- [ ] `cargo test -p poker-hands-storage-tools`.
- [ ] `cargo test -p poker-hands-storage-tools --test compact_line_matrix_v3_test`.
- [ ] `cargo test -p poker-hands-storage-tools --test line_matrix_export_test` (V2 backward compat).
- [ ] Generate V3 archive from production `range.db`, compare `storage-comparison.json` with V2.
