# RangeDB 二进制数据存储方案设计

更新日期：2026-07-04

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

| 项目                             |        字节数 |    约 MB | 占源 SQLite |
| -------------------------------- | ------------: | -------: | ----------: |
| 源 SQLite `data/sqlite/range.db` | 1,517,748,224 | 1,447.44 |     100.00% |
| Range Strata 输出总计            |   362,296,945 |   345.51 |      23.87% |
| `.bin` 合计                      |   272,110,768 |   259.51 |      17.93% |
| `.idx` 合计                      |    11,465,092 |    10.93 |       0.76% |
| `meta.db`                        |    78,716,928 |    75.07 |       5.19% |
| `manifest.json`                  |         4,157 |    0.004 |       0.00% |

与源 SQLite 相比，当前二进制运行目录约为源库的 23.87%，体积减少约 76.13%。

在 Range Strata 输出目录内部：

| 组成             | 占输出目录 |
| ---------------- | ---------: |
| `.bin` 策略数据  |     75.11% |
| `.idx` 索引数据  |      3.16% |
| `meta.db` 元数据 |     21.73% |
| `manifest.json`  |   约 0.00% |

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

`meta.db` 是瘦身后的 SQLite 元数据文件，不保存 range 明细。所有 range 明细数据存储在 `.bin` 文件中。当前包含以下表：

#### `build_info`

构建溯源表，key-value 结构：

| 字段    | 类型               | 说明                                             |
| ------- | ------------------ | ------------------------------------------------ |
| `key`   | `TEXT PRIMARY KEY` | 键名，当前固定为 `source_checksum` 和 `built_at` |
| `value` | `TEXT NOT NULL`    | 键值                                             |

写入两条记录：

| key               | 值                                  | 来源                 |
| ----------------- | ----------------------------------- | -------------------- |
| `source_checksum` | 源 SQLite 文件的 SHA-256 hex 字符串 | 构建时对源库文件计算 |
| `built_at`        | ISO 8601 时间戳                     | 构建完成时刻         |

用途：验证运行时数据是否与预期源数据匹配，防止数据版本混淆。

#### `action_schemas`

存储 action 组合定义（action schema），每个 schema 描述一个 concrete line 中所有可能的 action 集合。同一个 schema 可在多个维度或多个 concrete line 间复用。

| 字段           | 类型                                | 说明                                                          |
| -------------- | ----------------------------------- | ------------------------------------------------------------- |
| `id`           | `INTEGER PRIMARY KEY AUTOINCREMENT` | 自增主键，被 `.idx` 记录的 `action_schema_id` 引用            |
| `action_count` | `INTEGER NOT NULL`                  | 此 schema 中 action 的数量，范围 1..=32                       |
| `action_blob`  | `BLOB NOT NULL`                     | action 定义的序列化二进制数据，长度为 `action_count * 9` 字节 |
| `checksum`     | `INTEGER NOT NULL`                  | `action_blob` 的 CRC32C 校验和（转换为 i64）                  |
| `schema_key`   | `TEXT NOT NULL UNIQUE`              | `action_blob` 的 hex 编码字符串，用于构建去重                 |

这里的 `checksum` 只校验 `action_blob`，也就是 action schema 自身的动作定义字节，不校验 `.bin` 中的具体策略 payload。它的职责是保护“这个 `action_schema_id` 对应的动作语义是否完整”。

**`action_blob` 编码**：每个 action 固定 9 字节，按 `action_id`（0..N）顺序排列：

| 偏移 | 字段          | 类型     | 字节 | 说明                                             |
| ---: | ------------- | -------- | ---: | ------------------------------------------------ |
|    0 | `action_type` | `u8`     |    1 | 0=fold, 1=call, 2=check, 3=bet, 4=raise, 5=allin |
|    1 | `action_size` | `f32 LE` |    4 | 原始 action_size 值（来自源 SQLite）             |
|    5 | `amount_bb`   | `f32 LE` |    4 | 以 BB 为单位的大盲注金额                         |

单个 action 在 `action_blob` 中的字节布局：

```
+---------+-----------+-----------+
| type(1) | size(4)   | amount(4) |
+---------+-----------+-----------+
  offset 0        offset 1        offset 5
```

N 个 action 的 `action_blob` 总长度 = `N * 9` 字节。

**去重机制**：构建时通过 `schema_key`（`action_blob` 的 hex 编码）判断是否已存在相同 schema。若存在则复用已有 `id`，否则插入新记录。这确保相同 action 组合的维度共享同一个 schema 记录。

`schema_key` 是构建期字段，不进入 `.idx`。构建器先用 `schema_key` 查找或插入 `action_schemas`，拿到稳定的 `id` 后，`.idx` record 只保存 `action_schema_id`。运行时不需要再次通过 `schema_key` 去重，只需要用这个 `id` 读取对应的 `action_blob` 并解释 action 语义。

**运行时读取**：服务启动时通过 `SELECT id, action_count, action_blob FROM action_schemas ORDER BY id` 加载全部 schema 到内存 HashMap，key 为 `id`，value 为解码后的 `Vec<ActionDef>`。

#### `dimension_action_schemas`

维度与 action schema 的多对多关联表。一个维度（strategy + player_count + depth_bb）可能包含多个不同的 action schema（不同 concrete line 使用不同 action 组合）。

| 字段               | 类型               | 说明                         |
| ------------------ | ------------------ | ---------------------------- |
| `strategy`         | `TEXT NOT NULL`    | 策略名称，如 `default`       |
| `player_count`     | `INTEGER NOT NULL` | 玩家数量，如 6、8、9         |
| `depth_bb`         | `INTEGER NOT NULL` | 深度（BB），如 100、200、300 |
| `action_schema_id` | `INTEGER NOT NULL` | 引用 `action_schemas.id`     |

主键：`(strategy, player_count, depth_bb, action_schema_id)` 复合主键。

运行时通过 `SELECT action_schema_id FROM dimension_action_schemas WHERE strategy=? AND player_count=? AND depth_bb=? ORDER BY action_schema_id` 加载维度可用的全部 schema ID 列表。

#### `drill_scenario_lines_{strategy}`

Drill scenario（训练场景）到 abstract line 的映射表。每个 strategy 一张表，表名格式 `drill_scenario_lines_{strategy}`。

| 字段            | 类型                                | 说明                             |
| --------------- | ----------------------------------- | -------------------------------- |
| `id`            | `INTEGER PRIMARY KEY AUTOINCREMENT` | 自增主键（内部使用，对外不暴露） |
| `drill_name`    | `TEXT NOT NULL`                     | 场景名称，如 `UTG`、`RFI`        |
| `abstract_line` | `TEXT NOT NULL`                     | 抽象行动线，如 `F-F-F`、`R-C`    |
| `player_count`  | `INTEGER NOT NULL`                  | 玩家数量                         |
| `drill_depth`   | `INTEGER NOT NULL DEFAULT 100`      | 深度（BB）                       |

唯一约束：`UNIQUE(drill_name, player_count, drill_depth, abstract_line)`。

源 SQLite 中该表列名为 `depth`，复制到 meta.db 时重命名为 `drill_depth`。运行时查询使用 `SELECT abstract_line FROM drill_scenario_lines_{strategy} WHERE drill_name=? AND player_count=? AND drill_depth=? ORDER BY abstract_line`。

#### `concrete_lines_{strategy}_{N}max_{BB}BB`

每个维度一张表，存储 abstract line 到 concrete line 的映射关系，同时支持 concrete line 字符串的精确定位。表名格式 `concrete_lines_{strategy}_{player_count}max_{depth_bb}BB`。

| 字段               | 类型                  | 说明                                                                 |
| ------------------ | --------------------- | -------------------------------------------------------------------- |
| `concrete_line_id` | `INTEGER PRIMARY KEY` | concrete line id，与 `.idx` 记录和 pack 一一映射，从 1 开始连续递增  |
| `abstract_line`    | `TEXT NOT NULL`       | 抽象行动线，如 `F-F-F`（代表翻前弃牌、翻牌弃牌、转牌弃牌）           |
| `concrete_line`    | `TEXT NOT NULL`       | 具体行动线，如 `F-F-F`、`R2.5-C`（代表翻前加注 2.5BB、翻牌过牌跟注） |

唯一约束：`UNIQUE(abstract_line, concrete_line)`。
索引：`idx_{table_name}_concrete_line ON {table}(concrete_line)`，加速按 concrete_line 字符串的反查。

源 SQLite 中该表主键列为 `id`，复制到 meta.db 时重命名为 `concrete_line_id`。

**与 `.idx/.bin` 的关系**：`concrete_line_id` 是连接元数据和策略数据的桥梁。每个 `concrete_line_id` 值恰好对应 `.idx` 中的一条记录和 `.bin` 中的一个 pack。运行时通过 concrete_line 字符串查询 `concrete_line_id`，再通过 `.idx` 定位 pack。

**查询模式**：

```sql
-- 按 abstract_line 查找（返回多条）
SELECT concrete_line_id, abstract_line, concrete_line
FROM concrete_lines_default_6max_100BB
WHERE abstract_line = 'F-F-F'
ORDER BY concrete_line_id;

-- 按 concrete_line 精确定位（返回一条）
SELECT concrete_line_id, abstract_line, concrete_line
FROM concrete_lines_default_6max_100BB
WHERE concrete_line = 'R2.5-C'
ORDER BY concrete_line_id;

-- 按 (abstract, concrete) 组合精确定位
SELECT concrete_line_id, abstract_line, concrete_line
FROM concrete_lines_default_6max_100BB
WHERE abstract_line = 'R-C' AND concrete_line = 'R2.5-C'
ORDER BY concrete_line_id;
```

### `.idx` 文件

每个维度一个 `.idx` 文件，文件名：

```text
ranges_{strategy}_{player_count}max_{depth_bb}BB.idx
```

`.idx` 是定长记录索引，用于从 `concrete_line_id` 定位 `.bin` 中的 pack。

Header 固定 16 字节：

| 偏移 | 字段         | 类型     | 说明                              |
| ---: | ------------ | -------- | --------------------------------- |
|    0 | magic        | 4 bytes  | `PFXI`                            |
|    4 | version      | `u16 LE` | 当前为 `1`                        |
|    8 | record_count | `u32 LE` | 记录数（等于 concrete line 总数） |
|   12 | header_size  | `u16 LE` | 当前为 `16`                       |

Record 固定 22 字节，按 `concrete_line_id` 升序排列。当前正式数据要求同一维度内 `.idx` 的 `concrete_line_id` 连续递增（从 1 开始），每个 `concrete_line_id` 都对应一个 pack：

| 偏移 | 字段               | 类型     | 说明                                                                                       |
| ---: | ------------------ | -------- | ------------------------------------------------------------------------------------------ |
|    0 | `concrete_line_id` | `u32 LE` | concrete line id，从 1 开始连续递增(concrete_line_id甚至都可以不存入，直接 下标 + 1 == concrete_line_id)                                                        |
|    4 | `action_schema_id` | `u32 LE` | 引用 `meta.db.action_schemas.id`，决定 pack 中 action 的语义                               |
|    8 | `hand_count`       | `u16 LE` | pack 中包含的手牌数量，范围 1..=169                                                        |
|   10 | `offset`           | `u32 LE` | pack payload 在 `.bin` 文件中的起始字节偏移（相对于 `.bin` 文件开头，跳过 16 字节 header） |
|   14 | `byte_length`      | `u32 LE` | pack payload 的字节长度                                                                    |
|   18 | `checksum`         | `u32 LE` | pack payload 的 CRC32C 校验和                                                              |

这里的 `checksum` 校验对象是 `.bin` 中该 record 指向的整段 pack payload，也就是 `hand_ids + action_masks + cells`。它不校验 `action_schemas.action_blob`，而是保护这个 `concrete_line_id` 下的具体策略数据块是否损坏。

运行时只走 dense 下标读取：

```text
index = concrete_line_id - first_concrete_line_id
record_offset = header_size + index * 22
```

由于 `concrete_line_id` 连续递增（first_concrete_line_id = 1），`index = concrete_line_id - 1`，记录偏移可直接计算为 `16 + (concrete_line_id - 1) * 22`。这意味着 O(1) 随机访问，无需二分查找。

`IdxReader::open()` 会校验 `.idx` 中 `concrete_line_id` 必须连续递增；不满足 dense 布局的 `.idx` 会被视为格式错误。读取 record 后仍会校验 `record.concrete_line_id == concrete_line_id`，但不会退回二分查找。standalone verify 也会检查正式 `.idx` 的 `concrete_line_id` 连续性，构建或发布前应先通过验证。

`.idx` 不保存 `schema_key` 或 `action_blob`，只保存 `action_schema_id`，这是当前格式的刻意边界：

| 不放入 `.idx` 的字段   | 原因                                                                              |
| ---------------------- | --------------------------------------------------------------------------------- |
| `schema_key`           | 只用于构建期去重；运行时已可通过 `action_schema_id` 直接定位 schema               |
| `action_blob`          | 是变长 metadata，长度为 `action_count * 9`；放入 `.idx` 会破坏 22 字节定长 record |
| action schema 全量定义 | 同一个 schema 会被大量 concrete line 复用；放入每条 `.idx` record 会重复存储      |

因此 `.idx` 的职责保持为 hot path 定位信息：`concrete_line_id -> action_schema_id + offset + byte_length + checksum`。`action_schemas` 则放在 `meta.db` 中，负责保存可变长、可去重、可跨维度复用的动作定义。

### 两类 checksum 的区别与联系

当前格式里有两类 CRC32C：

| 位置                              | 校验对象                 | 保护内容                                                         | 与谁关联                                              |
| --------------------------------- | ------------------------ | ---------------------------------------------------------------- | ----------------------------------------------------- |
| `meta.db.action_schemas.checksum` | `action_blob`            | action schema 的动作定义是否损坏                                 | 通过 `.idx.action_schema_id` 被引用                   |
| `.idx` record `checksum`          | `.bin` 中的 pack payload | 该 `concrete_line_id` 的手牌、mask、frequency、EV 数据块是否损坏 | 通过同一条 `.idx` record 的 `offset/byte_length` 定位 |

可以把两者理解成两层完整性校验：

- `action_schemas.checksum` 保护“怎么解释数据”。
- `.idx.checksum` 保护“数据本身”。

两者通过 `action_schema_id` 联系起来：

1. `.idx` record 先定位 `.bin` 中的 pack payload。
2. 同一条 `.idx` record 再通过 `action_schema_id` 指向 `meta.db.action_schemas`。
3. pack payload 给出手牌策略数值，action schema 给出这些数值对应的 action 语义。

因此这两类 checksum 不是重复设计，而是分别覆盖“动作定义层”和“具体策略 payload 层”。

### `.bin` 文件

每个维度一个 `.bin` 文件，文件名：

```text
ranges_{strategy}_{player_count}max_{depth_bb}BB.bin
```

`.bin` 由 16 字节 PFSP header 加连续 pack payload 组成。

Header：

| 偏移 | 字段        | 类型     | 说明                                 |
| ---: | ----------- | -------- | ------------------------------------ |
|    0 | magic       | 4 bytes  | `PFSP`（Poker Hands Storage Format） |
|    4 | version     | `u16 LE` | 当前为 `1`                           |
|    6 | endian      | `u8`     | `1` 表示 little-endian               |
|    7 | float_type  | `u8`     | `1` 表示 Float32                     |
|    8 | layout      | `u8`     | `1` 表示 sparse hand-major v1        |
|    9 | compression | `u8`     | `0` 表示无压缩                       |
|   10 | header_size | `u16 LE` | 当前为 `16`                          |

`.bin` 不保存每个 pack 的业务 key（`concrete_line_id`、`hand_count` 等），pack 位置完全由 `.idx` 记录定位。这种分离设计使得 `.bin` 成为纯连续字节流，可被 `mmap` 高效映射，且构建时可顺序写入，无需随机定位。

## Range Pack 编码

一个 pack 对应一个 `concrete_line_id` 下的一组手牌策略数据。pack 是 `.bin` 文件中不可分割的最小数据单元。

pack 的二进制布局分为三个连续段：

```
+------------------+---------------------+------------------------------------------+
| hand_ids 段       | action_masks 段      | cells 段                                  |
| hand_count bytes | hand_count * 4 bytes | hand_count * action_count * 8 bytes       |
+------------------+---------------------+------------------------------------------+
```

### 第一段：hand_ids

```
hand_ids[hand_count]  // u8 数组，升序排列，取值 0..168
```

- 长度：`hand_count` 字节（每手牌 1 字节）
- 排序：严格升序，便于二分查找
- 取值：`hand_id` 是 169 种起手牌的数值编码（AA=0, KK=14, QQ=162, ... 72o=168）。具体映射见 [`data-flow-overview.md#四-hand_id-说明`](./data-flow-overview.md#四hand_id-说明)
- 不一定包含全部 169 手牌：某些 action line 可能只覆盖部分起手牌（如纯弃牌线只覆盖少数 hand）

### 第二段：action_masks

```
action_masks[hand_count]  // u32 LE 数组，每手牌 4 字节
```

- 长度：`hand_count * 4` 字节
- 每个 u32 是一个位图，bit N 置 1 表示该手牌在 action schema 中的第 N 个 action 有有效数据
- 对于 169 手牌的完整 pack，此段占用 676 字节

**action_mask 的设计意图**：GTO 策略中，很多 (hand, action) 组合是不合法的。例如 AA 作为翻前加注者不可能 fold。如果没有 mask，pack 中每手牌都要为所有 action 存储 frequency/EV 数据（即使该 action 不存在），造成 `169 * action_count * 8` 字节的浪费。mask 使得 pack 可以存储所有 action 的 cell 数据（保持定长布局以支持 mmap 随机访问），同时通过位图标记哪些 cell 是有效的。

**hands-by-actions 查询中的二次利用**：在 `POST /range/hands-by-actions` 的 hot path 中，action_mask 的思想被进一步复用——解码 pack 后在内存中构建 `hand_masks[169]` 数组（每手牌一个 u32），然后通过 `hand_mask & filter_mask != 0` 的位运算一次性判断多 hand 是否匹配目标 action，完全避免逐 cell 分支判断。

### 第三段：cells

```
cells[hand_count][action_count]  // 每 cell 8 字节：frequency f32 LE + hand_ev f32 LE
```

- 长度：`hand_count * action_count * 8` 字节
- 存储顺序：先按 hand_id 顺序遍历，每手牌内按 action_id (0..N) 顺序存储
- 每 cell 包含两个 f32：

| 字段        | 类型     | 字节 | 说明                                                            |
| ----------- | -------- | ---: | --------------------------------------------------------------- |
| `frequency` | `f32 LE` |    4 | 策略频率，范围 0.0..1.0                                         |
| `hand_ev`   | `f32 LE` |    4 | 该手牌在该 action 下的期望值；`NaN`（0x7FC00000）表示 null/无值 |

### pack 长度公式

```
byte_length = hand_count * (5 + action_count * 8)
```

其中 `5 = 1 (hand_id) + 4 (action_mask)` 是每手牌的固定头部开销，`action_count * 8` 是每手牌的 cell 数据量（每 action 8 字节）。

示例：一个包含全部 169 手牌、32 个 action 的 pack：

```
byte_length = 169 * (5 + 32 * 8) = 169 * 261 = 44,109 字节 <= 44KB
```

### 字段详细说明

| 字段           | 编码方式   | 约束                         | 备注                                                                                   |
| -------------- | ---------- | ---------------------------- | -------------------------------------------------------------------------------------- |
| `frequency`    | Float32 LE | 0.0 <= f <= 1.0              | 与源 SQLite REAL 的精度转换通过 Float32 bit-exact 验证保证                             |
| `hand_ev`      | Float32 LE | 可为 NaN                     | NaN 表示 null；非 NaN 值通过 bit-exact 验证与源库对齐                                  |
| `hand_id`      | u8         | 0..168                       | 169 种起手牌的组合数学编码                                                             |
| `action_mask`  | u32 LE     | bit N = 1 表示 action N 存在 | 运行时用于存在性标记和 hands-by-actions 位运算过滤                                     |
| `action_count` | 派生值     | 1..=32                       | 由 `.idx.action_schema_id` 查 `action_schemas.action_count` 获得，不直接存储在 pack 中 |

## 查询流程

`POST /range/hand-strategy` 的核心读取流程详见 [`data-flow-overview.md`](./data-flow-overview.md#23-单次查询-query)。

简要概括：

```text
1. 根据 strategy/player_count/depth_bb 定位维度
2. HandlePool 获取 DimensionReader
3. .idx dense 下标定位 concrete_line_id
4. .bin 按 offset/length 读取 pack
5. 在 pack 的 hand_ids 中查找目标 hand_id（详见 data-flow-overview.md 第五节）
6. 解码目标 hand 的 action cells
7. meta.db action_schemas 解释 action_id
8. 返回 API 业务结构
```

`POST /range/hands-by-actions` 会完整解码一个 pack，按 action 和 frequency 过滤手牌。其 hot path 使用 `hand_masks[169]` 位图进行 O(1) 位运算过滤，避免逐 cell 分支判断。

业务侧如果只有具体行动线字符串，应先通过 `meta.db` 的 `concrete_lines_*` 表精确查询 `concrete_line_id`，再进入上述 `.idx/.bin` 查询流程。

## 当前维度文件大小

| 维度                 | concrete lines | `.bin` bytes | `.idx` bytes |  合计 bytes | `.idx` 占维度 |
| -------------------- | -------------: | -----------: | -----------: | ----------: | ------------: |
| `default:6max:100BB` |          3,737 |    2,172,204 |       82,230 |   2,254,434 |         3.65% |
| `default:6max:200BB` |          2,363 |    1,666,509 |       52,002 |   1,718,511 |         3.03% |
| `default:6max:300BB` |          1,816 |    1,390,341 |       39,968 |   1,430,309 |         2.79% |
| `default:8max:100BB` |          8,892 |    4,635,494 |      195,640 |   4,831,134 |         4.05% |
| `default:8max:200BB` |          5,454 |    3,438,513 |      120,004 |   3,558,517 |         3.37% |
| `default:8max:300BB` |          3,643 |    2,865,913 |       80,162 |   2,946,075 |         2.72% |
| `default:9max:100BB` |        197,087 |   83,756,612 |    4,335,930 |  88,092,542 |         4.92% |
| `default:9max:200BB` |        203,028 |  108,969,070 |    4,466,632 | 113,435,702 |         3.94% |
| `default:9max:300BB` |         95,114 |   63,216,112 |    2,092,524 |  65,308,636 |         3.20% |

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
2. 初始化输出目录和 `meta.db`（创建所有表结构）。
3. 写入 `build_info` 记录（source_checksum、built_at）。
4. 复制 drill scenario 和 concrete line 元数据到 meta.db。
5. 按 `concrete_line_id` 聚合源 `range_data_*` 行（`SELECT ... ORDER BY concrete_line_id, hole_cards, action_name`）。
6. 对每个 concrete line 构建 `ActionKey` 集合（action_type + action_size + amount_bb），生成或复用 action schema。
7. 将 concrete line 的行编码为 pack（hand_ids + action_masks + cells）。
8. 写 `.bin.tmp` 和 `.idx.tmp`。
9. 维度构建成功后 rename 成正式 `.bin/.idx`。
10. 写 `manifest.json`。

### 编码细节

每个 concrete line 的 pack 编码过程：

1. 读取该 concrete_line_id 下的所有源行（按 hole_cards, action_name 排序）。
2. 提取唯一的 hand_id 集合（BTreeSet 自动排序）和唯一的 ActionKey 集合（action_type + action_size + amount_bb）。
3. ActionKey 按 (action_type, action_size, amount_bb) 排序后分配 action_id (0..N)。
4. 对每手牌构建 action_mask：对该手牌存在的每个 ActionKey，设置对应 bit。
5. 对每手牌每个 action_id，写入 (frequency as f32, hand_ev.map(|v| v as f32).unwrap_or(NaN))。
6. 将 action 列表编码为 action_blob（每 action 9 字节），计算 CRC32C 和 schema_key，写入或复用 action_schemas 记录。
7. 将 payload（hand_ids + action_masks + cells）追加到 `.bin.tmp`，记录写入 `.idx.tmp`。

## 运行时约束

- 运行数据目录应视为不可变目录。
- 不应在服务持有 mmap handle 时原地覆盖 `.idx/.bin`。
- 发布新数据应使用新目录，验证通过后切换挂载或重启服务。
- `PHS_MAX_OPEN_HANDLES` 控制同时打开的维度 handle 数量。
- mmap 不等于立即把整个 `.bin` 文件读入物理内存，实际 RSS 会随访问页增长。
