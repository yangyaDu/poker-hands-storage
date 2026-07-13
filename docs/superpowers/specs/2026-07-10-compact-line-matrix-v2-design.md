# Proto LineMatrix Archive Design

## Goal

Store every line from each discovered 169-hand dimension as a separate random-access Proto archive while omitting SQLite rows where `hand_ev IS NULL`.

## Compatibility

The old V1 LineMatrix exporter and archive are removed. The current Proto scheme uses package `zenithstrat.gto.v2`, archive header version `2`, and manifest payload schema `zenithstrat.gto.v2.CompactLineMatrix`.

## Encoding

`valid_hand_bitmap` is in original hand-id space, LSB-first, and has fixed length 22 bytes for 169 hands. A set bit denotes that the hand has at least one retained source cell. Its set-bit rank maps `hand_id` to `global_compact_index`.

Each `action_hand_bitmap` is in global compact-index space, LSB-first, and has length `ceil(popcount(valid_hand_bitmap) / 8)`. `frequency_x10000` and `ev_x10000` are ordered by the set bits of that action bitmap and both have length `popcount(action_hand_bitmap)`.

The exporter excludes all `hand_ev IS NULL` rows in the SQLite query, regardless of source frequency. Action identity is `(action_type, action_size_x10000, amount_centi_bb)` and action columns are sorted by that identity.

## Archive

The archive retains `matrices.lmbin`, `matrices.lmidx`, `lines.db`, and `manifest.json`. Each index record is `u64 offset`, `u32 byte_length`, `u32 crc32c`. No whole-payload compression is added so single-record reads remain direct.

## Reading

After CRC verification and Protobuf decoding, the reader validates canonical bitmap lengths, zero padding bits, unique action identities, and array lengths. It builds a `hand_id -> global_compact_index` table and a `global_compact_index -> action_compact_index` table for each action. Value lookup is then O(1).

