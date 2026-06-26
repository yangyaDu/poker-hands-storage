# Service Directory Refactor Design

## 背景

`crates/service/src` 当前同时放着 HTTP、CLI、SQLite 动态加载、构建器、查询服务、manifest、命名、verifier 等文件。顶层文件过多，且部分文件名偏技术或通用，例如 `builder.rs`、`http.rs`、`pool.rs`、`naming.rs`，需要读内部实现才能确认业务职责。

用户选择第三种方案：按领域重新设计模块边界，允许大面积调整 public module path 和内部职责拆分。

## 目标

1. `service` crate 目录按业务领域分组，顶层只保留 `lib.rs`、`main.rs` 和少量一级领域目录。
2. 文件名必须能表达业务含义，减少 `builder`、`pool`、`naming` 这类需要上下文解释的名字。
3. `utils` 只放纯通用工具，禁止成为业务逻辑垃圾桶。
4. CLI/运维入口放到 `scripts`，HTTP handler 继续放到 `routes`。
5. 测试代码统一放到 `crates/service/tests`，测试文件命名使用对应代码文件的 `<name>.test.rs`。
6. 重构后 `cargo fmt`、`cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`、`cargo test --workspace --target x86_64-pc-windows-msvc` 必须通过。

## 非目标

1. 不改变现有 CLI 行为、HTTP 路由、JSON schema、binary 格式和 verifier report 格式。
2. 不在这次结构重构中实现 7c benchmark 功能。
3. 不把 `range-store-core` 一并重构；本设计聚焦 `poker-hands-storage-service`。
4. 不把所有业务常量强行集中到 `constants`。靠近业务文件更清晰的局部常量仍保留在业务模块内。

## 设计原则

按领域优先，而不是按技术层机械分类。比如 pack 编码属于 range store 构建领域，不应该放进 `utils`；SQLite 动态加载属于 storage/sqlite 基础设施，不应该放在顶层 `sqlite.rs`。

`lib.rs` 作为公开 facade，顶层目录名承担架构导航职责。内部模块可以多层拆分，但每个文件都要能回答三个问题：负责什么、给谁用、依赖什么。

旧 public path 不作为长期兼容目标。实施期间可以用短期 re-export 降低单步 diff 风险，但最终代码和 tests 都应使用新路径。

## 目标目录结构

```text
crates/service/src/
  lib.rs
  main.rs

  config/
    mod.rs
    environment.rs
    service_config.rs

  constants/
    mod.rs
    http_defaults.rs
    report_paths.rs
    runtime_defaults.rs

  errors/
    mod.rs
    app_error.rs

  scripts/
    mod.rs
    cli_args.rs
    build_store.rs
    query_hand.rs
    serve_http.rs
    verify_store.rs

  http/
    mod.rs
    app_state.rs
    blocking_task.rs
    error_response.rs
    openapi.rs
    router.rs
    server.rs

  routes/
    mod.rs
    health_routes.rs
    hand_query_routes.rs
    metadata_routes.rs

  domain/
    mod.rs
    action_schema.rs
    dimension.rs
    hole_cards.rs

  storage/
    mod.rs
    manifest/
      mod.rs
      manifest_model.rs
      manifest_reader.rs
      queryable_dimensions.rs
    metadata/
      mod.rs
      metadata_reader.rs
    sqlite/
      mod.rs
      sqlite_connection.rs
      sqlite_dynamic_loader.rs
      sqlite_statement.rs
      sqlite_value.rs

  range_store_builder/
    mod.rs
    action_schema_catalog.rs
    binary_store_writer.rs
    build_models.rs
    build_orchestrator.rs
    dimension_discovery.rs
    manifest_writer.rs
    metadata_export.rs
    range_pack_encoder.rs

  query/
    mod.rs
    dimension_handle_pool.rs
    hand_query_results.rs
    hand_query_service.rs

  verification/
    mod.rs
    catalog_checks.rs
    float32_precision.rs
    report/
      mod.rs
      markdown_report.rs
      report_model.rs
      report_totals.rs
      report_writer.rs
    standalone/
      mod.rs
      binary_file_checks.rs
      index_file_checks.rs
      index_pack_reconciliation.rs
      manifest_checks.rs
      standalone_runner.rs
    cross/
      mod.rs
      binary_source_reconciliation.rs
      source_row_loader.rs
      source_cross_runner.rs

  utils/
    mod.rs
    clock.rs
    hex.rs
    sha256.rs
```

`scripts` 在这里表示 CLI/运维命令入口，不表示一次性脚本。它承接当前 `main.rs` 中的 `run_build`、`run_query`、`run_verify` 和 `serve` 分发逻辑，让 `main.rs` 只负责初始化 tracing、读取 argv、调用脚本入口和退出码处理。

## 现有文件映射

| 当前文件 | 目标位置 | 说明 |
|---|---|---|
| `action_schema.rs` | `domain/action_schema.rs` | action blob decode 和 action 类型是业务领域模型 |
| `hand_dict.rs` | `domain/hole_cards.rs` | 牌型解析和 169 hand id 映射，命名从字典改成业务概念 |
| `naming.rs` | `domain/dimension.rs` | `DimensionRef`、dimension key、表名/文件名规则归到维度领域 |
| `config.rs` | `config/service_config.rs` + `config/environment.rs` | 配置模型与 env 解析拆开 |
| `error.rs` | `errors/app_error.rs` | 应用错误模型独立成 errors 目录 |
| `http.rs` | `http/router.rs`、`http/server.rs`、`http/app_state.rs`、`http/error_response.rs`、`http/openapi.rs` | HTTP bootstrap、router、state、error response 拆分 |
| `api_doc.rs` | `http/openapi.rs` | OpenAPI 属于 HTTP API 描述 |
| `routes/query.rs` | `routes/hand_query_routes.rs` | 明确是 hand query route，不是 query service |
| `routes/metadata.rs` | `routes/metadata_routes.rs` | route 文件名显式带 routes |
| `routes/health.rs` | `routes/health_routes.rs` | route 文件名显式带 routes |
| `validation.rs` | `http/error_response.rs` 或 `routes/request_validation.rs` | `ValidatedJson` 和字段校验贴近 HTTP request 层 |
| `sqlite.rs` | `storage/sqlite/*` | 动态加载、connection、statement、value 分文件 |
| `manifest.rs` | `storage/manifest/*` | manifest model、reader、queryable filtering 拆分 |
| `meta_db.rs` | `storage/metadata/metadata_reader.rs` | meta.db reader 是 storage 领域 |
| `builder.rs` | `range_store_builder/*` | 按构建流程拆分，避免单文件 1000 行 |
| `query_service.rs` | `query/hand_query_service.rs` + `query/hand_query_results.rs` | 查询服务和 response model 分离 |
| `pool.rs` | `query/dimension_handle_pool.rs` | pool 的业务对象是 dimension handle |
| `verifier/*` | `verification/*` | verifier 改为名词化领域 `verification`，内部按 standalone/cross/report 拆分 |
| `main.rs` | 保留，但变薄 | 调用 `scripts::run_cli`，不承载业务解析和执行 |

## Public Facade

最终 `lib.rs` 建议只公开一级领域模块：

```rust
pub mod config;
pub mod constants;
pub mod domain;
pub mod errors;
pub mod http;
pub mod query;
pub mod range_store_builder;
pub mod routes;
pub mod scripts;
pub mod storage;
pub mod utils;
pub mod verification;
```

关键 public API 路径调整：

| 旧路径 | 新路径 |
|---|---|
| `builder::build_store` | `range_store_builder::build_store` |
| `builder::BuildOptions` | `range_store_builder::BuildOptions` |
| `builder::DimensionSpec` | `domain::dimension::DimensionSpec` 或 `range_store_builder::DimensionSpec` facade re-export |
| `config::ServiceConfig` | `config::ServiceConfig` |
| `error::AppError` | `errors::AppError` |
| `hand_dict::parse_hole_cards` | `domain::hole_cards::parse_hole_cards` |
| `naming::DimensionRef` | `domain::dimension::DimensionRef` |
| `query_service::QueryService` | `query::HandQueryService` |
| `sqlite::Connection` | `storage::sqlite::SqliteConnection` |
| `sqlite::Value` | `storage::sqlite::SqliteValue` |
| `verifier::standalone::run_standalone_verify` | `verification::standalone::run_standalone_verification` |
| `verifier::source_cross::run_cross_verify` | `verification::cross::run_source_cross_verification` |

The CLI binary remains stable from the user perspective:

```text
poker-hands-storage-service build ...
poker-hands-storage-service query ...
poker-hands-storage-service verify ...
poker-hands-storage-service serve
```

## Builder Split

`builder.rs` is the largest file and should be split by build pipeline stages:

- `build_models.rs`: `BuildOptions`, `BuildSummary`, `DimensionSpec`, internal row structs.
- `dimension_discovery.rs`: discover `range_data_*` tables and select requested dimensions.
- `metadata_export.rs`: copy drill/concrete/action metadata into `meta.db`.
- `range_pack_encoder.rs`: group source rows and encode sparse hand-major packs.
- `action_schema_catalog.rs`: normalize action names and deduplicate action schemas.
- `binary_store_writer.rs`: write PFSP/PFXI headers, idx records, temp files, atomic rename.
- `manifest_writer.rs`: produce manifest dimensions and write `manifest.json`.
- `build_orchestrator.rs`: public `build_store` orchestration only.

SHA-256 and UTC formatting move to `utils/sha256.rs` and `utils/clock.rs` because they are generic and reused by builder/verifier/reporting.

## Verification Split

`verification` should replace `verifier` as the business area name. The split should make standalone and cross flows readable independently:

- `catalog_checks.rs`: meta.db catalog validation.
- `float32_precision.rs`: bit-exact float32 helpers and stats.
- `standalone/standalone_runner.rs`: orchestration and report writing.
- `standalone/manifest_checks.rs`: manifest read/validation failure conversion.
- `standalone/index_file_checks.rs`: PFXI header/order/record checks.
- `standalone/binary_file_checks.rs`: PFSP header/size/checksum checks.
- `standalone/index_pack_reconciliation.rs`: idx/pack byte-level consistency and hand id checks.
- `cross/source_cross_runner.rs`: orchestration against source SQLite.
- `cross/source_row_loader.rs`: source DB row loading and dimension row counts.
- `cross/binary_source_reconciliation.rs`: per-cell action/frequency/EV reconciliation.
- `report/report_model.rs`: report structs and totals input model.
- `report/report_totals.rs`: total calculation and repair suggestions.
- `report/markdown_report.rs`: Markdown rendering.
- `report/report_writer.rs`: JSON/Markdown file output.

## Test Layout And Naming

Requirement: test files should live outside source and use `<code-file-name>.test.rs`.

Rust detail: Cargo default integration test discovery only reliably handles root-level test files. To support nested mirrored paths and `.test.rs` names, `crates/service/Cargo.toml` should use explicit integration test targets:

```toml
autotests = false

[[test]]
name = "domain_action_schema_test"
path = "tests/domain/action_schema.test.rs"

[[test]]
name = "verification_standalone_runner_test"
path = "tests/verification/standalone/standalone_runner.test.rs"
```

Test support helpers are not test targets and should not use `.test.rs`:

```text
crates/service/tests/support/
  verify_store_fixture.rs
  http_service_fixture.rs
```

Target service test layout:

```text
crates/service/tests/
  support/
    http_service_fixture.rs
    verify_store_fixture.rs

  domain/
    action_schema.test.rs
    dimension.test.rs
    hole_cards.test.rs

  config/
    service_config.test.rs

  storage/
    manifest/
      manifest_reader.test.rs
      queryable_dimensions.test.rs
    sqlite/
      sqlite_connection.test.rs

  range_store_builder/
    build_orchestrator.test.rs
    dimension_discovery.test.rs
    range_pack_encoder.test.rs

  query/
    hand_query_service.test.rs

  http/
    router.test.rs
    error_response.test.rs

  verification/
    float32_precision.test.rs
    report/
      markdown_report.test.rs
      report_totals.test.rs
    standalone/
      standalone_runner.test.rs
      index_pack_reconciliation.test.rs
    cross/
      source_cross_runner.test.rs

  scripts/
    verify_store.test.rs
```

Existing service inline tests should be moved into these files. Existing verifier integration tests should be renamed and split:

| 当前测试文件 | 目标测试文件 |
|---|---|
| `tests/verify_cli_args.rs` | `tests/scripts/verify_store.test.rs` |
| `tests/verifier_precision_report.rs` | `tests/verification/float32_precision.test.rs`, `tests/verification/report/markdown_report.test.rs`, `tests/verification/report/report_totals.test.rs` |
| `tests/verifier_standalone.rs` | `tests/verification/standalone/standalone_runner.test.rs` |
| `tests/verifier_source_cross.rs` | `tests/verification/cross/source_cross_runner.test.rs` |
| `tests/common/mod.rs` | `tests/support/verify_store_fixture.rs` |

## Migration Strategy

1. Add the new test target mechanism first with one small `.test.rs` file and run `cargo test -p poker-hands-storage-service --target x86_64-pc-windows-msvc` to prove Cargo accepts the naming scheme.
2. Move tests out of inline modules and current integration test files into the target `tests/` tree. No production behavior changes in this step.
3. Create empty module directories and facade `mod.rs` files.
4. Move domain/config/errors modules and update imports.
5. Split `storage/sqlite` and `storage/manifest`, then update builder/query/verifier imports.
6. Split `range_store_builder` into pipeline files while preserving `build_store` behavior.
7. Split HTTP bootstrap and routes.
8. Split query service and dimension handle pool.
9. Rename and split `verifier` into `verification`.
10. Remove temporary compatibility re-exports, run full validation, and update README/progress docs.

Each migration step should compile and test independently. Large file splits should use `git mv` or exact file moves in the implementation plan so history remains readable.

## Risks And Mitigations

| Risk | Mitigation |
|---|---|
| Public module path churn breaks many imports | Use facade modules during intermediate steps, remove only after all imports are migrated |
| `.test.rs` naming not auto-discovered | Use explicit `[[test]]` targets and `autotests = false` |
| `builder.rs` split accidentally changes binary output | Keep byte-for-byte fixture tests and rerun standalone/cross verifier smoke after split |
| Verifier report split changes JSON/Markdown shape | Keep report snapshot-style assertions for required keys and headings |
| Too much moved in one commit | Implement by domain phase, validating after every phase |
| `utils` grows unclear | Only generic clock/hex/sha256/filesystem helpers go into `utils`; business logic stays in domain modules |

## Validation Plan

Required after implementation:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

Smoke checks after the final module move:

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify --mode standalone --dir data/range-strata --verify-checksum
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify --mode cross --dir data/range-strata --source data/sqlite/range.db --sample-size 10000 --verify-checksum
```

## Definition Of Done

1. `crates/service/src` no longer contains business files directly at top level except `lib.rs` and `main.rs`.
2. Each top-level module directory has an explicit responsibility documented by names and `mod.rs`.
3. All service tests live under `crates/service/tests`.
4. Test files corresponding to production files use `<name>.test.rs`.
5. Test support files live under `tests/support` and are not compiled as standalone test targets.
6. No stale imports reference removed modules such as `builder`, `query_service`, `sqlite`, or `verifier`.
7. CLI commands, HTTP routes, report schema, and binary verification behavior remain unchanged.
