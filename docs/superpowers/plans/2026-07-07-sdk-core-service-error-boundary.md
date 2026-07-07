# SDK Core Service Error Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split core, native SDK, and HTTP service contracts so core returns domain results or typed errors, native SDK returns payloads or throws `RangeStoreError`, and service remains the only HTTP envelope owner.

**Architecture:** Keep the query logic in `range-store-core`, but remove response-envelope and item-level error shapes from core results. Let `range-store-native` expose slim JavaScript payloads and convert N-API failures into a stable `RangeStoreError`. Let `service` translate the same core errors into HTTP status codes and `{ code, data, message }` envelopes.

**Tech Stack:** Rust 2021 workspace, napi-rs, Axum, utoipa, Bun test, Cargo integration tests.

## Global Constraints

- Native SDK methods must never return `{ code, data, message }`.
- Native SDK methods return direct payloads on success and throw `RangeStoreError` on failure.
- `RangeStoreError` exposes only `name`, `code`, and `message`.
- SDK error codes are string codes: `INVALID_ARGUMENT`, `DIMENSION_NOT_FOUND`, `DATA_FILE_NOT_FOUND`, `INVALID_FORMAT`, `META_DB_ERROR`, `ACTION_SCHEMA_NOT_FOUND`, `ABSTRACT_LINE_NOT_FOUND`, `CONCRETE_LINE_NOT_FOUND`, `HAND_STRATEGY_NOT_FOUND`, `DRILL_SCENARIO_NOT_FOUND`, `HANDS_NOT_FOUND`, `INTERNAL`.
- Hand parse failures map to `INVALID_ARGUMENT`, not `UNKNOWN_HAND`.
- Batch query semantics are all-or-nothing in core, native SDK, and service.
- Batch failure messages include request index, concrete line id, and dimension in the message string.
- HTTP service remains the only layer that returns numeric business codes and HTTP response envelopes.
- Keep changes scoped to query contracts, error mapping, tests, and docs.

---

## File Structure

- `range-store-core/src/query/store_query_service.rs`: Owns core query result structs, batch semantics, and low-level query error variants.
- `range-store-core/src/query/range_store_facade.rs`: Owns facade error-code mapping used by native SDK and service.
- `range-store-core/src/query/mod.rs`: Re-exports updated core query types.
- `range-store-core/Cargo.toml`: Registers a new core query contract integration test if added.
- `range-store-core/tests/query_contract.test.rs`: Verifies strict batch failure and error classification at the core boundary.
- `service/src/query/hand_query_service.rs`: Maps core results into service response structs without reintroducing item-level errors.
- `service/src/query/mod.rs`: Re-exports updated service query structs.
- `service/src/errors/app_error.rs`: Maps core errors into service error codes; removes `UNKNOWN_HAND`.
- `service/src/routes/hand_query_routes.rs`: Updates batch response payload and OpenAPI wording.
- `service/src/http/openapi.rs`: Updates OpenAPI envelope schemas for slim query and batch payloads.
- `service/tests/http/router.test.rs`: Verifies HTTP all-or-nothing batch behavior and `INVALID_ARGUMENT` semantics.
- `range-store-native/src/lib.rs`: Updates N-API response structs and error encoding.
- `range-store-native/index.js`: Removes envelope wrapping and implements `RangeStoreError`.
- `range-store-native/index.d.ts`: Exposes direct payload return types and `RangeStoreErrorCode`.
- `range-store-native/tests/sdk-contract.test.js`: Verifies direct payloads and thrown SDK errors.
- `range-store-native/tests/http-consistency.test.js`: Compares direct SDK payloads with HTTP `data`.
- `docs/native-sdk.md`: Documents direct payload and thrown errors.
- `docs/api-business-contract.md`: Documents service batch all-or-nothing and removes `UNKNOWN_HAND`.
- `docs/query-chain-explanation.md`: Updates SDK call chain examples.
- `README.md`: Updates public usage examples.

---

### Task 1: Core Query Result And Error Contract

**Status:** Implementation verified in worktree `C:\tmp\poker-hands-storage-task1` on branch `codex/task1-core-query-contract`; commit step pending explicit approval.

**Files:**
- Modify: `range-store-core/src/query/store_query_service.rs`
- Modify: `range-store-core/src/query/range_store_facade.rs`
- Modify: `range-store-core/src/query/mod.rs`
- Modify: `range-store-core/Cargo.toml`
- Create: `range-store-core/tests/query_contract.test.rs`

**Interfaces:**
- Consumes: `DimensionRef`, `StoreQueryService`, `RangeStoreFacade`, existing fixture binary format.
- Produces:
  - `QueryResult { actions: Vec<ActionResult> }`
  - `QueryBatchResult { results: Vec<QueryBatchItemResult> }`
  - `QueryBatchItemResult { concrete_line_id: u32, hole_cards: String, actions: Vec<ActionResult> }`
  - `StoreQueryError::InvalidArgument(String)`
  - `StoreQueryError::ConcreteLineNotFound { dimension: DimensionRef, concrete_line_id: u32 }`
  - `StoreQueryError::HandStrategyNotFound { dimension: DimensionRef, concrete_line_id: u32, hole_cards: String }`
  - `StoreQueryError::BatchItem { index: usize, concrete_line_id: u32, hole_cards: String, dimension: DimensionRef, source: Box<StoreQueryError> }`

- [x] **Step 1: Write the failing core contract test**

Add this test registration to `range-store-core/Cargo.toml`:

```toml
[[test]]
name = "query_contract_test"
path = "tests/query_contract.test.rs"
```

Create `range-store-core/tests/query_contract.test.rs` with tests that build a minimal store, open `RangeStoreFacade`, and assert the new contract:

```rust
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use range_store_core::crc32c::crc32c;
use range_store_core::dimension::DimensionRef;
use range_store_core::query::RangeStoreFacade;
use range_store_core::sqlite::{Connection, Value};
use range_store_core::types::{IDX_HEADER_SIZE, IDX_RECORD_SIZE, PFSP_HEADER_SIZE};

#[test]
fn single_query_returns_actions_only() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = build_query_test_store(temp.path());
    let store = RangeStoreFacade::open(&data_dir, 2, true).unwrap();

    let result = store
        .query_hand_strategy(&DimensionRef::new("default", 6, 100), 1, "AsAh")
        .unwrap();

    assert_eq!(result.actions.len(), 2);
    assert_eq!(result.actions[0].action_name, "fold");
}

#[test]
fn batch_query_fails_whole_request_for_invalid_hand() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = build_query_test_store(temp.path());
    let store = RangeStoreFacade::open(&data_dir, 2, true).unwrap();

    let error = store
        .query_batch(
            &DimensionRef::new("default", 6, 100),
            &[(1, "AA".to_owned()), (1, "AsXx".to_owned())],
        )
        .unwrap_err();

    assert_eq!(error.code(), "INVALID_ARGUMENT");
    assert!(error.to_string().contains("Batch item requests[1] failed"));
    assert!(error.to_string().contains("Invalid card format: AsXx"));
    assert!(error.to_string().contains("from concrete_line_id=1"));
    assert!(error.to_string().contains("dimension=default:6:100"));
}

#[test]
fn batch_query_fails_whole_request_for_missing_line() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = build_query_test_store(temp.path());
    let store = RangeStoreFacade::open(&data_dir, 2, true).unwrap();

    let error = store
        .query_batch(
            &DimensionRef::new("default", 6, 100),
            &[(1, "AA".to_owned()), (999, "KK".to_owned())],
        )
        .unwrap_err();

    assert_eq!(error.code(), "CONCRETE_LINE_NOT_FOUND");
    assert!(error.to_string().contains("Batch item requests[1] failed"));
    assert!(error.to_string().contains("concrete_line_id=999"));
    assert!(error.to_string().contains("dimension=default:6:100"));
}
```

Use a local `build_query_test_store` helper copied from `service/tests/support/api_test_fixture.rs`, adjusted to import `range_store_core::sqlite::{Connection, Value}` and write one `manifest.json`, `meta.db`, `.idx`, and `.bin` under the temp output directory. Keep the helper private to this test file.

- [x] **Step 2: Run the core test and verify it fails**

Run:

```powershell
cargo test -p range-store-core --test query_contract_test
```

Expected: FAIL because `QueryResult` still has `input_hole_cards` and `hand_code`, `query_batch` still returns per-item errors, and `UNKNOWN_HAND` still exists in facade mapping.

- [x] **Step 3: Update core result structs**

In `range-store-core/src/query/store_query_service.rs`, replace the query and batch result structs with:

```rust
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub actions: Vec<ActionResult>,
}

#[derive(Debug, Clone)]
pub struct QueryBatchResult {
    pub results: Vec<QueryBatchItemResult>,
}

#[derive(Debug, Clone)]
pub struct QueryBatchItemResult {
    pub concrete_line_id: u32,
    pub hole_cards: String,
    pub actions: Vec<ActionResult>,
}
```

Remove `BatchItemError`, `hand_code`, and optional `actions` from core batch result types.

- [x] **Step 4: Update core error variants**

In `StoreQueryError`, add typed variants and keep source errors structured:

```rust
#[derive(Debug)]
pub enum StoreQueryError {
    Manifest(ManifestError),
    ActionSchema(ActionSchemaLoadError),
    ActionFilter(ActionFilterParseError),
    HandlePool(HandlePoolError),
    InvalidArgument(String),
    ActionSchemaNotFound(u32),
    Io(String),
    ConcreteLineNotFound {
        dimension: DimensionRef,
        concrete_line_id: u32,
    },
    HandStrategyNotFound {
        dimension: DimensionRef,
        concrete_line_id: u32,
        hole_cards: String,
    },
    BatchItem {
        index: usize,
        concrete_line_id: u32,
        hole_cards: String,
        dimension: DimensionRef,
        source: Box<StoreQueryError>,
    },
    Internal(String),
}
```

Update `From<HandDictError>` to produce `StoreQueryError::InvalidArgument(error.to_string())`.

- [x] **Step 5: Update core display strings**

Use these exact display formats:

```rust
Self::InvalidArgument(message) => write!(f, "{message}"),
Self::ConcreteLineNotFound { dimension, concrete_line_id } => write!(
    f,
    "Concrete line not found: concrete_line_id={concrete_line_id}, dimension={}:{}:{}",
    dimension.strategy, dimension.player_count, dimension.depth_bb
),
Self::HandStrategyNotFound {
    dimension,
    concrete_line_id,
    hole_cards,
} => write!(
    f,
    "Hand {hole_cards} is outside the range for action line concrete_line_id={concrete_line_id} in dimension {}:{}:{}",
    dimension.strategy, dimension.player_count, dimension.depth_bb
),
Self::BatchItem {
    index,
    concrete_line_id,
    dimension,
    source,
    ..
} => write!(
    f,
    "Batch item requests[{index}] failed: {source} from concrete_line_id={concrete_line_id}, dimension={}:{}:{}",
    dimension.strategy, dimension.player_count, dimension.depth_bb
),
```

- [x] **Step 6: Update core batch implementation**

Change `query_batch` to return `Result<QueryBatchResult, StoreQueryError>`. Iterate in input order, and return the first failure as `StoreQueryError::BatchItem`. Keep the existing optimization that groups by `concrete_line_id` only if it does not reintroduce partial item errors. A simple first pass may call the existing single-query logic per item:

```rust
pub fn query_batch(
    &self,
    dimension: &DimensionRef,
    requests: &[(u32, String)],
) -> Result<QueryBatchResult, StoreQueryError> {
    let mut results = Vec::with_capacity(requests.len());
    for (index, (concrete_line_id, hole_cards)) in requests.iter().enumerate() {
        let item = self
            .query(dimension, *concrete_line_id, hole_cards)
            .map_err(|source| StoreQueryError::BatchItem {
                index,
                concrete_line_id: *concrete_line_id,
                hole_cards: hole_cards.clone(),
                dimension: dimension.clone(),
                source: Box::new(source),
            })?;
        results.push(QueryBatchItemResult {
            concrete_line_id: *concrete_line_id,
            hole_cards: hole_cards.clone(),
            actions: item.actions,
        });
    }
    Ok(QueryBatchResult { results })
}
```

After the contract is passing, preserve or restore the grouped `query_many_hands` optimization in the same all-or-nothing shape if benchmark impact matters in this change.

- [x] **Step 7: Update facade mapping**

In `range-store-core/src/query/range_store_facade.rs`, map:

```rust
StoreQueryError::InvalidArgument(_) => "INVALID_ARGUMENT",
StoreQueryError::ActionFilter(_) => "INVALID_ARGUMENT",
StoreQueryError::ConcreteLineNotFound { .. } => "CONCRETE_LINE_NOT_FOUND",
StoreQueryError::HandStrategyNotFound { .. } => "HAND_STRATEGY_NOT_FOUND",
StoreQueryError::BatchItem { source, .. } => source.code(),
```

Remove the `UNKNOWN_HAND` mapping. Update `public_code()` so `INVALID_ARGUMENT` maps to `1000` and no `UNKNOWN_HAND` branch remains.

- [x] **Step 8: Run core tests**

Run:

```powershell
cargo test -p range-store-core
```

Expected: PASS.

- [ ] **Step 9: Commit core contract changes**

```powershell
git add range-store-core/Cargo.toml range-store-core/src/query/store_query_service.rs range-store-core/src/query/range_store_facade.rs range-store-core/src/query/mod.rs range-store-core/tests/query_contract.test.rs
git commit -m "refactor: tighten core query error contract"
```

---

### Task 2: Service Query Contract And HTTP Error Semantics

**Files:**
- Modify: `service/src/query/hand_query_service.rs`
- Modify: `service/src/query/mod.rs`
- Modify: `service/src/errors/app_error.rs`
- Modify: `service/src/routes/hand_query_routes.rs`
- Modify: `service/src/http/openapi.rs`
- Modify: `service/tests/http/router.test.rs`

**Interfaces:**
- Consumes: `range_store_core::query::QueryResult`, `QueryBatchResult`, `RangeStoreError`.
- Produces:
  - HTTP single query data: `{ "actions": [...] }`
  - HTTP batch data: `{ "results": [{ "concrete_line_id", "hole_cards", "actions" }] }`
  - HTTP batch failure: non-200 error envelope.

- [ ] **Step 1: Write failing service tests**

In `service/tests/http/router.test.rs`, update `hand_strategy_query_returns_expected_frequency`:

```rust
assert_eq!(result["code"], 0);
assert!(result["data"].get("input_hole_cards").is_none());
assert!(result["data"].get("hand_code").is_none());
assert_eq!(result["data"]["actions"].as_array().unwrap().len(), 2);
```

Replace `hand_strategy_batch_query_returns_per_item_results` with:

```rust
#[tokio::test]
async fn hand_strategy_batch_query_is_all_or_nothing() {
    let (_directory, app) = build_test_app();

    let valid_batch = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "requests": [
            { "concrete_line_id": 1, "hole_cards": "AA" },
            { "concrete_line_id": 1, "hole_cards": "KK" }
        ]
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy-batch",
        Some(valid_batch),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["code"], 0);
    assert_eq!(result["data"]["results"][0]["concrete_line_id"], 1);
    assert_eq!(result["data"]["results"][0]["hole_cards"], "AA");
    assert!(result["data"]["results"][0].get("hand_code").is_none());
    assert!(result["data"]["results"][0].get("strategy").is_none());
    assert!(result["data"]["results"][0]["actions"].is_array());

    let invalid_batch = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "requests": [
            { "concrete_line_id": 1, "hole_cards": "AA" },
            { "concrete_line_id": 1, "hole_cards": "AsXx" }
        ]
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy-batch",
        Some(invalid_batch),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert!(error["data"].is_null());
    assert_error_message_contains(
        &error,
        &[
            "Batch item requests[1] failed",
            "Invalid card format: AsXx",
            "from concrete_line_id=1",
            "dimension=default:6:100",
        ],
    );
}
```

- [ ] **Step 2: Run service tests and verify failure**

Run:

```powershell
cargo test -p poker-hands-storage-service --test http_router_test
```

Expected: FAIL because service still returns per-item batch errors and `hand_code`.

- [ ] **Step 3: Update service query structs**

In `service/src/query/hand_query_service.rs`, use:

```rust
#[derive(Debug, Clone, Serialize, ToSchema, PartialEq)]
pub struct QueryResult {
    pub actions: Vec<ActionResult>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BatchItemResult {
    pub concrete_line_id: u32,
    pub hole_cards: String,
    pub actions: Vec<ActionResult>,
}
```

Remove `BatchStrategyResult` and `ErrorInfo` from the service query module.

- [ ] **Step 4: Update service mapping functions**

Use:

```rust
fn query_result_from_core(result: CoreQueryResult) -> QueryResult {
    QueryResult {
        actions: result.actions.into_iter().map(action_from_core).collect(),
    }
}

fn batch_item_from_core(item: CoreBatchItemResult) -> BatchItemResult {
    BatchItemResult {
        concrete_line_id: item.concrete_line_id,
        hole_cards: item.hole_cards,
        actions: item.actions.into_iter().map(action_from_core).collect(),
    }
}
```

Change `QueryService::query_batch` to consume `CoreQueryBatchResult` and return `Vec<BatchItemResult>`:

```rust
pub fn query_batch(
    &self,
    dimension: &DimensionRef,
    requests: &[(u32, String)],
) -> Result<Vec<BatchItemResult>, AppError> {
    Ok(self
        .facade
        .query_batch(dimension, requests)?
        .results
        .into_iter()
        .map(batch_item_from_core)
        .collect())
}
```

- [ ] **Step 5: Update service error mapping**

In `service/src/errors/app_error.rs`:

```rust
pub fn public_code(&self) -> i32 {
    match self.code {
        "INVALID_ARGUMENT" => 1000,
        "BIN_FILE_NOT_FOUND"
        | "PACK_NOT_FOUND"
        | "DATA_FILE_NOT_FOUND"
        | "DRILL_SCENARIO_NOT_FOUND"
        | "ABSTRACT_LINE_NOT_FOUND"
        | "DIMENSION_NOT_FOUND"
        | "ACTION_SCHEMA_NOT_FOUND"
        | "CONCRETE_LINE_NOT_FOUND"
        | "HAND_STRATEGY_NOT_FOUND"
        | "ACTION_NOT_FOUND"
        | "HANDS_NOT_FOUND" => 404,
        "SERVICE_UNAVAILABLE" => 503,
        _ => 500,
    }
}
```

Change `impl From<HandDictError> for AppError` to:

```rust
impl From<HandDictError> for AppError {
    fn from(error: HandDictError) -> Self {
        Self::invalid_argument(error.to_string())
    }
}
```

- [ ] **Step 6: Update route and OpenAPI wording**

In `service/src/routes/hand_query_routes.rs`, change `BatchPayload` doc to:

```rust
/// All-or-nothing batch query result. Any invalid item fails the entire request.
results: Vec<BatchItemResult>,
```

In the `#[utoipa::path]` for `/range/hand-strategy-batch`, change the 200 description to:

```rust
(status = 200, description = "All-or-nothing batch query result.", body = crate::http::openapi::BatchResponseEnvelope),
```

In `service/src/http/openapi.rs`, remove imports and schema registrations for `BatchStrategyResult` and `ErrorInfo`. Ensure `BatchItemResult` schema contains only `concrete_line_id`, `hole_cards`, and `actions`.

- [ ] **Step 7: Run service tests**

Run:

```powershell
cargo test -p poker-hands-storage-service
```

Expected: PASS.

- [ ] **Step 8: Commit service contract changes**

```powershell
git add service/src/query/hand_query_service.rs service/src/query/mod.rs service/src/errors/app_error.rs service/src/routes/hand_query_routes.rs service/src/http/openapi.rs service/tests/http/router.test.rs
git commit -m "refactor: align service batch error semantics"
```

---

### Task 3: Native N-API Rust Contract

**Files:**
- Modify: `range-store-native/src/lib.rs`

**Interfaces:**
- Consumes: `RangeStoreFacade` updated result and error types.
- Produces:
  - `QueryHandStrategyResponse { actions }`
  - `QueryBatchResponse { results: Vec<QueryBatchItemResponse> }`
  - `QueryBatchItemResponse { concrete_line_id, hole_cards, actions }`
  - N-API errors encoded as `RANGE_STORE_ERROR:{code}:{message}`.

- [ ] **Step 1: Update Rust N-API response structs**

In `range-store-native/src/lib.rs`, change:

```rust
#[napi(object)]
pub struct QueryHandStrategyResponse {
    pub actions: Vec<ActionResult>,
}

#[napi(object)]
pub struct QueryBatchResponse {
    pub results: Vec<QueryBatchItemResponse>,
}

#[napi(object)]
pub struct QueryBatchItemResponse {
    pub concrete_line_id: u32,
    pub hole_cards: String,
    pub actions: Vec<ActionResult>,
}
```

Remove `QueryBatchItemError` and the `input_hole_cards` / `hand_code` fields from native response structs.

- [ ] **Step 2: Update native query methods**

Use:

```rust
Ok(QueryHandStrategyResponse {
    actions: result
        .actions
        .into_iter()
        .map(action_result_from_core)
        .collect(),
})
```

For batch:

```rust
let results = self
    .inner
    .query_batch(&dimension, &requests)
    .map_err(to_napi_error)?
    .results
    .into_iter()
    .map(|item| QueryBatchItemResponse {
        concrete_line_id: item.concrete_line_id,
        hole_cards: item.hole_cards,
        actions: item.actions.into_iter().map(action_result_from_core).collect(),
    })
    .collect();
Ok(QueryBatchResponse { results })
```

- [ ] **Step 3: Encode stable error code in N-API error message**

Change `to_napi_error` to:

```rust
fn to_napi_error(error: RangeStoreError) -> Error {
    Error::new(
        Status::GenericFailure,
        format!("RANGE_STORE_ERROR:{}:{}", error.code(), error),
    )
}
```

- [ ] **Step 4: Run Rust compile checks for native**

Run:

```powershell
cargo check -p range-store-native
```

Expected: PASS.

- [ ] **Step 5: Commit native Rust contract changes**

```powershell
git add range-store-native/src/lib.rs
git commit -m "refactor: slim native napi query payloads"
```

---

### Task 4: JavaScript SDK Payloads And RangeStoreError

**Files:**
- Modify: `range-store-native/index.js`
- Modify: `range-store-native/index.d.ts`
- Modify: `range-store-native/tests/sdk-contract.test.js`

**Interfaces:**
- Consumes: N-API direct payloads and encoded error message.
- Produces:
  - `RangeStoreError`
  - Direct SDK payloads
  - SDK tests for throw behavior.

- [ ] **Step 1: Write failing SDK tests**

In `range-store-native/tests/sdk-contract.test.js`, update success assertions:

```js
expect(store.stats()).toMatchObject({
  openHandleCount: 0,
  schemaCount: 0,
});

const line = store.getConcreteLines({
  ...baseDimension(),
  concreteLine: "F-F-F",
});
expect(line.lines).toHaveLength(1);

const result = store.queryBatch({
  ...baseDimension(),
  items: [
    { concreteLineId: 1, holeCards: "AA" },
    { concreteLineId: 1, holeCards: "KK" },
  ],
});
expect(result.results[0]).toMatchObject({
  concreteLineId: 1,
  holeCards: "AA",
});
expect(result.results[0].actions.length).toBeGreaterThan(0);
expect(result.results[0].handCode).toBeUndefined();
expect(result.results[0].error).toBeUndefined();
```

Add throw assertion:

```js
test("throws RangeStoreError for invalid batch item", () => {
  const store = openStore();

  expect(() =>
    store.queryBatch({
      ...baseDimension(),
      items: [
        { concreteLineId: 1, holeCards: "AA" },
        { concreteLineId: 1, holeCards: "AsXx" },
      ],
    }),
  ).toThrow(RangeStoreError);

  try {
    store.queryBatch({
      ...baseDimension(),
      items: [
        { concreteLineId: 1, holeCards: "AA" },
        { concreteLineId: 1, holeCards: "AsXx" },
      ],
    });
  } catch (error) {
    expect(error).toBeInstanceOf(RangeStoreError);
    expect(error.code).toBe("INVALID_ARGUMENT");
    expect(error.message).toContain("Batch item requests[1] failed");
    expect(error.message).toContain("Invalid card format: AsXx");
    expect(error.message).toContain("from concrete_line_id=1");
    return;
  }
  throw new Error("expected queryBatch to throw");
});
```

- [ ] **Step 2: Run SDK test and verify failure**

Run:

```powershell
Set-Location range-store-native
bun test tests/sdk-contract.test.js
```

Expected: FAIL because SDK still returns envelopes and `RangeStoreError` is not exported.

- [ ] **Step 3: Implement `RangeStoreError`**

In `range-store-native/index.js`, add:

```js
export class RangeStoreError extends Error {
  constructor(code, message, options = undefined) {
    super(message, options);
    this.name = "RangeStoreError";
    this.code = code;
  }
}
```

Add parser:

```js
function toRangeStoreError(error) {
  const message = error instanceof Error ? error.message : String(error);
  const match = /^RANGE_STORE_ERROR:([A-Z_]+):(.*)$/s.exec(message);
  if (match) {
    return new RangeStoreError(match[1], match[2], { cause: error });
  }
  return new RangeStoreError("INTERNAL", message, { cause: error });
}

function callNative(fn) {
  try {
    return fn();
  } catch (error) {
    throw toRangeStoreError(error);
  }
}
```

- [ ] **Step 4: Remove envelope helpers**

Delete `apiErrorResult` and `normalizeApiResult`. Update SDK methods to use `callNative` and return direct payloads:

```js
queryHandStrategy(request) {
  const result = callNative(() =>
    this.#native.queryHandStrategy({
      ...toNativeDimension(request),
      concreteLineId: request.concreteLineId,
      holeCards: request.holeCards,
    }),
  );
  return {
    actions: result.actions.map(fromNativeAction),
  };
}

queryBatch(request) {
  const result = callNative(() =>
    this.#native.queryBatch({
      ...toNativeDimension(request),
      items: request.items.map((item) => ({
        concreteLineId: item.concreteLineId,
        holeCards: item.holeCards,
      })),
    }),
  );
  return {
    results: result.results.map((item) => ({
      concreteLineId: item.concreteLineId,
      holeCards: item.holeCards,
      actions: item.actions.map(fromNativeAction),
    })),
  };
}
```

Update all other methods to return direct payloads:

```js
getConcreteLines(request) {
  const result = callNative(() => this.#native.getConcreteLines({ ... }));
  return { lines: result.lines.map(...) };
}

getAbstractLines(request) {
  const result = callNative(() => this.#native.getAbstractLines({ ... }));
  return { abstractLines: result.abstractLines };
}

handsByActions(request) {
  const result = callNative(() => this.#native.handsByActions({ ... }));
  return { holeCards: result.holeCards };
}

prewarm(request) {
  const result = callNative(() => this.#native.prewarm(toNativeDimension(request)));
  return { openHandleCount: result.openHandleCount };
}

stats() {
  const result = this.#native.stats();
  return {
    schemaCount: result.schemaCount,
    openHandleCount: result.openHandleCount,
    knownDimensions: result.knownDimensions,
  };
}
```

- [ ] **Step 5: Update TypeScript declarations**

In `range-store-native/index.d.ts`, delete `ApiResponse<T>`. Add:

```ts
export type RangeStoreErrorCode =
  | "INVALID_ARGUMENT"
  | "DIMENSION_NOT_FOUND"
  | "DATA_FILE_NOT_FOUND"
  | "INVALID_FORMAT"
  | "META_DB_ERROR"
  | "ACTION_SCHEMA_NOT_FOUND"
  | "ABSTRACT_LINE_NOT_FOUND"
  | "CONCRETE_LINE_NOT_FOUND"
  | "HAND_STRATEGY_NOT_FOUND"
  | "DRILL_SCENARIO_NOT_FOUND"
  | "HANDS_NOT_FOUND"
  | "INTERNAL"

export declare class RangeStoreError extends Error {
  name: "RangeStoreError"
  code: RangeStoreErrorCode
}
```

Change method declarations:

```ts
getConcreteLines(request: ConcreteLinesRequest): ConcreteLinesData
getAbstractLines(request: AbstractLinesRequest): AbstractLinesData
handsByActions(request: HandsByActionsRequest): HandsByActionsResponse
queryHandStrategy(request: QueryHandStrategyRequest): QueryHandStrategyResponse
queryBatch(request: QueryBatchRequest): QueryBatchResponse
prewarm(request: DimensionInput): PrewarmResponse
stats(): StatsResponse
```

Use:

```ts
export interface QueryHandStrategyResponse {
  actions: Array<ActionResult>
}

export interface QueryBatchItemResponse {
  concreteLineId: number
  holeCards: string
  actions: Array<ActionResult>
}

export interface QueryBatchResponse {
  results: Array<QueryBatchItemResponse>
}
```

- [ ] **Step 6: Build native and run SDK tests**

Run:

```powershell
Set-Location range-store-native
bun run build:native
bun test tests/sdk-contract.test.js
```

Expected: PASS.

- [ ] **Step 7: Commit JS SDK contract changes**

```powershell
git add range-store-native/index.js range-store-native/index.d.ts range-store-native/tests/sdk-contract.test.js
git commit -m "refactor: expose direct native sdk payloads"
```

---

### Task 5: HTTP Consistency And Documentation

**Files:**
- Modify: `range-store-native/tests/http-consistency.test.js`
- Modify: `docs/native-sdk.md`
- Modify: `docs/api-business-contract.md`
- Modify: `docs/query-chain-explanation.md`
- Modify: `README.md`
- Modify: `docs/release-qna-v1.1.0.md`
- Modify: `docs/data-verification-and-format-validation.md`

**Interfaces:**
- Consumes: updated SDK direct payloads and HTTP envelope `data`.
- Produces: updated docs and consistency tests that compare direct SDK payload to HTTP `data`.

- [ ] **Step 1: Update HTTP consistency test**

In `range-store-native/tests/http-consistency.test.js`, keep `requireOk` for HTTP responses only. For SDK responses, remove `requireOk`:

```js
const sdkConcrete = store.getConcreteLines({
  ...sdkDimension(),
  concreteLine: "F-F-F",
});
const httpConcrete = requireOk(
  await postJson("/range/concrete-lines", {
    ...httpDimension(),
    concrete_line: "F-F-F",
  }),
);
expect(normalizeConcreteLines(sdkConcrete.lines)).toEqual(
  normalizeConcreteLines(httpConcrete.lines),
);
```

For single query:

```js
const sdkHand = store.queryHandStrategy({
  ...sdkDimension(),
  concreteLineId: 1,
  holeCards: "AA",
});
const httpHand = requireOk(
  await postJson("/range/hand-strategy", {
    ...httpDimension(),
    concrete_line_id: 1,
    hole_cards: "AA",
  }),
);
expect(normalizeHandStrategy(sdkHand)).toEqual(normalizeHandStrategy(httpHand));
```

For batch, only use valid items:

```js
const sdkBatch = store.queryBatch({
  ...sdkDimension(),
  items: [
    { concreteLineId: 1, holeCards: "AA" },
    { concreteLineId: 1, holeCards: "KK" },
  ],
});
const httpBatch = requireOk(
  await postJson("/range/hand-strategy-batch", {
    ...httpDimension(),
    requests: [
      { concrete_line_id: 1, hole_cards: "AA" },
      { concrete_line_id: 1, hole_cards: "KK" },
    ],
  }),
);
expect(normalizeBatch(sdkBatch)).toEqual(normalizeBatch(httpBatch));
```

Update `normalizeBatch` to read:

```js
function normalizeBatch(data) {
  return data.results.map((item) => ({
    concreteLineId: item.concreteLineId ?? item.concrete_line_id,
    holeCards: item.holeCards ?? item.hole_cards,
    actionNames: (item.actions ?? []).map((action) => normalizeAction(action).actionName),
  }));
}
```

- [ ] **Step 2: Run HTTP consistency when service URL is available**

If `PHS_HTTP_URL` is set and the service is running, run:

```powershell
Set-Location range-store-native
bun test tests/http-consistency.test.js
```

Expected: PASS, or SKIP if `PHS_HTTP_URL` is unset.

- [ ] **Step 3: Update SDK docs**

In `docs/native-sdk.md`, replace the module positioning sentence with:

```markdown
- JavaScript side loads `index.node` through `index.js`, returns direct payloads, and throws `RangeStoreError` on failures.
```

Replace the method table return column:

```markdown
| `getConcreteLines(request)` | `{ lines }` | Query concrete lines by abstract or concrete line. |
| `getAbstractLines(request)` | `{ abstractLines }` | Query drill scenario abstract lines. |
| `handsByActions(request)` | `{ holeCards }` | Filter hands by concrete line id, actions, and frequency. |
| `queryHandStrategy(request)` | `{ actions }` | Query one hand's strategy. |
| `queryBatch(request)` | `{ results: [{ concreteLineId, holeCards, actions }] }` | All-or-nothing batch hand strategy query. |
| `prewarm(request)` | `{ openHandleCount }` | Open one dimension in the handle pool. |
| `stats()` | `{ schemaCount, openHandleCount, knownDimensions }` | Inspect SDK cache and handle state. |
```

Add error section:

```markdown
## Error contract

Native SDK methods throw `RangeStoreError`.

```ts
class RangeStoreError extends Error {
  name: "RangeStoreError";
  code: RangeStoreErrorCode;
}
```

`RangeStoreErrorCode` values are `INVALID_ARGUMENT`, `DIMENSION_NOT_FOUND`, `DATA_FILE_NOT_FOUND`, `INVALID_FORMAT`, `META_DB_ERROR`, `ACTION_SCHEMA_NOT_FOUND`, `ABSTRACT_LINE_NOT_FOUND`, `CONCRETE_LINE_NOT_FOUND`, `HAND_STRATEGY_NOT_FOUND`, `DRILL_SCENARIO_NOT_FOUND`, `HANDS_NOT_FOUND`, and `INTERNAL`.
```

- [ ] **Step 4: Update HTTP API docs**

In `docs/api-business-contract.md`:

```markdown
| `INVALID_ARGUMENT` | 1000 |
```

Remove the `UNKNOWN_HAND` row. Update batch rules:

```markdown
- Batch query is all-or-nothing. If any item has an invalid hand, missing concrete line, or missing hand strategy, the whole HTTP request returns an error envelope.
- Successful batch items contain `concrete_line_id`, `hole_cards`, and `actions`.
- Batch responses do not contain per-item `error` or `strategy` fields.
```

Use this failed batch example:

```json
{
  "code": 1000,
  "data": null,
  "message": "Batch item requests[1] failed: Invalid card format: AsXx from concrete_line_id=1, dimension=default:6:100"
}
```

- [ ] **Step 5: Update remaining docs references**

In `docs/query-chain-explanation.md` and `README.md`, replace SDK examples that show:

```js
{ code: 0, data: ..., message: null }
```

with direct payload examples:

```js
const result = store.queryHandStrategy({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLineId: 42,
  holeCards: "AKs",
});

// { actions: [...] }
```

In `docs/release-qna-v1.1.0.md`, replace the old per-item batch explanation with:

```markdown
Batch query is now strict. A single invalid item fails the whole request and returns a batch-context error message.
```

In `docs/data-verification-and-format-validation.md`, replace `UNKNOWN_HAND` with `INVALID_ARGUMENT`.

- [ ] **Step 6: Run full verification**

Run:

```powershell
cargo test
```

Then:

```powershell
Set-Location range-store-native
bun run build:native
bun test tests/sdk-contract.test.js
```

If an HTTP service is running:

```powershell
Set-Location range-store-native
bun test tests/http-consistency.test.js
```

Expected: all Cargo tests PASS; native SDK contract test PASS; HTTP consistency PASS or SKIP when no `PHS_HTTP_URL` exists.

- [ ] **Step 7: Commit docs and consistency changes**

```powershell
git add range-store-native/tests/http-consistency.test.js docs/native-sdk.md docs/api-business-contract.md docs/query-chain-explanation.md README.md docs/release-qna-v1.1.0.md docs/data-verification-and-format-validation.md
git commit -m "docs: update sdk and service query contracts"
```

---

## Self-Review

**Spec coverage:** The plan covers core result slimming, strict batch failure, native direct payloads, `RangeStoreError`, service all-or-nothing batch behavior, removal of `UNKNOWN_HAND`, tests, and docs.

**Placeholder scan:** The plan contains concrete file paths, commands, expected outcomes, type signatures, and message formats. It does not contain deferred implementation markers.

**Type consistency:** The planned result names are consistent across layers: core uses `QueryBatchResult` and `QueryBatchItemResult`; service exposes `BatchItemResult`; native SDK exposes `QueryBatchResponse` and `QueryBatchItemResponse`. Error codes use `INVALID_ARGUMENT` for hand parse failures across all layers.
