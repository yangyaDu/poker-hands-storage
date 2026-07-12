# Compact LineMatrix V3 Design

## Goal

Introduce a V3 schema and reader optimization that solves four problems while keeping V2 archives fully readable:

1. `read_matrix` rebuilds map#1 + map#2 on every call -> LRU cache.
2. Action identity lookup is a linear scan -> identity extracted to a sorted array, binary search.
3. Every read opens `File::open` -> mmap reuses file handles.
4. `Vec<Vec<i16>>` nested allocation + serial `verify_all` -> flatten / eliminate + rayon parallel.

## Compatibility

V2 archives (magic `LMCN`/`LMCX`, version 2) remain readable by the V2 reader and are rejected by the V3 reader. V3 archives (magic `LM3N`/`LM3X`, version 3) are read only by the V3 reader. V2 CLI commands are unchanged. No existing function signature changes, no existing proto field is renumbered.

## Phase 0 Gate

Whether V3 takes "full V3" or "partial V3" form depends on production action coverage. Phase 0 decodes existing V2 archives and collects:

- `valid_hand_count` distribution (min/mean/p50/p90/p99).
- `action_count` distribution.
- `action_coverage_ratio` = `popcount(action_hand_bitmap) / valid_hand_count`.
- `bloatFactor` = `estimated_v3_entries / v2_actual_entries`.

| bloatFactor | Branch | Notes |
| --- | --- | --- |
| < 1.5 | Form A (full V3) | identity extraction + drop action-local compact, map#2 disappears |
| 1.5 - 3.0 | Review with user | Whether query-path gains justify storage cost |
| > 3.0 | Form B (partial V3) | Only extract identity, keep action-local compact, map#2 flattened |

## Form A: Full V3

Schema changes:

- `CompactActionIdentity` is extracted from `CompactActionColumn` into `action_identities[]`, sorted; array index = `action_index`.
- `frequency_x10000` / `ev_x10000` length = `valid_hand_count` (global compact space).
- `action_hand_bitmap` is retained, length `ceil(valid_hand_count/8)`, degenerated to a validity flag.
- Array subscript directly = `global_compact_index`; no local-index conversion.

Reader changes:

- `DecodedCompactLineMatrixV3` keeps only map#1 (`hand_id_to_global_index`).
- map#2 disappears entirely.
- `action_value(action_index, hand_id)` = `frequency[action_index][global_index]`.
- `action_value_by_identity(...)` binary-searches `action_identities`.

## Form B: Partial V3

Schema changes:

- `CompactActionIdentity` is extracted into `action_identities[]`, sorted.
- `frequency_x10000` / `ev_x10000` length stays = `popcount(action_hand_bitmap)` (action-local compact preserved).
- `action_hand_bitmap` retains compact index semantics.
- map#3 degenerates to array subscript (binary search on sorted identity).

Reader changes:

- `DecodedCompactLineMatrixV3` keeps map#1 + map#2.
- map#2 is flattened to `Vec<i16>` + `Vec<u16>` offset table (eliminates `Vec<Vec<i16>>`).
- `action_value_by_identity(...)` binary-searches `action_identities` to get `action_index`, then map#1 -> map#2.

## Shared Reader Optimizations

- mmap replaces per-read `File::open` (follows `range-store-core/src/bin_reader.rs` `memmap2::Mmap` pattern; `_file` kept alive, `Mmap::map(&file)` read-only, slice access).
- LRU caches `DecodedCompactLineMatrixV3` (key = `concrete_line_id`, value = `Arc<DecodedCompactLineMatrixV3>`).
- `verify_all` uses rayon parallel iteration, bypassing LRU via a private `decode_matrix_uncached`. A `verify_all_sequential` fallback is kept for tests/debug.

## Query Path

V2 per-read: `File::open(index)` + `File::open(data)` + `fs::metadata` + seek/read index + seek/read payload + CRC + protobuf decode + validate + build map#1 + build map#2 (per action). `action_value`: map#1 -> map#2 -> frequency.

V3 cache miss: (mmap open, no `File::open`) read index slice + read data slice + CRC + protobuf decode + simplified validate + build map#1; no map#2 (Form A) or flattened map#2 (Form B). `action_value`: map#1 -> frequency (Form A direct).

V3 cache hit: LRU lookup -> `Arc::clone` (O(1)). `action_value`: map#1 -> frequency.

Benefit sources: mmap removes 4-5 syscalls/read; LRU removes repeat decode + validate + map builds; map#2 removal (Form A) removes one Vec indirection; map#2 flatten (Form B) removes N allocations and improves cache locality; identity binary search removes O(actions) scan.

## Storage Impact

Form A per-matrix delta = `sum_actions (valid_hand_count - popcount(action_hand_bitmap)) * 2 bytes` (padding entry: frequency=0 is 1-byte varint + ev=0 is 1-byte zigzag = 2 bytes). Identity extraction itself does not change storage; `action_hand_bitmap` is retained at `ceil(valid_hand_count/8)` bytes/action.

Form B: frequency/ev length unchanged, identity extraction does not change storage. Storage is essentially flat; only the reader path is optimized.

Phase 0 provides the actual `bloatFactor` distribution because `valid_hand_count` may differ per line.

## Archive Structure

```rust
pub struct CompactLineMatrixV3Archive {
    data_mmap: Mmap,
    index_mmap: Mmap,
    _data_file: File,
    _index_file: File,
    matrix_count: u64,
    cache: Mutex<LruCache<u64, Arc<DecodedCompactLineMatrixV3>>>,
}
```

`open(dir)`: read manifest, validate version=3 / payload_schema; `File::open` data + index; `unsafe Mmap::map`; validate header from mmap slice (magic / version / count); validate index file size = `HEADER_SIZE + matrix_count * INDEX_RECORD_SIZE`; create `LruCache::unbounded()` (default capacity 1024).

`read_matrix(concrete_line_id)`: LRU hit -> `Arc::clone`; miss -> read IndexRecord from `index_mmap` slice (position = `HEADER_SIZE + (id-1) * 16`), read payload from `data_mmap[offset..offset+byte_length]`, CRC32C verify, protobuf decode, `DecodedCompactLineMatrixV3::new` (validate + build map#1), wrap `Arc`, insert LRU, return `Arc::clone`.

## Validation

`validate_compact_line_matrix_v3` checks:

- `schema_version == 3`.
- `hand_encoding == HAND_ENCODING_169`.
- `action_identities` non-empty, sorted, unique.
- `action_data.len() == action_identities.len()`.
- Form A: each `action_data` `frequency`/`ev` length = `valid_hand_count`; `action_hand_bitmap` length = `ceil(valid_hand_count/8)`.
- Form B: each `action_data` `frequency`/`ev` length = `popcount(action_hand_bitmap)`.
- `valid_hand_bitmap` 22 bytes, padding bits zero.

## New Dependencies

`storage-tools/Cargo.toml`: `memmap2 = "0.9"`, `lru = "0.12"`, `rayon = "1"`.
