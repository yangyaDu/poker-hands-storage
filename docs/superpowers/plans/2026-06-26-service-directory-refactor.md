# Service Directory Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reorganize `poker-hands-storage-service` into business-oriented modules, move service tests into `tests/`, and use `<source-file>.test.rs` names for integration tests.

**Architecture:** Keep CLI, HTTP routes, binary store format, verifier behavior, and report schema unchanged. Move code by domain first, use short-lived module facades only while imports are being migrated, then split large files where doing so preserves behavior with existing tests. Tests move before risky code splits so each phase has a regression net.

**Tech Stack:** Rust 2021, Cargo explicit integration test targets, Axum, dynamic SQLite via `libloading`, Windows MSVC target `x86_64-pc-windows-msvc`.

---

## File Structure

Target production directories:

- `crates/service/src/config`: environment-driven service configuration.
- `crates/service/src/constants`: shared defaults and report path constants.
- `crates/service/src/domain`: poker/range business concepts such as action schemas, dimensions, and hole cards.
- `crates/service/src/errors`: `AppError` and error conversions.
- `crates/service/src/http`: server bootstrap, router assembly, OpenAPI, HTTP errors, request validation.
- `crates/service/src/range_store_builder`: SQLite source to binary range-store build pipeline.
- `crates/service/src/query`: hand query service, query result models, and dimension handle pool.
- `crates/service/src/routes`: Axum route handlers only.
- `crates/service/src/scripts`: CLI command parsing and execution entry points.
- `crates/service/src/storage`: manifest, metadata DB, and SQLite infrastructure.
- `crates/service/src/utils`: generic clock, SHA-256, and hex helpers only.
- `crates/service/src/verification`: standalone and source-cross verification.

Target test directories:

- `crates/service/tests/support`: fixtures shared by integration tests.
- `crates/service/tests/<domain>/<source-file>.test.rs`: integration test files explicitly registered in `Cargo.toml`.

---

### Task 1: Add Explicit `.test.rs` Test Target Mechanism

**Files:**
- Modify: `crates/service/Cargo.toml`
- Move: `crates/service/tests/verify_cli_args.rs` -> `crates/service/tests/scripts/verify_store.test.rs`

- [ ] **Step 1: Register the existing CLI test under a `.test.rs` path**

Move the existing `verify_cli_args.rs` test file to:

```text
crates/service/tests/scripts/verify_store.test.rs
```

Append this Cargo test configuration to `crates/service/Cargo.toml`:

```toml
autotests = false

[[test]]
name = "scripts_verify_store_test"
path = "tests/scripts/verify_store.test.rs"
```

- [ ] **Step 2: Run the renamed test**

Run:

```text
cargo test -p poker-hands-storage-service --test scripts_verify_store_test --target x86_64-pc-windows-msvc
```

Expected: the four verify CLI argument tests pass. This proves Cargo can run the required `.test.rs` naming scheme.

### Task 2: Move Existing Service Tests Into Named Integration Tests

**Files:**
- Modify: `crates/service/Cargo.toml`
- Create: `crates/service/tests/support/verify_store_fixture.rs`
- Create: `crates/service/tests/domain/action_schema.test.rs`
- Create: `crates/service/tests/domain/dimension.test.rs`
- Create: `crates/service/tests/domain/hole_cards.test.rs`
- Create: `crates/service/tests/config/service_config.test.rs`
- Create: `crates/service/tests/http/router.test.rs`
- Create: `crates/service/tests/storage/manifest/manifest_reader.test.rs`
- Create: `crates/service/tests/storage/sqlite/sqlite_connection.test.rs`
- Create: `crates/service/tests/range_store_builder/build_orchestrator.test.rs`
- Create: `crates/service/tests/verification/float32_precision.test.rs`
- Create: `crates/service/tests/verification/report/markdown_report.test.rs`
- Create: `crates/service/tests/verification/report/report_totals.test.rs`
- Create: `crates/service/tests/verification/standalone/standalone_runner.test.rs`
- Create: `crates/service/tests/verification/cross/source_cross_runner.test.rs`
- Delete old service test files after content is moved.

- [ ] **Step 1: Add Cargo targets for every service integration test**

Add these targets to `crates/service/Cargo.toml`:

```toml
[[test]]
name = "domain_action_schema_test"
path = "tests/domain/action_schema.test.rs"

[[test]]
name = "domain_dimension_test"
path = "tests/domain/dimension.test.rs"

[[test]]
name = "domain_hole_cards_test"
path = "tests/domain/hole_cards.test.rs"

[[test]]
name = "config_service_config_test"
path = "tests/config/service_config.test.rs"

[[test]]
name = "http_router_test"
path = "tests/http/router.test.rs"

[[test]]
name = "storage_manifest_reader_test"
path = "tests/storage/manifest/manifest_reader.test.rs"

[[test]]
name = "storage_sqlite_connection_test"
path = "tests/storage/sqlite/sqlite_connection.test.rs"

[[test]]
name = "range_store_builder_build_orchestrator_test"
path = "tests/range_store_builder/build_orchestrator.test.rs"

[[test]]
name = "verification_float32_precision_test"
path = "tests/verification/float32_precision.test.rs"

[[test]]
name = "verification_markdown_report_test"
path = "tests/verification/report/markdown_report.test.rs"

[[test]]
name = "verification_report_totals_test"
path = "tests/verification/report/report_totals.test.rs"

[[test]]
name = "verification_standalone_runner_test"
path = "tests/verification/standalone/standalone_runner.test.rs"

[[test]]
name = "verification_source_cross_runner_test"
path = "tests/verification/cross/source_cross_runner.test.rs"
```

- [ ] **Step 2: Move verifier fixture support**

Move `crates/service/tests/common/mod.rs` into:

```text
crates/service/tests/support/verify_store_fixture.rs
```

Each verifier integration test should import it with:

```rust
#[path = "../support/verify_store_fixture.rs"]
mod verify_store_fixture;
```

or the correct relative path from its nested directory.

- [ ] **Step 3: Move verifier integration tests**

Move:

```text
tests/verifier_precision_report.rs
  -> tests/verification/float32_precision.test.rs
  -> tests/verification/report/markdown_report.test.rs
  -> tests/verification/report/report_totals.test.rs

tests/verifier_standalone.rs
  -> tests/verification/standalone/standalone_runner.test.rs

tests/verifier_source_cross.rs
  -> tests/verification/cross/source_cross_runner.test.rs
```

Expected split:

- Float32 round-trip tests go to `float32_precision.test.rs`.
- Empty report Markdown shape test goes to `markdown_report.test.rs`.
- Structural failure total tests go to `report_totals.test.rs`.
- Standalone behavior tests go to `standalone_runner.test.rs`.
- Cross behavior tests go to `source_cross_runner.test.rs`.

- [ ] **Step 4: Move source inline tests into integration tests**

Move inline tests from these source files:

```text
action_schema.rs -> tests/domain/action_schema.test.rs
hand_dict.rs -> tests/domain/hole_cards.test.rs
naming.rs -> tests/domain/dimension.test.rs
config.rs -> tests/config/service_config.test.rs
manifest.rs -> tests/storage/manifest/manifest_reader.test.rs
sqlite.rs -> tests/storage/sqlite/sqlite_connection.test.rs
http.rs -> tests/http/router.test.rs
builder.rs -> tests/range_store_builder/build_orchestrator.test.rs
```

Do not move private helper-only tests by exposing private functions just for tests. Replace those with public-behavior assertions through `build_store`, manifest output, `QueryService`, or verifier reports.

- [ ] **Step 5: Run service tests**

Run:

```text
cargo test -p poker-hands-storage-service --target x86_64-pc-windows-msvc
```

Expected: every explicit service test target passes, and `src/lib.rs` / `src/main.rs` have zero inline tests.

### Task 3: Create Domain, Error, Config, And Storage Module Directories

**Files:**
- Move: `crates/service/src/action_schema.rs` -> `crates/service/src/domain/action_schema.rs`
- Move: `crates/service/src/hand_dict.rs` -> `crates/service/src/domain/hole_cards.rs`
- Move: `crates/service/src/naming.rs` -> `crates/service/src/domain/dimension.rs`
- Move: `crates/service/src/error.rs` -> `crates/service/src/errors/app_error.rs`
- Move: `crates/service/src/config.rs` -> `crates/service/src/config/service_config.rs`
- Move: `crates/service/src/manifest.rs` -> `crates/service/src/storage/manifest/mod.rs`
- Move: `crates/service/src/meta_db.rs` -> `crates/service/src/storage/metadata/metadata_reader.rs`
- Move: `crates/service/src/sqlite.rs` -> `crates/service/src/storage/sqlite/mod.rs`
- Create matching `mod.rs` facade files.
- Modify imports across `crates/service/src` and `crates/service/tests`.

- [ ] **Step 1: Create module facades**

Create:

```rust
// crates/service/src/domain/mod.rs
pub mod action_schema;
pub mod dimension;
pub mod hole_cards;
```

```rust
// crates/service/src/errors/mod.rs
pub mod app_error;

pub use app_error::AppError;
```

```rust
// crates/service/src/config/mod.rs
pub mod service_config;

pub use service_config::ServiceConfig;
```

```rust
// crates/service/src/storage/mod.rs
pub mod manifest;
pub mod metadata;
pub mod sqlite;
```

```rust
// crates/service/src/storage/metadata/mod.rs
pub mod metadata_reader;

pub use metadata_reader::{ConcreteLineRow, MetadataReader};
```

- [ ] **Step 2: Update `lib.rs`**

Replace old top-level declarations with:

```rust
pub mod config;
pub mod domain;
pub mod errors;
pub mod http;
pub mod query;
pub mod range_store_builder;
pub mod routes;
pub mod scripts;
pub mod storage;
pub mod verification;
```

Keep temporary compatibility exports only if a step needs them, then remove them before final validation.

- [ ] **Step 3: Update imports**

Apply these path changes:

```text
crate::action_schema -> crate::domain::action_schema
crate::hand_dict -> crate::domain::hole_cards
crate::naming -> crate::domain::dimension
crate::error::AppError -> crate::errors::AppError
crate::manifest -> crate::storage::manifest
crate::meta_db -> crate::storage::metadata
crate::sqlite -> crate::storage::sqlite
```

Use the equivalent `poker_hands_storage_service::...` paths in integration tests.

- [ ] **Step 4: Run tests**

Run:

```text
cargo test -p poker-hands-storage-service --target x86_64-pc-windows-msvc
```

Expected: service crate compiles and behavior tests pass.

### Task 4: Create Query And Range Store Builder Domains

**Files:**
- Move: `crates/service/src/query_service.rs` -> `crates/service/src/query/hand_query_service.rs`
- Move: `crates/service/src/pool.rs` -> `crates/service/src/query/dimension_handle_pool.rs`
- Move: `crates/service/src/builder.rs` -> `crates/service/src/range_store_builder/mod.rs`
- Create: `crates/service/src/query/mod.rs`
- Modify imports across service code and tests.

- [ ] **Step 1: Create query facade**

Create:

```rust
// crates/service/src/query/mod.rs
pub mod dimension_handle_pool;
pub mod hand_query_service;

pub use hand_query_service::{
    BatchItemResult, BatchStrategyResult, ErrorInfo, QueryResult, QueryService,
};
```

- [ ] **Step 2: Create range store builder facade**

Move `builder.rs` content to `range_store_builder/mod.rs` and keep the current public API names for this phase:

```rust
pub fn build_store(options: &BuildOptions) -> Result<BuildSummary, AppError>
pub fn discover_dimensions(connection: &Connection) -> Result<Vec<DimensionSpec>, AppError>
```

- [ ] **Step 3: Update imports**

Apply:

```text
crate::query_service -> crate::query::hand_query_service
crate::pool -> crate::query::dimension_handle_pool
crate::builder -> crate::range_store_builder
```

In route and script code, prefer facade imports:

```rust
use crate::query::QueryService;
use crate::range_store_builder::{build_store, BuildOptions, DimensionSpec};
```

- [ ] **Step 4: Run build/query tests**

Run:

```text
cargo test -p poker-hands-storage-service --test range_store_builder_build_orchestrator_test --target x86_64-pc-windows-msvc
cargo test -p poker-hands-storage-service --test http_router_test --target x86_64-pc-windows-msvc
```

Expected: builder and HTTP query workflows still pass.

### Task 5: Split HTTP Bootstrap, Routes, Scripts, And Verification Names

**Files:**
- Move: `crates/service/src/http.rs` -> `crates/service/src/http/mod.rs`
- Move: `crates/service/src/api_doc.rs` -> `crates/service/src/http/openapi.rs`
- Move: `crates/service/src/validation.rs` -> `crates/service/src/http/request_validation.rs`
- Move: `crates/service/src/routes/query.rs` -> `crates/service/src/routes/hand_query_routes.rs`
- Move: `crates/service/src/routes/metadata.rs` -> `crates/service/src/routes/metadata_routes.rs`
- Move: `crates/service/src/routes/health.rs` -> `crates/service/src/routes/health_routes.rs`
- Move: `crates/service/src/verifier` -> `crates/service/src/verification`
- Create: `crates/service/src/scripts/mod.rs`
- Move verify CLI arg parsing into scripts path if needed.

- [ ] **Step 1: Rename route modules**

Update `routes/mod.rs` to:

```rust
pub mod hand_query_routes;
pub mod health_routes;
pub mod metadata_routes;

use crate::errors::AppError;
use crate::http::HttpError;
```

Update router assembly to reference:

```rust
routes::health_routes::{health, ready}
routes::hand_query_routes::{batch, prewarm, query}
routes::metadata_routes::{concrete_lines, drill_scenario_lines}
```

- [ ] **Step 2: Rename verification module**

Move the current `verifier` directory to `verification` and update imports:

```text
crate::verifier -> crate::verification
poker_hands_storage_service::verifier -> poker_hands_storage_service::verification
```

Keep file names initially:

```text
catalog.rs
cli_args.rs
precision.rs
report.rs
source_cross.rs
standalone.rs
```

Then rename files during Task 6 when reporting and runner modules are split.

- [ ] **Step 3: Create scripts facade**

Create `crates/service/src/scripts/mod.rs` and move verify CLI parsing behind:

```rust
pub mod verify_store;
```

If full CLI command extraction is not completed in this task, re-export current parser from `verification::cli_args` temporarily:

```rust
pub use crate::verification::cli_args::{parse_verify_args, VerifyCommand};
```

- [ ] **Step 4: Run HTTP and verifier tests**

Run:

```text
cargo test -p poker-hands-storage-service --test http_router_test --target x86_64-pc-windows-msvc
cargo test -p poker-hands-storage-service --test verification_standalone_runner_test --target x86_64-pc-windows-msvc
cargo test -p poker-hands-storage-service --test verification_source_cross_runner_test --target x86_64-pc-windows-msvc
```

Expected: HTTP and verifier behavior still pass.

### Task 6: Split Large Business Files Without Behavior Changes

**Files:**
- Split: `crates/service/src/range_store_builder/mod.rs`
- Split: `crates/service/src/http/mod.rs`
- Split: `crates/service/src/storage/sqlite/mod.rs`
- Split: `crates/service/src/verification/report.rs`
- Split: `crates/service/src/verification/standalone.rs`
- Split: `crates/service/src/verification/source_cross.rs`

- [ ] **Step 1: Split range store builder**

Create files:

```text
range_store_builder/build_models.rs
range_store_builder/dimension_discovery.rs
range_store_builder/metadata_export.rs
range_store_builder/range_pack_encoder.rs
range_store_builder/action_schema_catalog.rs
range_store_builder/binary_store_writer.rs
range_store_builder/manifest_writer.rs
range_store_builder/build_orchestrator.rs
```

Move functions according to the design doc. `range_store_builder/mod.rs` should expose:

```rust
pub use build_models::{BuildOptions, BuildSummary, DimensionSpec};
pub use build_orchestrator::build_store;
pub use dimension_discovery::discover_dimensions;
```

- [ ] **Step 2: Split HTTP module**

Create files:

```text
http/app_state.rs
http/blocking_task.rs
http/error_response.rs
http/openapi.rs
http/request_validation.rs
http/router.rs
http/server.rs
```

`http/mod.rs` should expose:

```rust
pub use app_state::AppState;
pub use error_response::{ErrorResponse, HttpError};
pub use request_validation::{ValidatedJson, ValidateRequest};
pub use router::router;
pub use server::serve;
```

- [ ] **Step 3: Split SQLite module**

Create files:

```text
storage/sqlite/sqlite_dynamic_loader.rs
storage/sqlite/sqlite_connection.rs
storage/sqlite/sqlite_statement.rs
storage/sqlite/sqlite_value.rs
```

`storage/sqlite/mod.rs` should expose:

```rust
pub use sqlite_connection::Connection;
pub use sqlite_statement::Statement;
pub use sqlite_value::Value;
pub use sqlite_dynamic_loader::SqliteError;
```

- [ ] **Step 4: Split verification reporting and runners**

Create:

```text
verification/report/mod.rs
verification/report/report_model.rs
verification/report/report_totals.rs
verification/report/markdown_report.rs
verification/report/report_writer.rs
verification/standalone/mod.rs
verification/standalone/standalone_runner.rs
verification/standalone/manifest_checks.rs
verification/standalone/index_file_checks.rs
verification/standalone/binary_file_checks.rs
verification/standalone/index_pack_reconciliation.rs
verification/cross/mod.rs
verification/cross/source_cross_runner.rs
verification/cross/source_row_loader.rs
verification/cross/binary_source_reconciliation.rs
```

`verification/mod.rs` should expose:

```rust
pub mod catalog_checks;
pub mod cross;
pub mod float32_precision;
pub mod report;
pub mod standalone;
```

- [ ] **Step 5: Run full service tests**

Run:

```text
cargo test -p poker-hands-storage-service --target x86_64-pc-windows-msvc
```

Expected: all service tests pass.

### Task 7: Final Cleanup, Docs, And Full Validation

**Files:**
- Modify: `README.md`
- Modify: `docs/progress.md`
- Modify: `docs/superpowers/specs/2026-06-26-service-directory-refactor-design.md` if implementation differs.

- [ ] **Step 1: Remove compatibility shims**

Search:

```text
rg -n "pub mod (builder|query_service|sqlite|verifier|error|naming|hand_dict|manifest|meta_db)|crate::(builder|query_service|sqlite|verifier|error|naming|hand_dict|manifest|meta_db)|poker_hands_storage_service::(builder|query_service|sqlite|verifier|error|naming|hand_dict|manifest|meta_db)" crates/service
```

Expected: no stale module paths remain.

- [ ] **Step 2: Verify test naming**

Run:

```text
Get-ChildItem -Path crates\service\tests -Recurse -File | Where-Object { $_.Extension -eq ".rs" -and $_.FullName -notmatch "\\support\\" -and $_.Name -notmatch "\.test\.rs$" }
```

Expected: no output.

- [ ] **Step 3: Run full validation**

Run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

Expected: all commands pass.

- [ ] **Step 4: Run verifier smoke checks**

Run:

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify --mode standalone --dir data/range-strata --verify-checksum
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify --mode cross --dir data/range-strata --source data/sqlite/range.db --sample-size 10000 --verify-checksum
```

Expected: both verifier commands complete with zero failures and write JSON/Markdown reports.

- [ ] **Step 5: Update docs**

Update docs to describe the new service layout:

- `README.md`: add a short service module layout section.
- `docs/progress.md`: mention that the service crate was reorganized into domain modules and `.test.rs` integration tests.

Run:

```text
cargo fmt --all -- --check
```

Expected: formatting remains clean.

---

## Self-Review

Spec coverage:

- Business-oriented directories are covered by Tasks 3-6.
- Test naming and test directory requirements are covered by Tasks 1-2 and Task 7 Step 2.
- CLI, HTTP, report, binary format behavior preservation is covered by service tests and verifier smoke checks.
- Large file split is covered by Task 6.

Placeholder scan:

- This plan contains no `TODO`, `TBD`, or open-ended implementation placeholders.

Type consistency:

- Intermediate tasks keep current type names such as `QueryService`, `Connection`, and `Value` until all path migration is stable.
- The implementation may introduce aliases or final business names in a follow-up only after tests are green; behavior-preserving path refactor is the priority.
