# Proto LineMatrix Archive Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the compact Proto LineMatrix archive exporter and O(1) reader.

**Architecture:** The Proto scheme uses its own Protobuf package and archive module with archive version 2, an explicit payload schema in the manifest, and dedicated CLI commands.

**Tech Stack:** Rust, Prost, SQLite, CRC32C, tempfile integration tests.

## Global Constraints

- Every discovered dimension is exported separately and must use `HAND_ENCODING_169`.
- The old V1 LineMatrix files and readers are no longer part of the implementation.
- `hand_ev IS NULL` is filtered by the Proto source query and never enters the payload.
- Bitmap bit order is LSB-first and all padding bits are zero.

---

### Task 1: Define and generate the Proto payload

**Files:**
- Create: `storage-tools/proto/zenithstrat/gto/v2/compact_matrix.proto`
- Modify: `storage-tools/build.rs`
- Modify: `storage-tools/src/lib.rs`

- [ ] Add Proto messages with original and compact bitmap domains documented in schema comments.
- [ ] Compile the self-contained Proto payload through Prost.
- [ ] Expose the Proto module without a V1 schema dependency.

### Task 2: Build and validate compact matrices

**Files:**
- Create: `storage-tools/src/proto_range_storage/line_matrix_codec.rs`
- Create: `storage-tools/src/proto_range_storage/proto.rs`
- Test: `storage-tools/tests/compact_line_matrix_archive.test.rs`

- [ ] Write failing tests for NULL-EV filtering, action-local compact arrays, and canonical bitmap validation.
- [ ] Convert source rows into the Proto layout and enforce all source and payload invariants.
- [ ] Run the focused test target.

### Task 3: Write and read Proto archives

**Files:**
- Create: `storage-tools/src/proto_range_storage/line_matrix_store.rs`
- Create: `storage-tools/src/proto_range_storage/cli.rs`
- Modify: `storage-tools/src/main.rs`
- Test: `storage-tools/tests/compact_line_matrix_archive.test.rs`

- [ ] Write failing archive roundtrip and O(1) lookup tests.
- [ ] Implement version-2 headers, manifest, CRC checking, metadata database, and compact-index cache construction.
- [ ] Add `export-compact-line-matrix-archive` CLI command.

### Task 4: Document and verify

**Files:**
- Modify: `docs/protobuf-line-matrix-export.md`
- Modify: `reports/line-matrix-storage-comparison-6max-100BB.md`

- [ ] Document the Proto schema and reader mapping rules.
- [ ] Generate a full Proto archive and update the storage comparison.
- [ ] Run formatting and the complete `storage-tools` test suite.

