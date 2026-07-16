# Proto V3 业务存储实施方案

状态：当前最高优先级，待实施  
更新日期：2026-07-16

## 目标

将每个维度目录升级为面向业务查询的 Proto V3 存储：

```text
drill_name
  -> abstract_action_path[]

abstract_action_path
  -> (concrete_action_path_id, concrete_action_path)[]

concrete_action_path_id + hole_cards
  -> action[]
```

V3 必须保留 `hand_ev IS NULL` 的 source cell。该 cell 使用已选定的 A 方案编码：
`frequency_x10000 = 20000`，`hand_ev_x10000 = 0`。

## 存储边界与目录

一个目录只代表一个维度，维度由 `manifest.json` 声明；不在每个 Proto record 中重复写入
`dimension_id`，不增加 `LineMatrixRecord` 或根 `catalog.pb`。

```text
default_9max_200BB/
├── manifest.json
├── drill-scenarios.pb
├── drill-scenarios.idx
├── abstract-action-paths.pb
├── abstract-action-paths.idx
├── hand-strategies.pb
└── hand-strategies.idx
```

- `.pb` 是连续追加的原始 Protobuf payload。
- `.idx` 是 mmap 的固定宽度二进制索引；它不是业务事实来源，只负责低延迟定位 Proto page/payload。
- `manifest.json` 记录 V3 format、维度、文件名、大小、校验信息和 record/page count。
- `concrete_action_path_id` 只在当前维度内有效，并且从 1 连续编号。

## Proto schema

```proto
syntax = "proto3";

package poker.hands.storage.v3;

enum ActionType {
  ACTION_TYPE_UNSPECIFIED = 0;
  ACTION_TYPE_FOLD = 1;
  ACTION_TYPE_CHECK = 2;
  ACTION_TYPE_CALL = 3;
  ACTION_TYPE_BET = 4;
  ACTION_TYPE_RAISE = 5;
  ACTION_TYPE_ALLIN = 6;
}

enum HandEncoding {
  HAND_ENCODING_UNSPECIFIED = 0;
  HAND_ENCODING_PREFLOP = 1;
  HAND_ENCODING_POSTFLOP = 2;
}

message DrillScenarioPage {
  repeated DrillScenarioEntry entries = 1;
}

message DrillScenarioEntry {
  string drill_name = 1;
  repeated string abstract_action_paths = 2;
}

message AbstractActionPathPage {
  repeated AbstractActionPathEntry entries = 1;
}

message AbstractActionPathEntry {
  string abstract_action_path = 1;
  repeated ConcreteActionPathRef concrete_action_paths = 2;
}

message ConcreteActionPathRef {
  // 1..=hand-strategies.idx record count in this dimension.
  uint32 concrete_action_path_id = 1;
  string concrete_action_path = 2;
}

message HandStrategy {
  uint32 schema_version = 1;
  HandEncoding hand_encoding = 2;
  repeated ActionStrategyColumn actions = 3;
  bytes available_hand_bitmap = 100;
}

message ActionStrategyColumn {
  ActionType action_type = 1;
  uint32 amount_centi_bb = 2;
  uint32 action_size_x10000 = 3;

  // 0..=10000: round(frequency * 10000).
  // 20000: hand_ev is NULL; hand_ev_x10000 must then be 0.
  repeated uint32 frequency_x10000 = 4 [packed = true];

  repeated sint32 hand_ev_x10000 = 5 [packed = true];

  // Bitmap in global compact hand-index space.
  bytes action_hand_bitmap = 6;
}
```

`HAND_ENCODING_PREFLOP` 和 `HAND_ENCODING_POSTFLOP` 必须使用不同的 enum 值；若两者都为
`1`，reader 无法区分阶段。V3 首期仅导出/读取 `HAND_ENCODING_PREFLOP`；`POSTFLOP = 2`
为 schema 预留值，在定义 postflop 手牌编码前不得写入该值。

## 三个业务数据集

### Drill scenario

导出时严格按当前维度过滤源表：

```sql
SELECT drill_name, abstract_line
FROM drill_scenario_lines_{strategy}
WHERE player_count = :player_count
  AND depth = :depth_bb
ORDER BY drill_name, abstract_line;
```

`drill-scenarios.idx`：

```text
drill_name hash -> page_id + entry_index
```

### Abstract/concrete action path

`abstract-action-paths.pb` 只存一次路径关系：

```text
abstract_action_path -> (concrete_action_path_id, concrete_action_path)[]
```

`abstract-action-paths.idx` 包含两个索引区：

```text
abstract_action_path hash -> page_id + entry_index
concrete_action_path hash -> page_id + entry_index + value_index
```

concrete 索引直接指向 Proto page 内已有字符串，避免重复写一份 `concrete -> id` 数据页。
hash 命中后必须进行完整字符串比较以处理 hash collision。

### Hand strategy

`hand-strategies.pb` 直接存 `HandStrategy` payload；`hand-strategies.idx` 的第 N 条记录就是
`concrete_action_path_id = N` 的 offset、byte length 和 CRC32C。

查询 `concrete_action_path + hole_cards -> action[]` 时，先由 concrete 索引取得 ID，再直接读取
策略 payload。`hole_cards` 使用既有 169 hand 编码转换为 `hand_id`，不在 payload 中重复保存字符串。

## 不变量与 API 语义

- 每个 `concrete_action_path` 在一个维度内唯一；重复时导出报
  `DUPLICATE_CONCRETE_ACTION_PATH`。
- `concrete_action_path_id` 必须从 1 连续到 `hand-strategies.idx` record count。
- 每个 `ConcreteActionPathRef` 都必须对应一条 `HandStrategy`。
- 每个 drill 导出的 abstract path 都必须能在 abstract 索引中解析。
- `available_hand_bitmap` 必须等于全部 `action_hand_bitmap` 覆盖手牌的并集。
- 每列 `frequency_x10000.len()` 与 `hand_ev_x10000.len()` 必须等于对应 action bitmap 的置位数。
- frequency 只允许 `0..=10000` 或 null-EV sentinel `20000`；`10001..19999` 和大于
  `20000` 的值为格式错误。
- sentinel cell 解码为 API `frequency = 0`、`hand_ev = null`；`hand_ev_x10000` 必须是 0。
- source 中若出现 `hand_ev IS NULL AND frequency != 0`，导出失败，不能静默覆盖 frequency。

现有对外 metadata API 可以保留：concrete 索引定位到的 abstract page 已包含所属
`abstract_action_path`，因此在需要兼容 `ConcreteLineRow` 时仍可构造完整结果；业务快路径只返回
`concrete_action_path_id`。

## 实施任务

### 1. 建立 V3 schema 和归档格式

- **结果**：Prost 生成 V3 类型，V3 manifest 能识别三个业务数据集。
- **影响组件**：`storage-tools/proto/`、`storage-tools/build.rs`、
  `storage-tools/src/proto_range_storage/proto.rs`、`format.rs`。
- **行动**：新增 V3 `.proto`，定义新的 data/index magic、header、manifest 字段与文件名；保留
  V2 format 只读兼容路径。
- **验证**：`cargo test -p poker-hands-storage-tools --test compact_line_matrix_archive` 通过；增加
  schema round-trip 和非法 enum/array length 测试。

### 2. 导出 drill scenario 和 action path metadata

- **结果**：每个维度只生成属于自身 `strategy/player_count/depth_bb` 的 drill 数据，且 abstract/
  concrete 查询不再打开 SQLite。
- **影响组件**：`sqlite_source.rs`、现有 exporter、metadata reader、`query_facade.rs`。
- **行动**：实现两个分页 Proto writer、三个索引区和带 CRC 的 page reader；导出时校验 concrete
  path 唯一和 ID 连续。
- **验证**：对每个维度，逐项比较带维度条件的 SQLite drill 查询、abstract 查询、concrete 查询与
  V3 结果；损坏 page、错误 hash locator、重复 concrete path 均应失败。

### 3. 导出和读取完整 hand strategy

- **结果**：所有 source range cell 都进入 V3，包括 null EV cell。
- **影响组件**：`line_matrix_codec.rs`、`line_matrix_store.rs`、`query_service.rs`、
  `three_way_*benchmark.rs`、verification 模块。
- **行动**：source SQL 移除 `hand_ev IS NOT NULL`；实现 20000 sentinel 编码、解码和验证；将当前
  matrix 术语替换为 `HandStrategy` 业务术语。
- **验证**：覆盖非零 EV、零 EV、null EV + 零 frequency、null EV + 非零 frequency（导出失败）；
  V3 action 返回值与 SQLite 逐 cell 一致。

### 4. 替换运行时 metadata cache

- **结果**：metadata 查询只访问 mmap index 和按需 Proto page，应用堆内存有明确上限。
- **影响组件**：`query_facade.rs`、handle pool、cache stats、benchmark profile。
- **行动**：移除 `lines.db` connection 和无界 query-result HashMap；引入 facade 级、有 byte budget
  的 decoded metadata page cache，cache key 必须包含维度目录和 page id。
- **验证**：分别测量 drill、abstract、concrete 的首次访问、page-cache hit、handle 淘汰后重读；
  cache resident bytes 不得超过配置预算。

### 5. 完整性验证、迁移和切换

- **结果**：V3 可由源 SQLite 全量重建、验证、发布和回滚。
- **影响组件**：verify CLI、benchmark CLI、文档、V2/V3 reader dispatch。
- **行动**：增加三条业务映射、策略 payload、CRC、索引边界和 null EV 的全量校验；writer 默认输出
  V3 到新目录，reader 依据 manifest version 双读 V2/V3。
- **验证**：`cargo test --workspace`、`cargo fmt --all -- --check` 通过；九维全量导出后，所有映射和
  action 值与 SQLite 对比零差异。记录 metadata P50/P95、全局 cache bytes 和 RSS。

## Rollout 与回滚

1. 保留现有 V2 目录，只向新的 V3 输出目录导出。
2. 对九维运行全量验证和 cold/hot benchmark；确认三条业务查询链和策略值均一致。
3. 将服务/SDK 默认 archive root 切到 V3；V2 reader 在过渡期保留。
4. 如出现兼容性或性能问题，配置切回 V2 archive root；不修改、不删除 V2 产物。

V2 的 cache/decode 优化文档仍可作为历史和后续性能优化参考，但其中“过滤 null EV”与
“使用 lines.db”的假设不适用于 V3。
