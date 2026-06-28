# RangeDB 二进制数据存储方案设计

更新日期：2026-06-28

## 设计目标

RangeDB 二进制存储的目标是把源 SQLite 中体积最大的 range 明细转换成面向查询的不可变文件，同时保留必要元数据的可查询性。

运行时数据目录包含：

```text
data/range-strata/
  manifest.json
  meta.db
  ranges_default_6max_100BB.idx
  ranges_default_6max_100BB.bin
  ...
```

源 SQLite `data/sqlite/range.db` 不进入线上服务热路径，只用于构建、验证和 benchmark baseline。

## 当前体积快照

当前本地数据集快照：

| 项目 | 字节数 | 约 MB | 占源 SQLite |
| --- | ---: | ---: | ---: |
| 源 SQLite `data/sqlite/range.db` | 1,517,748,224 | 1,447.44 | 100.00% |
| Range Strata 输出总计 | 362,296,945 | 345.51 | 23.87% |
| `.bin` 合计 | 272,110,768 | 259.51 | 17.93% |
| `.idx` 合计 | 11,465,092 | 10.93 | 0.76% |
| `meta.db` | 78,716,928 | 75.07 | 5.19% |
| `manifest.json` | 4,157 | 0.004 | 0.00% |

与源 SQLite 相比，当前二进制运行目录约为源库的 23.87%，体积减少约 76.13%。

在 Range Strata 输出目录内部：

| 组成 | 占输出目录 |
| --- | ---: |
| `.bin` 策略数据 | 75.11% |
| `.idx` 索引数据 | 3.16% |
| `meta.db` 元数据 | 21.73% |
| `manifest.json` | 约 0.00% |

这些数字是当前数据集快照，不是格式承诺。数据版本、维度数量、action schema 分布和 SQLite vacuum 状态都会影响最终比例。

## 文件职责

### `manifest.json`

`manifest.json` 是运行目录入口，格式为 PFSP v1。它记录：

- `format`：当前为 `PFSP`。
- `version`：当前为 `1`。
- `sourceDbChecksum`：源 SQLite 文件的 SHA-256。
- `builtAt`：构建时间。
- `dimensions`：每个维度的构建结果、pack 数、文件名和文件大小。
- `files`：运行目录应包含的文件列表。

服务启动时先读取 manifest，筛出可查询维度，再根据维度打开对应 `.idx/.bin` 文件。

### `meta.db`

`meta.db` 是瘦身后的 SQLite 元数据目录，不保存 range 明细。当前包含：

| 表 | 作用 |
| --- | --- |
| `build_info` | 记录 `source_checksum` 和 `built_at` |
| `action_schemas` | 存储 action 组合定义，供 `.idx` 中的 `action_schema_id` 引用 |
| `dimension_action_schemas` | 记录每个维度引用了哪些 action schema |
| `drill_scenario_lines_{strategy}` | drill scenario 到 abstract line 的查询表 |
| `concrete_lines_{strategy}_{N}max_{BB}BB` | abstract line 到 concrete line 的查询表 |

`action_schemas.action_blob` 的编码是每个 action 9 字节：

| 字段 | 类型 | 字节 |
| --- | --- | ---: |
| `action_type` | `u8` | 1 |
| `action_size` | `f32 LE` | 4 |
| `amount_bb` | `f32 LE` | 4 |

`action_type` 映射：

| 值 | action |
| ---: | --- |
| 0 | `fold` |
| 1 | `call` |
| 2 | `check` |
| 3 | `bet` |
| 4 | `raise` |
| 5 | `allin` |

### `.idx` 文件

每个维度一个 `.idx` 文件，文件名：

```text
ranges_{strategy}_{player_count}max_{depth_bb}BB.idx
```

`.idx` 是定长记录索引，用于从 `concrete_line_id` 定位 `.bin` 中的 pack。

Header 固定 16 字节：

| 偏移 | 字段 | 类型 | 说明 |
| ---: | --- | --- | --- |
| 0 | magic | 4 bytes | `PFXI` |
| 4 | version | `u16 LE` | 当前为 `1` |
| 8 | record_count | `u32 LE` | 记录数 |
| 12 | header_size | `u16 LE` | 当前为 `16` |

Record 固定 22 字节，按 `concrete_line_id` 升序排列：

| 偏移 | 字段 | 类型 | 说明 |
| ---: | --- | --- | --- |
| 0 | `concrete_line_id` | `u32 LE` | concrete line id |
| 4 | `action_schema_id` | `u32 LE` | 引用 `meta.db.action_schemas.id` |
| 8 | `hand_count` | `u16 LE` | pack 中包含的 hand 数，最大 169 |
| 10 | `offset` | `u32 LE` | pack 在 `.bin` 中的起始偏移 |
| 14 | `byte_length` | `u32 LE` | pack 字节长度 |
| 18 | `checksum` | `u32 LE` | pack payload 的 CRC32C |

查询时对 `.idx` 做二分查找，复杂度为 `O(log n)`。

### `.bin` 文件

每个维度一个 `.bin` 文件，文件名：

```text
ranges_{strategy}_{player_count}max_{depth_bb}BB.bin
```

`.bin` 由 16 字节 PFSP header 加连续 pack payload 组成。

Header：

| 偏移 | 字段 | 类型 | 说明 |
| ---: | --- | --- | --- |
| 0 | magic | 4 bytes | `PFSP` |
| 4 | version | `u16 LE` | 当前为 `1` |
| 6 | endian | `u8` | `1` 表示 little-endian |
| 7 | float_type | `u8` | `1` 表示 Float32 |
| 8 | layout | `u8` | `1` 表示 sparse hand-major v1 |
| 9 | compression | `u8` | `0` 表示无压缩 |
| 10 | header_size | `u16 LE` | 当前为 `16` |

`.bin` 不保存每个 pack 的业务 key，pack 位置完全由 `.idx` 记录定位。

## Range Pack 编码

一个 pack 对应一个 `concrete_line_id` 下的一组手牌策略。

pack 结构：

```text
hand_ids[hand_count]             // u8，升序，0..168
action_masks[hand_count]         // u32 LE，每个 bit 表示 action 是否存在
cells[hand_count][action_count]  // frequency f32 LE + hand_ev f32 LE
```

pack 长度公式：

```text
byte_length = hand_count * (5 + action_count * 8)
```

其中：

- `hand_id` 使用固定 169 手牌字典。
- `frequency` 使用 Float32 little-endian。
- `hand_ev` 使用 Float32 little-endian。
- `hand_ev = null` 编码为 NaN，解码时 NaN 转回 `null`。
- `action_count` 来自 `.idx.action_schema_id` 对应的 `action_schemas.action_count`。
- `action_mask` 决定某手牌哪些 action 实际存在。

## 查询流程

`POST /range/hand-strategy` 的核心读取流程：

```text
1. 根据 strategy/player_count/depth_bb 找到维度。
2. 通过 HandlePool 获取或打开该维度的 DimensionReader。
3. `.idx` 二分查找 concrete_line_id。
4. 从 `.idx` record 得到 action_schema_id、hand_count、offset、byte_length、checksum。
5. 从 `.bin` 读取 offset..offset+byte_length 的 pack。
6. 如果启用 checksum，计算 CRC32C 并和 `.idx.checksum` 比对。
7. 在 pack 的 hand_ids 中二分查找目标 hand_id。
8. 读取该 hand 的 action_mask 和 cells。
9. 用 `meta.db.action_schemas` 把 action_id 解释成 action_name、action_size、amount_bb。
10. 返回 API 业务结构。
```

`POST /range/hands-by-actions` 会完整解码一个 pack，按 action 和 frequency 过滤手牌。

## 当前维度文件大小

| 维度 | concrete lines | `.bin` bytes | `.idx` bytes | 合计 bytes | `.idx` 占维度 |
| --- | ---: | ---: | ---: | ---: | ---: |
| `default:6max:100BB` | 3,737 | 2,172,204 | 82,230 | 2,254,434 | 3.65% |
| `default:6max:200BB` | 2,363 | 1,666,509 | 52,002 | 1,718,511 | 3.03% |
| `default:6max:300BB` | 1,816 | 1,390,341 | 39,968 | 1,430,309 | 2.79% |
| `default:8max:100BB` | 8,892 | 4,635,494 | 195,640 | 4,831,134 | 4.05% |
| `default:8max:200BB` | 5,454 | 3,438,513 | 120,004 | 3,558,517 | 3.37% |
| `default:8max:300BB` | 3,643 | 2,865,913 | 80,162 | 2,946,075 | 2.72% |
| `default:9max:100BB` | 197,087 | 83,756,612 | 4,335,930 | 88,092,542 | 4.92% |
| `default:9max:200BB` | 203,028 | 108,969,070 | 4,466,632 | 113,435,702 | 3.94% |
| `default:9max:300BB` | 95,114 | 63,216,112 | 2,092,524 | 65,308,636 | 3.20% |

`.idx` 体积很小，因为每个 concrete line 只有一条 22 字节记录。主要体积仍在 `.bin` 的手牌/action 矩阵 payload。

## 构建流程

构建入口：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- build `
  --source-db data\sqlite\range.db `
  --out-dir data\range-strata `
  --overwrite
```

构建器执行：

1. 从源 SQLite 发现 `range_data_*` 和对应 `concrete_lines_*` 维度。
2. 初始化输出目录和 `meta.db`。
3. 复制 drill scenario 和 concrete line 元数据。
4. 按 `concrete_line_id` 聚合源 range rows。
5. 生成或复用 action schema。
6. 写 `.bin.tmp` 和 `.idx.tmp`。
7. 维度构建成功后 rename 成正式 `.bin/.idx`。
8. 写 `manifest.json`。

## 运行时约束

- 运行数据目录应视为不可变目录。
- 不应在服务持有 mmap handle 时原地覆盖 `.idx/.bin`。
- 发布新数据应使用新目录，验证通过后切换挂载或重启服务。
- `PHS_MAX_OPEN_HANDLES` 控制同时打开的维度 handle 数量。
- mmap 不等于立即把整个 `.bin` 文件读入物理内存，实际 RSS 会随访问页增长。

