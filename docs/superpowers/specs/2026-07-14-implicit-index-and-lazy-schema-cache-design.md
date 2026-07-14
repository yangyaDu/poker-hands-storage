# Implicit Index and Lazy Schema Cache Design

## Goal

Remove redundant `dimension_action_schemas` metadata and make every
`concrete_line_id` implicit in the fixed-width `.idx` record position. This is
a pre-production layout change: all local data directories must be rebuilt,
but the public format version remains `1`.

## Scope

- Remove `dimension_action_schemas` from the metadata schema, builder, core
  metadata APIs, validators, fixtures, and active documentation.
- Retain `action_schemas` as the global, de-duplicated action-definition
  dictionary referenced by every `.idx` record.
- Keep version `1` in `manifest.json`, `PFXI` headers, and `PFSP` headers.
- Replace the previous 22-byte index record with an 18-byte record.
- Use process-wide, single-ID lazy action-schema loading as the default.

HTTP and native SDK request and response contracts do not change.

## Format Contract

The manifest, `.idx`, and `.bin` headers remain version `1`. Because the
project has not released production data, the previous local 22-byte `.idx`
layout is not supported: readers reject it through exact index-file-size
validation and operators rebuild the directory with `storage-tools build`.

## Metadata Model

`meta.db` continues to store dimension metadata, concrete-line metadata, and
the global `action_schemas` table. `action_schemas` remains the source of truth
for `action_schema_id`, action count, encoded action definitions, and checksum.

`dimension_action_schemas` does not exist. A dimension's schema IDs can always
be derived from its own `.idx` records; no runtime query or validation scans a
separate dimension-schema relation.

## Implicit Index Layout

The `.idx` header is 16 bytes. Every record is exactly 18 bytes:

| Offset | Field | Type |
| --- | --- | --- |
| `0..4` | `action_schema_id` | `u32 LE` |
| `4..6` | `hand_count` | `u16 LE` |
| `6..10` | `bin_offset` | `u32 LE` |
| `10..14` | `byte_length` | `u32 LE` |
| `14..18` | `checksum` | `u32 LE` |

`concrete_line_id` is not stored. Each dimension must contain exactly the
ascending source-ID sequence `1..=record_count`. For a requested line `n`, the
reader uses `header_size + (n - 1) * record_size`; ID `0` and IDs greater than
`record_count` are absent. The in-memory `IdxRecord` exposes a synthesized ID
of `record_index + 1`.

The builder validates the source sequence while streaming rows ordered by
`concrete_line_id`. A non-one start, gap, or duplicate aborts the build. Source
IDs are never remapped.

## Lazy Action Schema Cache

`ActionSchemaCache` is process-wide and initially empty. A query resolves the
schema ID from its `.idx` record, then loads and decodes that one schema from
`action_schemas` only if it is absent from the shared cache. All later requests
and dimensions reuse the cached schema.

This keeps startup and single-line queries light. A first query for a new
schema pays one SQLite lookup and decode, which can affect cold-start p99 but
does not affect a true cache-hot query. Cache misses serialize access to the
SQLite connection; concurrent races remain correct because cache insertion is
idempotent.

`unique_action_schema_ids()` remains available as a future optional,
per-dimension prewarming mechanism. It is not called by the default request
path. A missing referenced schema reports `ACTION_SCHEMA_NOT_FOUND`.

## Build and Validation

1. The builder creates a `meta.db` without the removed table.
2. For each validated line, it reuses or inserts the global action schema,
   writes the 18-byte index record, and writes the pack payload.
3. Standalone validation checks the version-1 headers, exact index size and
   bounds, action-schema checksums, each index schema reference, and pack
   length derived from schema action count.
4. Cross verification resolves actions through `action_schemas` and does not
   inspect a dimension-schema relation.

## Error Behavior

| Condition | Result |
| --- | --- |
| Previous 22-byte local index layout | Index length mismatch; rebuild required |
| Source IDs are not `1..=N` | Build error identifying the unexpected ID |
| Requested ID is zero or beyond record count | Existing not-found result |
| `.idx` schema ID is absent | `ACTION_SCHEMA_NOT_FOUND` |
| Pack size disagrees with schema action count | Standalone verification failure |

## Test and Documentation Requirements

- Test implicit-ID lookup, 18-byte offsets, absence of the association table,
  valid and invalid source ID sequences, lazy cache reuse, and missing schemas.
- Keep version-1 header fixtures aligned with the 18-byte layout.
- Update active storage, query-chain, and verification documentation to remove
  the association table and describe the implicit-ID contract and lazy cache.

