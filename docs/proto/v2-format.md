# Proto V2 格式规范

更新日期：2026-07-14

Payload schema 为 `zenithstrat.gto.v2.CompactLineMatrix`，定义见
[`compact_matrix.proto`](../../storage-tools/proto/zenithstrat/gto/v2/compact_matrix.proto)。

| field | tag | 语义 |
| --- | ---: | --- |
| `CompactLineMatrix.schema_version` | 1 | 固定 `2`。 |
| `CompactLineMatrix.hand_encoding` | 2 | 当前 exporter/reader 仅接受 `HAND_ENCODING_169`。`HAND_ENCODING_1326_COMBO` 仅是 schema 预留枚举。 |
| `CompactLineMatrix.actions` | 3 | 行动列集合，不得为空。 |
| `CompactLineMatrix.valid_hand_bitmap` | 100 | 原始 `hand_id` 域，169 手牌时固定 22 bytes。 |
| `CompactActionColumn.action_type` | 1 | 行动类型，不能为 `UNSPECIFIED`。 |
| `CompactActionColumn.amount_centi_bb` | 2 | action 后本轮总投入，`1/100 BB`。 |
| `CompactActionColumn.action_size_x10000` | 3 | action size，`1/10000`。 |
| `CompactActionColumn.frequency_x10000` | 4 | action 局部紧凑数组，范围 `0..=10000`。 |
| `CompactActionColumn.ev_x10000` | 5 | 与 frequency 一一对应的 `sint32` 紧凑数组。 |
| `CompactActionColumn.action_hand_bitmap` | 6 | `global_compact_index` 域位图。 |

行动身份是 `(action_type, action_size_x10000, amount_centi_bb)`，同一 matrix 中不得重复。
写入前数值按 `round(frequency * 10000)`、`round(hand_ev * 10000)`、
`round(action_size * 10000)`、`round(amount_bb * 100)` 量化。

## NULL 与位图

SQLite `hand_ev IS NULL` 的 cell 在导出时直接过滤，绝不编码；真实零 EV 正常编码为 0。
任何 Core/SQLite 对比必须应用同一 NULL 过滤和 `x10000` 量化。

所有 bitmap 使用 LSB-first：

```text
byte_index = index >> 3
bit_index  = index & 7
mask       = 1 << bit_index
```

映射链是 V2 的固定规则：

```text
valid_hand_bitmap:  original hand_id -> global_compact_index
action_hand_bitmap: global_compact_index -> action_compact_index
frequency_x10000:   action_compact_index -> frequency
ev_x10000:          action_compact_index -> EV
```

`valid_hand_bitmap` 的置位 rank 给出 global index；某 action bitmap 的置位 rank 给出
action-local index。`action_hand_bitmap` 长度为
`ceil(popcount(valid_hand_bitmap) / 8)`，尾部 padding bit 必须为零，并且：

```text
frequency_x10000.len == ev_x10000.len
frequency_x10000.len == popcount(action_hand_bitmap)
union(all action_hand_bitmap) == all valid global compact bits
```

因此 `valid_hand_bitmap` 表示该行动线至少有一个已导出 action cell 的手牌，不是所有
169 张手牌的通用有效性。

## 文件布局

每个维度目录固定为：

```text
manifest.json
lines.db
matrices.lmbin
matrices.lmidx
```

manifest 固定声明 `format = "LMSP"`、`version = 2`、payload schema、维度、matrix
schema/encoding、matrix 数量及文件名/大小。

data/index 文件各有 16-byte little-endian header：`magic`（data=`LMCN`，index=`LMCX`）、
`u16 version=2`、`u16 header_size=16`、`u64 record_count`。data header 后拼接 raw Protobuf
payload；index header 后每条记录严格为：

```text
u64 offset
u32 byte_length
u32 crc32c
```

`concrete_line_id` 从 1 连续到 `matrix_count`，其记录位置为
`16 + (concrete_line_id - 1) * 16`。

`lines.db` 是每维度 SQLite metadata，含 `concrete_lines` 和复制的
`drill_scenario_lines_{strategy}`；源 `depth` 在目标表中命名为 `drill_depth`。

## 校验

打开时校验 manifest、header、文件长度、matrix count 与索引边界。单 matrix 读取仅在
`verify_checksums=true` 时校验 CRC32C；Rust 默认 open options 开启，基准通常关闭。
完整验证命令和 `verify_all` 始终校验 CRC、Protobuf decode 与全部格式不变量。
