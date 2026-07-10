# Compact LineMatrix V2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a compact V2 LineMatrix archive exporter and O(1) reader without changing V1 behavior.

**Architecture:** V2 gets its own Protobuf package and archive module. It shares the physical record index layout with V1 but uses archive version 2, an explicit payload schema in the manifest, and a separately named CLI command.

**Tech Stack:** Rust, Prost, SQLite, CRC32C, tempfile integration tests.

## Global Constraints

- Only `default:6:100` and `HAND_ENCODING_169` are supported by this exporter.
- V1 files and readers remain backward compatible.
- `hand_ev IS NULL` is omitted only when its source frequency is exactly zero.
- Bitmap bit order is LSB-first and all padding bits are zero.

---

### Task 1: Define and generate the V2 payload

**Files:**
- Create: `storage-tools/proto/zenithstrat/gto/v2/compact_matrix.proto`
- Modify: `storage-tools/build.rs`
- Modify: `storage-tools/src/lib.rs`

- [ ] Add V2 messages with original and compact bitmap domains documented in schema comments.
- [ ] Compile both V1 and V2 proto files through Prost.
- [ ] Expose a V2 module without altering V1 generated types.

### Task 2: Build and validate compact matrices

**Files:**
- Create: `storage-tools/src/compact_line_matrix/convert.rs`
- Create: `storage-tools/src/compact_line_matrix/proto.rs`
- Create: `storage-tools/src/compact_line_matrix/mod.rs`
- Test: `storage-tools/tests/compact_line_matrix.test.rs`

- [ ] Write failing tests for NULL-EV filtering, action-local compact arrays, and canonical bitmap validation.
- [ ] Convert source rows into the V2 layout and enforce all source and payload invariants.
- [ ] Run the focused test target.

### Task 3: Write and read V2 archives

**Files:**
- Create: `storage-tools/src/compact_line_matrix_archive/mod.rs`
- Create: `storage-tools/src/compact_line_matrix_archive/cli.rs`
- Modify: `storage-tools/src/main.rs`
- Test: `storage-tools/tests/compact_line_matrix.test.rs`

- [ ] Write failing archive roundtrip and O(1) lookup tests.
- [ ] Implement version-2 headers, manifest, CRC checking, metadata database, and compact-index cache construction.
- [ ] Add `export-compact-line-matrix-archive` CLI command.

### Task 4: Document and verify

**Files:**
- Modify: `docs/protobuf-line-matrix-export.md`
- Modify: `reports/line-matrix-storage-comparison-6max-100BB.md`

- [ ] Document V2 schema and reader mapping rules.
- [ ] Generate a V2 full archive and update the storage comparison.
- [ ] Run formatting and the complete `storage-tools` test suite.

