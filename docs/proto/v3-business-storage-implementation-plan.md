# Proto V3 业务存储实施方案

状态：代码实施完成；真实九维数据发布门禁待在完整源库环境执行
更新日期：2026-07-16

## 实施定位

V3 是项目首个计划上线的 Proto 业务存储格式，不是从已上线 V2 向 V3 的兼容迁移。

- 源 SQLite 是导出和正确性验证的唯一事实来源。
- V2 仅作为 mmap、固定宽度索引、Protobuf 编解码、LRU cache 和 benchmark 实现参考。
- V3 reader 不读取 V2，不增加 V2/V3 dispatch，不做 V2/V3 结果或性能对比。
- V3 不依赖 `lines.db`，运行时不得打开源 SQLite 或任何派生 SQLite metadata 文件。
- V3 首发只覆盖 preflop 169 hand 编码；postflop 不属于本次实施范围。
- V2 代码在 V3 实施期间保留，避免边开发边清理；V3 验收后是否删除另开任务，不阻塞首发。

## 目标

为每个维度构建面向业务查询的 Proto V3 存储：

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

### 已固化的二进制契约

所有整数使用 little-endian。六个文件使用独立 magic：

| 文件 | magic |
| --- | --- |
| `drill-scenarios.pb` | `V3DD` |
| `drill-scenarios.idx` | `V3DI` |
| `abstract-action-paths.pb` | `V3AD` |
| `abstract-action-paths.idx` | `V3AI` |
| `hand-strategies.pb` | `V3HD` |
| `hand-strategies.idx` | `V3HI` |

每个文件以 32-byte header 开始：

```text
magic[4]
format_version: u16 = 3
header_size: u16 = 32
primary_count: u64
secondary_count: u64
section_count: u32
flags: u32 = 0
```

`.idx` 的 section directory 紧跟 header，每条 32 bytes：

```text
section_kind: u16
record_size: u16
reserved: u32 = 0
offset: u64
record_count: u64
byte_length: u64
```

page/payload locator 固定为 16 bytes：`offset: u64 + byte_length: u32 + crc32c: u32`。hash locator
固定为 24 bytes：`hash: u64 + page_id: u32 + entry_index: u32 + value_index: u32 + reserved: u32`。
`page_id`、`entry_index` 和 `value_index` 均为 0-based；不使用 value index 时写 `u32::MAX`。

字符串索引固定使用 FNV-1a 64-bit，不能替换为进程随机 hash。hash locator 按 hash 排序；相同 hash 的
记录必须连续保存并逐项做完整字符串比较。locator 中的 CRC32C 覆盖单个 Proto payload；manifest
另存六个完整 `.pb/.idx` 文件各自的 CRC32C。

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
  // Fixed to 3 for this schema.
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
- 每条 `HandStrategy.schema_version` 必须固定为 `3`。
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

实施按以下顺序推进。每个里程碑必须独立通过测试后再开始下一阶段，避免 schema、索引、业务查询和
性能改造同时变化。

### 0. 固化格式契约和测试骨架

**结果**：所有会影响磁盘兼容性的决策先写入代码常量、manifest 类型和 golden tests。

**行动**：

1. 在 `storage-tools/proto/poker/hands/storage/v3/` 新增独立 V3 schema；V2 生成类型保持原样，
   V3 不能复用 V2 package 或类型别名。
2. 在 `storage-tools/src/proto_range_storage/v3/` 建立独立模块，至少拆分为 `format`、`proto`、
   `source`、`metadata_store`、`hand_strategy_store`、`exporter`、`reader`、`query_service` 和
   `verification`。不要继续扩大现有大型 V2 `line_matrix_store.rs`。
3. 固化六个文件的 magic、format version、header、index record/section layout、字节序、hash 算法、
   page locator、CRC32C 覆盖范围和对齐规则。hash 必须跨进程和 Rust 版本稳定，不能使用
   `DefaultHasher`。
4. 固化 ID 规则：按 source concrete table 的 `id` 升序导出，V3
   `concrete_action_path_id` 由导出顺序重新编号为 `1..=N`；所有 metadata ref 和 strategy record
   只使用该 V3 ID，不能隐式假设它等于 SQLite `id`。
5. manifest 记录维度、schema/format version、文件名、文件大小、page/record count、六个 `.pb/.idx`
   文件各自的 CRC32C，以及生成状态。manifest 最后原子发布；缺失 manifest 的目录视为未完成产物。
6. 新建 V3 专属集成测试文件，V2 测试不作为 V3 验收项，也不要求为 V3 修改 V2 测试期望。

**阶段门槛**：schema round-trip、manifest round-trip、header/index golden bytes、非法 version/magic、
截断 header/record、越界 locator、CRC 错误和 hash collision fixture 全部通过。

### 1. 实现 metadata 导出与读取

**结果**：不生成 `lines.db`，三条 metadata 映射完全由 Proto page 和 mmap index 提供。

**行动**：

1. `source` 按当前维度加载 drill 和 action path。drill SQL 必须同时带
   `strategy/player_count/depth_bb` 约束；禁止把同 strategy 的其他维度 drill 数据复制进来。
2. 一次扫描 concrete table，按 source `id` 排序后建立：
   `source id -> V3 concrete_action_path_id`、abstract 分组和 concrete 唯一性集合。
3. 重复 concrete path 报 `DUPLICATE_CONCRETE_ACTION_PATH`；drill 引用不存在的 abstract path、
   concrete path 无法分配 V3 ID、空数据集和 ID 不连续均立即终止导出。
4. 实现 `DrillScenarioPage` 和 `AbstractActionPathPage` 的分页 writer/readers。分页以编码后字节数为
   上限，单个 entry 大于目标 page size 时允许独占 page，不能截断业务 entry。
5. 建立 page directory 以及 drill、abstract、concrete 三个 hash index 区；相同 hash 的 locator 必须
   连续保存，命中后逐个读取 page 并比较完整字符串，不能把碰撞项覆盖掉。
6. 先提供与当前 facade 对齐的 API：`get_drill_scenario_lines` 和三个
   `ConcreteLineFilter` 分支；业务快路径额外暴露 `concrete_action_path -> id`。

**阶段门槛**：fixture 中逐项对比 SQLite 与 V3 的 drill、abstract、concrete 查询；覆盖空结果、重复
path、跨维度 drill 污染、碰撞后二次字符串比较、page/index 损坏和 V3 ID 重编号。

### 2. 实现完整 HandStrategy 导出与读取

**结果**：一个 V3 concrete ID 对应一个完整 `HandStrategy`，包括所有 null EV source cell。

**行动**：

1. 将 source loader 改为读取全部 range rows，移除 `hand_ev IS NOT NULL`；这里新建 V3 loader，
   不修改 V2 行为来模拟兼容。
2. 复用 V2 的 action normalization、数值量化、169 hand bitmap 和 compact index 思路，类型和错误
   文案统一改为 `HandStrategy` 业务术语。
3. 对 `hand_ev IS NULL AND frequency = 0` 写 sentinel `20000/0`；对
   `hand_ev IS NULL AND frequency != 0` 报错。非 null frequency 仍只允许 `0..=10000`。
4. `available_hand_bitmap` 按所有 source cell 计算，必须等于 action bitmap 并集；相同
   `(hand_id, action identity)` 重复时报 `DUPLICATE_ACTION_CELL`。
5. `hand-strategies.idx` 第 N 条记录直接定位 V3 ID=N 的 payload；reader 校验 ID 范围、offset、length、
   CRC、enum、bitmap、array length 和 sentinel 组合。
6. 导出完成前校验每个 `ConcreteActionPathRef` 都有对应 `HandStrategy`，且不存在 metadata 未引用的
   strategy record。
7. 查询解码保持现有 `QueryResult/ActionResult` 业务契约；sentinel 返回
   `frequency = 0, hand_ev = None`，普通 cell 返回量化后的数值。

**阶段门槛**：覆盖正/负/零 EV、普通零频率、null EV + 零频率、null EV + 非零频率失败、量化边界、
非法 sentinel、重复 cell、缺失 hand 和批量查询；fixture 与 SQLite 逐 cell 零差异。

### 3. 接入 facade、handle pool 和有界 cache

**结果**：V3 运行时只持有 mmap、文件句柄和有明确预算的 decoded cache。

**行动**：

1. dimension handle 打开该目录的三组 data/index 文件并验证 manifest；同一 handle 内共享 mmap，
   不为每次查询重复打开文件。
2. 保留现有按维度淘汰的 handle pool 思路，但 service 类型改为 V3 reader，不增加 format dispatch。
3. strategy cache 使用 V3 ID 作为 handle 内 key；metadata page cache key 至少包含数据集、page id，
   facade 聚合统计时还要包含维度。
4. 两类 cache 都必须有 byte budget；记录 hit/miss、entries、resident/peak bytes、evictions、
   oversized skips 和 disabled skips。任何时刻 resident estimated bytes 不得超过预算。
5. 删除 V3 路径中的 SQLite connection、`MetadataCache.connection` 和按 query key 无界增长的
   `HashMap`。metadata 查询结果可以短暂构造，但不能无界常驻。
6. 保持 facade 的 dimension-not-found、concrete-not-found、hand-strategy-not-found 和 batch error
   语义；V3 首发不要求读取旧 V2 目录。

**阶段门槛**：首次访问、cache hit、超预算不缓存、LRU 淘汰、handle 淘汰后重开、并发读取、损坏文件
和错误维度测试通过；cache resident bytes 的断言覆盖 metadata 与 strategy 两类 cache。

### 4. 建立 SQLite -> V3 全量验证

**结果**：一个命令可以验证单维或全部维度，SQLite 是唯一 comparison baseline。

**行动**：

1. standalone 验证 manifest、文件大小/CRC、header、index section、page/record count、locator 边界、
   concrete ID 连续性、所有 ref 可达性、bitmap/array/sentinel 不变量。
2. cross 验证重新查询源 SQLite，比较：
   - `drill_name -> abstract_action_path[]`；
   - `abstract_action_path -> (V3 id, concrete_action_path)[]`；
   - `concrete_action_path -> V3 id`；
   - 每个 concrete path、169 手牌和所有 action cell。
3. 比较 action cell 时按 action identity 对齐，不依赖 SQLite row 顺序；数值按 V3 量化规则比较，
   null EV 必须精确比较为 null。
4. 报告至少包含每维 source/export counts、映射差异、cell 差异、null EV cell 数、损坏项、失败样例
   和总耗时；任何差异返回非零退出码。
5. 全量 exporter 在发布 manifest 前执行 read-back standalone verify；九维 cross verify 作为首发门禁。

**阶段门槛**：故意制造每类映射差异、数值差异、null 差异、悬空 ref、CRC 损坏时验证器都能失败；
真实九维数据验证结果为零差异。不运行 V2/V3 comparison。

### 5. 建立 V3 性能基线并接管默认入口

**结果**：V3 达到业务可用性和资源门槛后，成为工具、服务和 SDK 的唯一 Proto 默认格式。

**行动**：

1. 将现有 three-way benchmark 改造或新建为 SQLite/V3 两方 benchmark，不保留 V2 case；workload
   必须覆盖 drill、abstract、concrete、单手策略、batch 和 hands-by-actions。
2. 分别测 cold open、首次 metadata page、metadata hit、首次 strategy decode、strategy hit、handle
   淘汰重开和稳定运行；输出 P50/P95/P99、吞吐、各阶段耗时、cache bytes 和进程 RSS。
3. benchmark 的正确性校验仍指向 SQLite，性能模式可以关闭逐请求 CRC，但发布验证必须开启 CRC。
4. 提供清晰的 V3 CLI：单维导出、全部维度导出、standalone verify、SQLite cross verify、benchmark。
5. 更新 README、API 说明、部署输入和运维文档，明确线上只需要 V3 目录，不需要 SQLite 或 V2 产物。
6. 最后再把默认 archive root 和公开示例切到 V3；在此之前 V3 通过显式 CLI/配置运行。

**阶段门槛**：`cargo fmt --all -- --check`、`cargo test --workspace` 以及九维 export + standalone +
cross verify 全部通过；benchmark 报告包含 metadata P50/P95、strategy P50/P95、全局 cache bytes 和
RSS，且没有 correctness failure。workspace test 只用于确认仓库未发生回归，不代表 V2 兼容性验证；
V3 的格式和业务验收只看 V3 专属测试及 SQLite/V3 cross verify。

## 建议提交边界

为便于 review 和回退实现错误，按以下边界提交，不把多个里程碑压进一个大提交：

1. V3 proto、format、manifest 和 golden tests。
2. metadata source/export/page/index/reader。
3. HandStrategy source/codec/store/reader。
4. V3 query service、facade、handle pool 和 cache stats。
5. standalone/cross verification 与故障 fixture。
6. SQLite/V3 benchmark、CLI、默认入口和文档。

每个提交只依赖前一提交已经公开并测试的接口；若磁盘格式在第 1 个提交后需要改变，必须同步更新
golden tests 和本方案，不能静默改变 magic、header 或 locator 语义。

## 首发与问题处理

1. 从源 SQLite 向全新的 V3 root 导出，禁止覆盖源库或把临时文件当成完成产物。
2. 对九维依次运行 read-back standalone verify 和 SQLite cross verify。
3. 运行 cold/hot benchmark，确认正确性、延迟、cache 上限和 RSS 均符合门槛。
4. 验收后将服务/SDK 默认 archive root 指向 V3，V3 作为首个上线 Proto 格式。
5. 若发现格式或数据问题，停止发布、修复 writer/reader 后从 SQLite 重新全量导出 V3；不回退到 V2，
   也不为此增加 V2 reader。

## Definition of Done

- 九个维度均能由源 SQLite 一次性全量导出，目录中只有 manifest 和三组 `.pb/.idx` 业务文件。
- standalone verify 对所有文件、索引、ref、payload 和业务不变量通过。
- SQLite cross verify 的三条业务映射和所有 action cell 零差异，包括全部 null EV cell。
- 运行时查询链不打开 SQLite，metadata/strategy cache 均受 byte budget 限制。
- 首次访问、cache hit、handle 淘汰和稳定运行性能均有可复现报告。
- workspace test、format check 和 V3 CLI 端到端测试通过。
- README、API、部署和运维文档只把 V3 描述为计划上线的 Proto 格式；V2 明确标记为实现参考。

V2 的 cache/decode 优化文档仍可作为历史和后续性能优化参考，但其中“过滤 null EV”、
“使用 lines.db”、V2/V3 双读和 V2 回滚的假设均不适用于 V3。
