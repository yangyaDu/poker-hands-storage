# 单行动线 Protobuf 导出方案

更新日期：2026-07-10

## 1. 目标与范围

当前实现从源 `range.db` 中查询一条具体行动线，把该行动线对应的 GTO 矩阵导出为独立的 Protobuf 文件。

第一版范围：

- 仅支持 `HAND_ENCODING_169`，即翻前固定 169 手牌。
- 支持按 `concrete_line_id` 或 `concrete_line` 文本查询。
- 支持不同手牌拥有不同数量的 action。
- 能区分“手牌没有该 action”“该 action 的 EV 为 NULL”和“该 action 的 EV 等于 0”。
- 这是 `storage-tools` 的离线导出能力，不替换当前 PFSP/PFXI 在线查询链路。

1326 combo 的枚举和 bitmap 语义已经在 schema 中保留，但不属于第一版导出范围。

## 2. 权威 Schema

权威 `.proto` 文件：

```text
storage-tools/proto/zenithstrat/gto/v1/matrix.proto
```

Protobuf package：

```text
zenithstrat.gto.v1
```

`LineMatrix` 与 `ActionColumn` 的包含关系：

```text
LineMatrix
├── schema_version
├── gto_data_version
├── hand_encoding
├── actions[]
│   └── ActionColumn
│       ├── action_type
│       ├── amount_centi_bb
│       ├── action_size_x10000
│       ├── frequency_x10000[hand_idx]
│       ├── ev_x10000[hand_idx]
│       ├── action_hand_bitmap
│       └── ev_null_bitmap
└── invalid_hand_bitmap
```

`LineMatrix.actions` 是当前行动线所有手牌出现过的唯一 action 并集。单个手牌拥有的 action 数量可以少于 `actions` 数量，但不可能多于该数量。

某个手牌也可以在当前行动线上完全没有策略数据，此时它在所有 `action_hand_bitmap` 中均为 0。这不等于 `invalid_hand_bitmap=1`：前者表示行动线未覆盖该手牌，后者表示手牌本身因公共牌等原因非法。

## 3. ActionColumn 语义

一个 action 由以下三个字段共同确定：

```text
(action_type, action_size_x10000, amount_centi_bb)
```

这三个字段都是 `ActionColumn` 的单个标量，每列只存一份，不按手牌重复：

```proto
ActionType action_type = 1;
uint32 amount_centi_bb = 2;
uint32 action_size_x10000 = 3;
```

只有以下两个数值字段按固定 `hand_idx` 展开：

```proto
repeated uint32 frequency_x10000 = 4 [packed = true];
repeated sint32 ev_x10000 = 5 [packed = true];
```

例如源数据：

```text
raise: action_size=40, amount_bb=2
call:  action_size=0,  amount_bb=0
fold:  action_size=0,  amount_bb=0
```

转换后：

| action | action_type | action_size_x10000 | amount_centi_bb |
| --- | --- | ---: | ---: |
| raise | `ACTION_TYPE_RAISE` | 400000 | 200 |
| call | `ACTION_TYPE_CALL` | 0 | 0 |
| fold | `ACTION_TYPE_FOLD` | 0 | 0 |

`action_size` 和 `amount_bb` 都直接保留源数据的数值语义，不根据 `action_type` 相互推导。

当前 PFSP 内部 action type 数字与 Protobuf 枚举值不同。导出时必须按 `action_name` 显式映射，禁止直接复用内部 `u8` 数值。

## 4. 固定手牌索引

第一版使用 `range-store-core` 现有 13x13、row-major 的 169 手牌字典：

```text
AA  = 0
AKs = 1
AKo = 13
22  = 168
```

每个 `frequency_x10000` 和 `ev_x10000` 数组长度严格等于 169。即使某个 action 不存在于某个手牌上，该位置仍保留占位值，是否存在由 bitmap 决定。

## 5. Bitmap 规则

所有 bitmap 使用低位优先：

```text
byte_index = hand_idx / 8
bit_index  = hand_idx % 8
mask       = 1 << bit_index
```

固定长度：

```text
HAND_ENCODING_169        -> ceil(169 / 8)  = 22 bytes
HAND_ENCODING_1326_COMBO -> ceil(1326 / 8) = 166 bytes
```

最后一个字节中超出手牌数量的 padding bits 必须为 0。

三个 bitmap 的职责不能混用：

- `LineMatrix.invalid_hand_bitmap`：矩阵全局非法手牌，主要用于翻后公共牌阻断；bit=1 表示整个手牌无效。
- `ActionColumn.action_hand_bitmap`：当前 action 是否存在于该手牌；bit=1 表示存在。
- `ActionColumn.ev_null_bitmap`：当前 action 存在时，EV 是否为 NULL；bit=1 表示 NULL。

读取优先级：

| invalid | action_hand | ev_null | 结果 |
| ---: | ---: | ---: | --- |
| 1 | 任意 | 任意 | 手牌全局非法，跳过所有 action |
| 0 | 0 | 任意 | 当前手牌不存在该 action |
| 0 | 1 | 1 | action 存在，frequency 有效，EV 为 NULL |
| 0 | 1 | 0 | action 存在，frequency 和 EV 都有效 |

缺失 action 的规范占位：

```text
action_hand_bitmap[hand_idx] = 0
frequency_x10000[hand_idx]   = 0
ev_x10000[hand_idx]          = 0
ev_null_bitmap[hand_idx]     = 0
```

EV 为 NULL 时：

```text
action_hand_bitmap[hand_idx] = 1
ev_null_bitmap[hand_idx]     = 1
ev_x10000[hand_idx]          = 0  // 占位，读取时忽略
```

真实 EV 等于 0 时，`ev_x10000` 同样为 0，但 `ev_null_bitmap` 必须为 0，因此不会与 NULL 混淆。

## 6. 数值转换

源 SQLite 到 Protobuf 的转换规则：

```text
hole_cards  -> get_hand_id(hole_cards)
action_name -> 显式映射到 ActionType
amount_bb   -> round(amount_bb * 100)
action_size -> round(action_size * 10000)
frequency   -> round(frequency * 10000)
hand_ev     -> round(hand_ev * 10000)
```

约束：

- `action_size`、`amount_bb` 必须有限且非负，转换后不能超过 `uint32`。
- `frequency` 必须在 `[0, 1]` 内。
- 非 NULL 的 `hand_ev` 必须有限，转换后不能超过 `sint32`。
- 对同一 `(hand_idx, action identity)`，源数据只能有一行。

ActionColumn 使用量化后的 action identity 进行去重并按以下顺序稳定排列：

```text
action_type -> action_size_x10000 -> amount_centi_bb
```

## 7. 查询方式

按 ID 查询：

```powershell
cargo run -p poker-hands-storage-tools -- export-line-matrix `
  --source-db data\sqlite\range.db `
  --dimension default:6:100 `
  --concrete-line-id 1 `
  --gto-data-version poc-001 `
  --out-dir reports\line-matrix-poc
```

按行动线文本查询：

```powershell
cargo run -p poker-hands-storage-tools -- export-line-matrix `
  --source-db data\sqlite\range.db `
  --dimension default:6:100 `
  --concrete-line F-F-F `
  --gto-data-version poc-001 `
  --out-dir reports\line-matrix-poc
```

`--concrete-line-id` 和 `--concrete-line` 必须二选一。如果同一个 `concrete_line` 匹配多行，命令返回歧义错误，此时增加：

```powershell
--abstract-line F-F-F
```

或者直接使用 `--concrete-line-id`。

默认不覆盖已有结果；明确传入 `--overwrite` 才会替换同名文件。

## 8. 输出与行动线元数据

假设解析得到 `concrete_line_id=1`：

```text
<out-dir>/
├── line-1.pb
├── line-1.debug.json
└── line-1.verify.json
```

- `line-1.pb`：最终 `LineMatrix` Protobuf payload。
- `line-1.debug.json`：可读的字段、数组和 bitmap 十六进制镜像。
- `line-1.verify.json`：源行数、action cell 数、NULL EV 数、频率误差、Protobuf 大小和校验结果。

源 SQLite 中可能存在 action frequency 合计不等于 1 的手牌。导出器会忠实保留这些源值，不会因此拒绝生成 Protobuf；verify JSON 会通过 `frequencySumMismatchHandCount`、`maxFrequencyErrorX10000`、`checks.sourceFrequencySumsWithinRoundingTolerance` 和 `warnings` 记录该源数据诊断。

为了保持 payload 紧凑，`concrete_line_id`、`abstract_line` 和 `concrete_line` 不重复放入 `LineMatrix`。它们记录在文件名、debug JSON 和 verify JSON 中。`gto_data_version` 写入 payload，用来标识源 GTO 数据版本；`schema_version` 单独标识 Protobuf 结构版本。

## 9. 验证条件

导出前和 Protobuf decode roundtrip 后都会执行结构校验：

- `schema_version == 1`。
- 第一版必须为 `HAND_ENCODING_169`。
- action identity 唯一且 action type 有效。
- frequency/EV 数组长度为 169。
- bitmap 长度为 22 bytes，padding bits 为 0。
- 无效手牌不能包含 action。
- action 不存在时，frequency、EV、EV NULL bit 都必须为 0。
- 允许一个合法手牌在当前行动线上完全没有 action 数据。
- 对至少存在一个 action 的手牌，统计所有存在 action 的量化频率之和与 10000 的偏差。

频率逐项四舍五入会引入最多每个 action 一个单位的累计误差，因此以下范围视为正常：

```text
abs(sum_frequency_x10000 - 10000) <= 当前手牌的 action 数量
```

超出该范围表示源数据频率未归一化，作为诊断信息写入 verify JSON，不属于 Protobuf 结构错误。

## 10. 实现位置

```text
storage-tools/
├── build.rs
├── proto/zenithstrat/gto/v1/matrix.proto
└── src/line_matrix_export/
    ├── mod.rs
    ├── cli.rs
    ├── source.rs
    ├── convert.rs
    ├── report.rs
    └── proto.rs
```

模块对外的主要 interface：

```rust
pub fn export_line_matrix(
    options: &ExportLineMatrixOptions,
) -> Result<ExportLineMatrixSummary, ToolError>
```

CLI 只负责解析参数和打印结果；SQLite 查询、action 聚合、量化、bitmap、验证、Protobuf 编码和报告生成都封装在 `line_matrix_export` 内部。

## 11. default:6:100 全量归档

当前可将 `default:6:100` 的所有连续 `concrete_line_id` 打包为一个不可变归档：

```powershell
cargo run -p poker-hands-storage-tools -- export-line-matrix-archive `
  --source-db data\sqlite\range.db `
  --out-dir reports\line-matrix-default-6max-100BB `
  --gto-data-version poc-random-001
```

归档目录包含：

```text
manifest.json
lines.db
matrices.lmbin
matrices.lmidx
```

`matrices.lmbin` 顺序保存 raw `LineMatrix` Protobuf payload；`matrices.lmidx` 的 header 后每条记录固定 16 bytes：

```text
u64 offset
u32 byte_length
u32 crc32c
```

## CompactLineMatrix V2 archive

V2 does not replace the V1 `LineMatrix` archive. It uses a separate command and
reader so an existing V1 archive cannot be decoded using the compact layout:

```powershell
cargo run -p poker-hands-storage-tools -- export-compact-line-matrix-archive `
  --source-db data\sqlite\range.db `
  --out-dir reports\line-matrix-compact-default-6max-100BB
```

The V2 payload is `zenithstrat.gto.v2.CompactLineMatrix`, with
`schema_version=2`. Its archive uses manifest version `2` and `LMCN` / `LMCX`
binary file magic. The index record layout is unchanged.

Rows where `hand_ev IS NULL` are omitted. Their source `frequency` must be
zero; otherwise export fails with `NULL_EV_WITH_NONZERO_FREQUENCY`.

`valid_hand_bitmap` is a 22-byte, LSB-first bitmap in original 169-hand id
space. Its set-bit rank maps `hand_id` to `global_compact_index`.
`action_hand_bitmap` is an LSB-first bitmap in that compact space, with length
`ceil(popcount(valid_hand_bitmap) / 8)`. For each action:

```text
frequency_x10000.len == ev_x10000.len == popcount(action_hand_bitmap)
```

Values are ordered by action bitmap set bits. The reader caches:

```text
original hand_id -> global_compact_index -> action_compact_index
```

The maps contain at most 1326 entries and provide O(1) hand/action lookup.

第 `n` 条 index record 对应 `concrete_line_id = n + 1`。`offset` 相对 `matrices.lmbin` 文件开头，`crc32c` 覆盖该条 raw Protobuf payload。`lines.db` 只保存行动线元数据：`concrete_line_id`、`abstract_line` 和 `concrete_line`。
