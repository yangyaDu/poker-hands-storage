# Proto LineMatrix Archive

Updated: 2026-07-13

The repository has one LineMatrix storage scheme: the compact Proto archive.
The old V1 LineMatrix exporter and archive implementation have been removed.
The v2 payload schema is self-contained: `ActionType` and `HandEncoding` are
defined in the same file and there is no V1 schema dependency.

## Current Schema

The payload schema is:

```text
storage-tools/proto/zenithstrat/gto/v2/compact_matrix.proto
zenithstrat.gto.v2.CompactLineMatrix
```

The payload layout is:

```text
CompactLineMatrix
|- schema_version = 2
|- hand_encoding
|- actions[]
|  `- CompactActionColumn
|     |- action_type
|     |- amount_centi_bb
|     |- action_size_x10000
|     |- frequency_x10000[]
|     |- ev_x10000[]
|     `- action_hand_bitmap
`- valid_hand_bitmap
```

`action_type`, `amount_centi_bb`, and `action_size_x10000` identify one action
column and are stored once per column. The frequency and EV arrays have one
entry per set bit in `action_hand_bitmap`.

## Bitmap Contract

All bitmaps are LSB-first:

```text
byte_index = index >> 3
bit_index  = index & 7
mask       = 1 << bit_index
```

For the current `HAND_ENCODING_169` implementation:

```text
valid_hand_bitmap:  original hand_id -> global_compact_index
action_hand_bitmap: global_compact_index -> action_compact_index
frequency_x10000:   action_compact_index -> frequency
ev_x10000:          action_compact_index -> EV
```

The set-bit rank in `valid_hand_bitmap` produces the global compact index.
The set-bit rank in an action bitmap produces the action-local compact index.
Therefore:

```text
frequency_x10000.len == ev_x10000.len
frequency_x10000.len == popcount(action_hand_bitmap)
```

The reader builds both maps once after decoding a matrix. A lookup is then
O(1): `hand_id -> global_compact_index -> action_compact_index`.

Rows where `hand_ev IS NULL` are filtered by the source query and are never
encoded. A real EV value of zero remains a normal encoded value.

## Archive Layout

Export one dimension, defaulting to `default:6:100`:

```powershell
cargo run -p poker-hands-storage-tools -- export-compact-line-matrix-archive `
  --source-db data\sqlite\range.db `
  --out-dir reports\line-matrix-compact-default-6max-100BB `
  --dimension default:6:100
```

Export and verify every discovered dimension:

```powershell
cargo run -p poker-hands-storage-tools --release -- export-all-compact-line-matrix-archives `
  --source-db data\sqlite\range.db `
  --out-dir reports\line-matrix-compact-all
```

Each archive contains:

```text
manifest.json
lines.db
matrices.lmbin
matrices.lmidx
```

`matrices.lmbin` stores one raw Proto payload after another. Each
`matrices.lmidx` record is exactly 16 bytes:

```text
u64 offset
u32 byte_length
u32 crc32c
```

The nth record maps to `concrete_line_id = n + 1`. The reader memory-maps the
data and index files, validates CRC32C, decodes a single payload on demand,
and supports full-archive verification.

## Query Service

`ProtoRangeQueryService` provides the first core-compatible query shape:

```text
query_hand_strategy(dimension, concrete_line_id, hole_cards) -> QueryResult
query_batch(dimension, requests) -> QueryBatchResult
query_hands_by_actions(dimension, concrete_line_id, filters, frequency) -> Vec<String>
query_hands_by_action_names(dimension, concrete_line_id, action_names, frequency) -> Vec<String>
```

It reuses `range_store_core::query::QueryResult` and `ActionResult`. The
service checks the requested dimension, parses the hand with the core hand
dictionary, reads one matrix, and converts each retained action to the core
result representation. Batch requests are grouped by `concrete_line_id`, so
the service reads each referenced matrix at most once before restoring input
order in `QueryBatchResult`. Hands-by-actions reuses the core `ActionFilter`
and `FrequencyFilter` semantics: strict frequency threshold, exact amount
matching, and OR semantics across non-empty filters.

Proto never stores `hand_ev IS NULL` cells. Therefore a strict core/Proto
comparison filters core actions whose `hand_ev` is `None` before comparing
action identity, frequency, and EV. The service returns core-style error codes
for a missing dimension, concrete line, or retained hand strategy. Batch
failures return `BATCH_ITEM_ERROR` for the lowest failing request index.
For hands-by-actions comparisons, core results must apply the same NULL EV
filter before evaluating action filters.

`ProtoRangeStoreFacade` is the multi-dimension entry point for a root emitted
by `export-all-compact-line-matrix-archives`. It discovers child directories
from their manifests, selects the archive by the standard dimension key (for
example `default:6max:100BB`), and keeps at most `max_open_handles` mapped
archive readers through LRU eviction. Its query methods delegate to
`ProtoRangeQueryService`; `prewarm` opens a selected dimension before requests
arrive.

`benchmark-compact-vs-core --compact-dir <proto-range-storage-root>` uses the
same facade in its query-plan, hot-query, and cold-worker paths. The supplied
directory must therefore be the export-all root, not a single dimension child
directory.

## Commands

```text
export-compact-line-matrix-archive
export-all-compact-line-matrix-archives
verify-compact-line-matrix-archive
benchmark-compact-vs-core
```

The compact archive is the Proto storage format. The existing `.bin/.idx`
range store remains a separate native binary format used for comparison.
