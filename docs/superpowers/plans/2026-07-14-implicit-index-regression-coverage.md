# Implicit Index Regression Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lock in the current version-1 implicit 18-byte index format, removed association table, and default lazy action-schema cache with focused regression tests.

**Architecture:** Production code already implements the implicit-ID index and on-demand schema cache. This plan adds characterization tests at the public facade and builder boundaries, then validates the existing active documentation and workspace behavior without refactoring working code.

**Tech Stack:** Rust 2021, Cargo, SQLite via the existing dynamic wrapper, tempfile, mmap-backed index reader.

## Global Constraints

- Keep `manifest.json`, `PFXI`, and `PFSP` at version `1`.
- Preserve the 18-byte `.idx` record and the required concrete-line ID sequence `1..=N`.
- Keep `ActionSchemaCache` lazy by default; do not wire `unique_action_schema_ids()` into `prewarm()`.
- All Cargo test commands use `--target x86_64-pc-windows-msvc`.
- Do not modify unrelated uncommitted files and do not create a commit unless the user explicitly requests one.

---

### Task 1: Characterize Lazy Schema Cache Behavior

**Files:**
- Modify: `range-store-core/tests/query_contract.test.rs`
- Test: `range-store-core/tests/query_contract.test.rs`

**Interfaces:**
- Consumes: `RangeStoreFacade::prewarm(&DimensionRef) -> Result<usize, RangeStoreError>`.
- Consumes: `RangeStoreFacade::schema_count() -> usize`.
- Consumes: `RangeStoreFacade::query_hand_strategy(&DimensionRef, u32, &str)`.
- Produces: a facade-level regression test proving that prewarming opens files without loading schemas and that a query loads one schema exactly once.

- [ ] **Step 1: Run the existing query contract target**

Run:
```powershell
cargo test -p range-store-core --test query_contract_test --target x86_64-pc-windows-msvc
```

Expected: PASS. The cache behavior is already implemented; this is a characterization addition, not a new production feature.

- [ ] **Step 2: Add the lazy-cache regression test**

Add this test after `single_query_returns_actions_only`:

```rust
#[test]
fn schema_cache_stays_empty_during_prewarm_and_reuses_a_loaded_schema() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = build_query_test_store(temp.path());
    let store = RangeStoreFacade::open(&data_dir, 2, true).unwrap();
    let dimension = DimensionRef::new("default", 6, 100);

    assert_eq!(store.schema_count(), 0);
    assert_eq!(store.prewarm(&dimension).unwrap(), 1);
    assert_eq!(store.schema_count(), 0);

    store.query_hand_strategy(&dimension, 1, "AA").unwrap();
    assert_eq!(store.schema_count(), 1);

    store.query_hand_strategy(&dimension, 1, "KK").unwrap();
    assert_eq!(store.schema_count(), 1);
}
```

- [ ] **Step 3: Run the focused cache test**

Run:
```powershell
cargo test -p range-store-core --test query_contract_test schema_cache_stays_empty_during_prewarm_and_reuses_a_loaded_schema --target x86_64-pc-windows-msvc
```

Expected: PASS with one test run.

- [ ] **Step 4: Run the full core query contract target**

Run:
```powershell
cargo test -p range-store-core --test query_contract_test --target x86_64-pc-windows-msvc
```

Expected: PASS with all query contract tests.

### Task 2: Lock Builder Output Against Association-Table Regression

**Files:**
- Modify: `storage-tools/tests/range_store_builder_build_orchestrator.test.rs`
- Test: `storage-tools/tests/range_store_builder_build_orchestrator.test.rs`

**Interfaces:**
- Consumes: `build_store(&BuildOptions) -> Result<BuildSummary, ToolError>`.
- Consumes: `Connection::open(&Path, true) -> Result<Connection, SqliteError>`.
- Consumes: `Connection::prepare(&str)` and `Statement::step_row()`.
- Produces: a built-store assertion that `meta.db` has no `dimension_action_schemas` table.

- [ ] **Step 1: Add the no-association-table assertion**

In `builds_queryable_store_from_sqlite`, immediately after asserting the build summary, add:

```rust
    let meta = Connection::open(&output_path.join("meta.db"), true).unwrap();
    let mut removed_table = meta
        .prepare(
            "SELECT 1
             FROM sqlite_master
             WHERE type = 'table' AND name = 'dimension_action_schemas'",
        )
        .unwrap();
    removed_table.start(&[]).unwrap();
    assert!(!removed_table.step_row().unwrap());
```

- [ ] **Step 2: Run the focused builder test**

Run:
```powershell
cargo test -p poker-hands-storage-tools --test range_store_builder_build_orchestrator_test builds_queryable_store_from_sqlite --target x86_64-pc-windows-msvc
```

Expected: PASS with one test run.

- [ ] **Step 3: Run the complete builder orchestrator target**

Run:
```powershell
cargo test -p poker-hands-storage-tools --test range_store_builder_build_orchestrator_test --target x86_64-pc-windows-msvc
```

Expected: PASS with all builder orchestrator tests.

### Task 3: Validate the Landed Format and Documentation

**Files:**
- Verify: `range-store-core/src/types.rs`
- Verify: `range-store-core/src/idx_reader.rs`
- Verify: `storage-tools/src/range_store_builder.rs`
- Verify: `storage-tools/src/verification/standalone.rs`
- Verify: `docs/range-db-binary-storage-design.md`
- Verify: `docs/verification_and_benchmark.md`
- Verify: `docs/sdk-and-query-chain-explanation.md`

**Interfaces:**
- Consumes: the version-1 artifact headers, `IDX_RECORD_SIZE = 18`, implicit line-ID lookup, and the lazy cache behavior locked by Tasks 1 and 2.
- Produces: command output demonstrating that the current implementation and active documentation contain no association-table dependency.

- [ ] **Step 1: Search active code and documentation for the removed table**

Run:
```powershell
rg -n -i "dimension_action_schemas|dimension_action_schema_ids|validate_dimension_schema_refs" README.md docs range-store-core storage-tools service range-store-native --glob "!target/**" --glob "!docs/release-*" --glob "!docs/superpowers/**"
```

Expected: exit code `1` and no matches. Historical release reports and this plan/spec are excluded intentionally.

- [ ] **Step 2: Confirm version-1 and 18-byte layout declarations**

Run:
```powershell
rg -n "IDX_RECORD_SIZE: usize = 18|version == 1|version != 1|record_count \* 18" range-store-core/src storage-tools/src docs/range-db-binary-storage-design.md docs/verification_and_benchmark.md
```

Expected: matches proving the reader, builder, verifier, and active documentation agree on version `1` and 18-byte records.

- [ ] **Step 3: Check formatting and the affected test targets**

Run:
```powershell
cargo fmt --all -- --check
cargo test -p range-store-core --test query_contract_test --target x86_64-pc-windows-msvc
cargo test -p poker-hands-storage-tools --test range_store_builder_build_orchestrator_test --target x86_64-pc-windows-msvc
```

Expected: all commands exit with code `0`.

- [ ] **Step 4: Run workspace validation when the focused targets pass**

Run:
```powershell
cargo test --workspace --target x86_64-pc-windows-msvc
```

Expected: workspace tests pass; if an unrelated existing failure occurs, record its target and output without changing unrelated code.

