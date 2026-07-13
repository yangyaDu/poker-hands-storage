# SDK 与查询链路详解

更新日期：2026-07-08

## 文档职责

- Bun / TypeScript native SDK 的公开 API、构建测试方式和生产接入边界。
- 一次 SDK 查询如何经过 SDK wrapper、N-API 绑定、`range-store-core` 查询核心，最终从 `.idx/.bin` 和 `meta.db` 返回结果。
- SDK 与 HTTP service 的契约差异。

## 模块定位

`range-store-native` 是 Bun / TypeScript 进程内只读 SDK，Node.js 运行时兼容：

- Rust 侧通过
api-rs` 暴露 `PokerHandsRange`。
- TypeScript 业务代码导入运行时入口 `index.js`；`index.d.ts` 只提供类型声明，由 TypeScript 自动解析。
- SDK wrapper 通过 `index.js` 加载 `index.node`，把 native 异常转换成 `RangeStoreError`。
- 查询语义复用 `range-store-core::query::RangeStoreFacade`，与 HTTP service 共用 core 业务路径。

它不负责：

- 从源 SQLite 构建 Range Strata Binary。
- source cross verify。
- benchmark 报告生成。
- HTTP 服务部署。

## 公开入口

包入口：

```typescript
import {
  PokerHandsRange,
  RangeStore,
  RangeStoreError,
  getPokerHandsRangeSingleton,
} from "./index.js"
```

在业务 `.ts` 文件中仍导入 `index.js`，不要把 `index.d.ts` 当运行时模块导入。`RangeStoreError` 是运行时 class，`instanceof RangeStoreError` 依赖这个真实导入。

构造参数：

```ts
interface PokerHandsRangeOptions {
  dataDir: string
  maxOpenHandles?: number
  verifyChecksums?: boolean
}
```

示例：

```typescript
const store = new PokerHandsRange({
  dataDir: "./data/range-strata",
  maxOpenHandles: 2,
  verifyChecksums: false,
})
```

`RangeStore` 是 `PokerHandsRange` 的别名。`getPokerHandsRangeSingleton(options)` 会在同一组选项下复用单例；重复初始化时选项不同会抛出普通 `Error`。

## SDK 返回契约

当前 SDK 成功时返回直接 payload，不返回 HTTP service 的 `{ code, data, message }` envelope。

| 方法                         | 返回                                                    | 说明                                                               |
| ---------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------ |
| `getConcreteLines(request)`  | `{ lines }`                                             | 按 `abstractLine` 列 concrete lines，或按 `concreteLine` 精确查 id |
| `getAbstractLines(request)`  | `{ abstractLines }`                                     | 查询 drill 场景下的 abstract lines                                 |
| `handsByActions(request)`    | `{ holeCards }`                                         | 按 concrete line id、actions、frequency 过滤手牌                   |
| `queryHandStrategy(request)` | `{ actions }`                                           | 查询单手牌策略                                                     |
| `queryBatch(request)`        | `{ results: [{ concreteLineId, holeCards, actions }] }` | 批量查询单手牌策略，当前为 all-or-nothing                          |
| `prewarm(request)`           | `{ openHandleCount }`                                   | 打开指定维度的 `.idx/.bin` reader                                  |
| `stats()`                    | `{ schemaCount, openHandleCount, knownDimensions }`     | 查询 SDK 内部缓存和 handle 状态                                    |

单手牌策略查询：

```typescript
const result = store.queryHandStrategy({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLineId: 1,
  holeCards: "AA",
})

// result:
// {
//   actions: [
//     {
//       actionName: "raise",
//       actionSize: 2.5,
//       amountBb: 2.5,
//       frequency: 0.75,
//       handEv: 1.0,
//     },
//   ],
// }
```

错误不会作为成功 payload 的 `code/message` 返回，而是抛出 `RangeStoreError`：

```typescript
try {
  store.queryHandStrategy({ ...request, holeCards: "AsXx" })
} catch (error) {
  if (error instanceof RangeStoreError) {
    console.log(error.code) // INVALID_ARGUMENT 等
    console.log(error.message)
  }
}
```

## 构建和测试

Windows 本地构建：

```powershell
Set-Location range-store-native
bun install
bun run build:native
bun run test:sdk
```

Linux x64 产物构建脚本：

```powershell
bun run build:native:linux
```

HTTP consistency 测试需要先启动 HTTP service，并设置 `PHS_HTTP_URL`：

```powershell
$env:PHS_HTTP_URL = "http://127.0.0.1:8080"
bun run test:http-consistency
```

`range-store-native/tests/sdk-contract.test.js` 是 SDK wrapper 的直接契约测试入口。benchmark 输出不能替代 SDK contract 测试。

## 与 HTTP service 的关系

两者是平级运行入口，都复用 `range-store-core`：

| 入口           | 使用场景                   | 返回契约                               | 边界成本                             |
| -------------- | -------------------------- | -------------------------------------- | ------------------------------------ |
| HTTP service   | 跨进程、跨语言、容器化服务 | `{ code, data, message }` envelope     | HTTP/JSON 序列化和 loopback/网络成本 |
| Bun native SDK | Bun / TypeScript 业务进程内查询 | 直接 payload，失败抛 `RangeStoreError` | N-API 边界和 SDK 包装成本            |

当前正式 benchmark 只保留 `core`、
ative-sdk`、`http-service` 三组对比。Native SDK 的策略查询最终仍落到 `RangeStoreFacade -> StoreQueryService`；如果某次报告显示 SDK 和 core 有明显速度差异，应优先从 page cache、运行时上下文、计时精度和样本局部性解释，不应假设 SDK 绕过了 core 算法。

## 生产接入边界

已完成：

- Windows MSVC 本地 `.node` 构建。
- SDK contract 测试。
- native SDK 与 HTTP service 的抽样一致性测试入口。
- `benchmark-native` fair runner，覆盖 core、native SDK 和 HTTP service。

待验证：

- Linux x64 `.node` 产物在业务容器中可加载。
- 只读 Range Strata 数据目录挂载后，constructor、prewarm、核心查询和 stats 均通过。
- 多副本读取同一只读数据目录。
- 业务 readiness 等待 native store 打开和必要 prewarm 完成。

## 查询链路总览

```text
业务 .ts 文件
  |
  | 从 range-store-native/index.js 导入运行时符号
  | TypeScript 类型由 index.d.ts 自动解析
  v
range-store-native/index.js
  |
  | SDK wrapper: 参数映射、直接 payload、RangeStoreError 归一化
  v
range-store-native/src/lib.rs
  |
  | napi-rs: camelCase 方法 <-> Rust snake_case 方法
  v
RangeStoreFacade
  |
  +-- CachedMetadataReader
  |     |
  |     +-- manifest.json 发现维度
  |     +-- meta.db 懒加载 concrete lines / drill lines，并写入内存 cache
  |
  +-- StoreQueryService
        |
        +-- ActionSchemaCache
        |     |
        |     +-- meta.db.action_schemas 按 action_schema_id 懒加载
        |
        +-- HandlePool
              |
              +-- DimensionReader
                    |
                    +-- IdxReader.find(concrete_line_id) -> .idx mmap dense lookup
                    +-- BinReader.read_pack(offset, byte_length) -> .bin mmap slice
                    +-- decode_pack_for_hand() / decode_pack()

HTTP service 是平级入口：

service route -> RangeStoreFacade -> HTTP { code, data, message } envelope
```

当前边界语义：

- Native SDK：成功返回直接 payload；失败抛 `RangeStoreError`。
- HTTP service：成功/失败都由 service 边界转换成 HTTP status + `{ code, data, message }`。
- Core：只返回领域结果或 typed error，不返回 HTTP envelope。

## 构造阶段

业务 `.ts` 代码调用
ew PokerHandsRange(options)` 后，`index.js` wrapper 会把 `dataDir/maxOpenHandles/verifyChecksums` 传给 native 类：

```typescript
new native.PokerHandsRange({
  dataDir: options.dataDir,
  maxOpenHandles: options.maxOpenHandles,
  verifyChecksums: options.verifyChecksums,
})
```

Rust N-API constructor 当前语义：

```rust
let max_open_handles = options.max_open_handles.unwrap_or(2).max(1) as usize;
let verify_checksums = options.verify_checksums.unwrap_or(false);
let inner = RangeStoreFacade::open(options.data_dir, max_open_handles, verify_checksums)?;
```

`RangeStoreFacade::open()` 会创建两个核心组件：

| 组件                   | 构造阶段做什么                                                                      | 构造阶段不做什么                               |
| ---------------------- | ----------------------------------------------------------------------------------- | ---------------------------------------------- |
| `CachedMetadataReader` | 读取 `manifest.json`、打开只读 `meta.db` 连接、记录已知维度                         | 不把 concrete line / drill line 全量加载进内存 |
| `StoreQueryService`    | 读取 `manifest.json`、校验 `meta.db` 存在、创建 `ActionSchemaCache` 和 `HandlePool` | 不 mmap `.idx/.bin`，不加载 `action_schemas`   |

因此构造后通常可以看到：

```typescript
store.stats() // { schemaCount: 0, openHandleCount: 0, knownDimensions: [...] }
```

`prewarm(request)` 只提前打开指定维度的 `.idx/.bin` reader，并放入 LRU handle pool；action schema 仍会在第一次策略查询命中具体 `action_schema_id` 时懒加载。

## 单手牌策略查询

### 1. SDK wrapper

`queryHandStrategy(request)` 位于 `range-store-native/index.js`：

```typescript
queryHandStrategy(request) {
  const result = callNative(() =>
    this.#native.queryHandStrategy({
      ...toNativeDimension(request),
      concreteLineId: request.concreteLineId,
      holeCards: request.holeCards,
    }),
  );
  return {
    actions: result.actions.map(fromNativeAction),
  };
}
```

`callNative()` 捕获 N-API 抛出的异常。如果 native 错误消息符合：

```text
RANGE_STORE_ERROR:{CODE}:{message}
```

SDK wrapper 会转换成：

```typescript
new RangeStoreError(code, message, { cause: error })
```

否则统一转换为 `RangeStoreError("INTERNAL", message)`。

### 2. N-API 绑定

`range-store-native/src/lib.rs` 把 SDK request 转成 core 的 `DimensionRef`：

```rust
let dimension = dimension_from_parts(request.strategy, request.player_count, request.depth_bb);
let result = self
    .inner
    .query_hand_strategy(&dimension, request.concrete_line_id, &request.hole_cards)?;
```

`strategy` 是可选字段，未传时默认为 `default`。N-API 返回的 Rust struct 再由 napi-rs 转成 JS runtime object，字段会以 camelCase 暴露；TypeScript 侧类型来自 `index.d.ts`。

### 3. RangeStoreFacade

`RangeStoreFacade::query_hand_strategy()` 是 HTTP service 和 native SDK 共用的业务入口：

```rust
self.query_service.query(dimension, concrete_line_id, hole_cards)
```

它的主要职责是统一 core 查询结果和 typed error。到边界层后再分流：

- Native SDK：native 层把 core error 编码成 `RANGE_STORE_ERROR:{CODE}:{message}`，`index.js` 再转换为 `RangeStoreError` 并抛出。
- HTTP service：service 层把同一类 core error 转成 HTTP status + `{ code, data, message }` envelope。

常见错误码：

| 错误码                    | 场景                                            |
| ------------------------- | ----------------------------------------------- |
| `INVALID_ARGUMENT`        | 手牌格式非法、action filter 非法等              |
| `DIMENSION_NOT_FOUND`     | `strategy/playerCount/depthBb` 不在 manifest 中 |
| `DATA_FILE_NOT_FOUND`     | 维度存在，但 `.idx/.bin` 打开失败               |
| `CONCRETE_LINE_NOT_FOUND` | `.idx` 中没有该 `concrete_line_id`              |
| `HAND_STRATEGY_NOT_FOUND` | concrete line 存在，但该 hand 不在 pack 中      |
| `ACTION_SCHEMA_NOT_FOUND` | `meta.db.action_schemas` 缺少引用的 schema      |
| `HANDS_NOT_FOUND`         | `handsByActions` 没有任何匹配手牌               |

### 4. StoreQueryService::query()

核心查询路径在 `range-store-core/src/query/store_query_service.rs`：

```rust
let parsed = parse_hole_cards(hole_cards)?;
let reader = self.pool.get_or_open(dimension)?;
let fragment = reader.query(concrete_line_id, parsed.hand_id, self.verify_checksums)?;
let action_schema = self.action_schemas.get(fragment.action_schema_id)?;
```

这里有四个关键动作：

1. `parse_hole_cards()` 把 `"AA"`、`"AKs"`、`"AKo"`、`"AsKh"` 等输入归一化为 169 手牌字典中的 `hand_id: u8`。
2. `HandlePool::get_or_open()` 获取或打开当前维度的 `DimensionReader`。
3. `DimensionReader::query()` 从 `.idx/.bin` 中定位并解码目标 hand 的 cells。
4. `ActionSchemaCache::get()` 用 `action_schema_id` 把 `action_id` 转成业务动作字段。

如果 `.idx` 找不到 `concrete_line_id`，返回 `CONCRETE_LINE_NOT_FOUND`。如果 concrete line 存在但目标 hand 不在 pack 中，返回 `HAND_STRATEGY_NOT_FOUND`。

## DimensionReader 热路径

### 1. HandlePool

`HandlePool` 是线程安全的 LRU 池：

```text
key = "{strategy}:{player_count}:{depth_bb}"
```

cache hit 时直接返回已有 `Arc<DimensionReader>` 并刷新 LRU 顺序。cache miss 时：

1. 检查维度是否在 manifest 的 queryable dimensions 中。
2. 拼出 `.idx` 和 `.bin` 文件路径。
3. 创建 `DimensionReader::open(idx_path, bin_path)`。
4. 插入池中，超过 `maxOpenHandles` 后淘汰最久未使用的 reader。

### 2. IdxReader

`.idx` 是 PFXI 文件，头部 16 字节，每条 record 18 字节：

```text
action_schema_id(4)
hand_count(2)
offset(4)
byte_length(4)
checksum(4)
```

`IdxReader::open()` 会 mmap 文件并校验 PFXI header。builder 与 standalone verify 保证 metadata id 为连续的 `1..N`；查找按固定偏移计算：

```rust
let index = concrete_line_id.checked_sub(1)?;
let offset = index as usize * IDX_RECORD_SIZE;
let record = decode_idx_record_at(records_base, offset);
```

这里没有 SQL 解析、B-tree 遍历或行格式解析。边界仍会被检查：id 为 0 或大于 record_count 时返回 `None`。

### 3. BinReader

`.bin` 是 PFSP 文件。`BinReader::open()` 会 mmap 文件并校验 header：

```text
Magic: PFSP
Version: 1
Endian: little-endian
Float type: float32
Layout: sparse hand-major v1
Compression: none
Header size: 16
```

读取 pack 时只根据 `.idx` record 中的 `offset/byte_length` 切出 mmap slice：

```rust
Ok(&self.mmap[start..end])
```

这一步不分配中间 buffer。首次访问相关页面时由 OS page cache 处理实际分页加载。

### 4. Pack 校验

`DimensionReader::read_and_validate_pack()` 会做运行时边界检查：

| 检查                                                 | 目的                                       |
| ---------------------------------------------------- | ------------------------------------------ |
| `hand_count > 0`                                     | 防止无效 idx record                        |
| `offset + byte_length` 不越界                        | 防止读取 `.bin` 外部                       |
| `byte_length == hand_count * (5 + action_count * 8)` | 确认 pack 长度能反推出合法 `action_count`  |
| `action_count <= 32`                                 | 因为 action mask 是 `u32`                  |
| 可选 CRC32C                                          | `verifyChecksums=true` 时校验 pack payload |

`action_count` 不来自 action schema，而是由 `.idx` 中的 `hand_count/byte_length` 推导：

```text
action_count = (byte_length / hand_count - 5) / 8
```

## Pack 解码

pack 内部是稀疏 hand-major 布局：

```text
hand_ids:      hand_count bytes
action_masks:  hand_count * 4 bytes
cell_data:     hand_count * action_count * 8 bytes
```

每个 cell 是：

```text
frequency f32 + hand_ev f32
```

`decode_pack_for_hand()` 的流程：

1. 在已排序的 `hand_ids` 段里二分查找目标 `hand_id`。
2. 读取目标 hand 对应的 `u32 action_mask`。
3. 定位目标 hand 的 cell row。
4. 遍历 `action_id = 0..action_count`。
5. 如果 mask 对应 bit 为 0，跳过该 action。
6. 如果 bit 为 1，读取 frequency 和 EV。
7. `hand_ev` 的 `NaN` 表示业务 null，返回到 Bun / TypeScript 调用方时是
ull`。

最多 169 个 hand，所以二分查找不超过 8 次比较。由于 pack 是稀疏的，不能直接用 `hand_ids[target_hand_id]` 当固定下标。

## ActionSchemaCache

pack 解码只得到 `action_id/frequency/hand_ev`，业务仍需要动作名和下注大小。这个映射来自 `meta.db.action_schemas`：

```rust
let action_schema = self.action_schemas.get(fragment.action_schema_id)?;
```

`ActionSchemaCache` 当前语义：

1. 先读 `RwLock<HashMap<u32, Arc<Vec<ActionDef>>>>`。
2. 命中时 clone `Arc` 返回。
3. 未命中时通过内部只读 SQLite 连接查询 `action_schemas`。
4. 解码 `action_blob` 后写入 cache。

因此 constructor 和 prewarm 都不会主动加载全部 schema；只有真实策略查询命中某个 `action_schema_id` 后，`schemaCount` 才会上升。

## 返回给 Bun / TypeScript

core 组装出的 action：

```rust
ActionResult {
    action_name,
    action_size,
    amount_bb,
    frequency,
    hand_ev,
}
```

N-API 层把 `f32` 转成 JS runtime
umber` / TypeScript
umber` 可表达的 `f64`：

```rust
ActionResult {
    action_name: action.action_name,
    action_size: f64::from(action.action_size),
    amount_bb: f64::from(action.amount_bb),
    frequency: action.frequency,
    hand_ev: action.hand_ev,
}
```

SDK wrapper 再转换成 camelCase：

```typescript
{
  actionName,
  actionSize,
  amountBb,
  frequency,
  handEv,
}
```

`queryHandStrategy()` 最终只返回：

```typescript
{
  actions
}
```

不会返回 `inputHoleCards`、`handCode`、`code`、`data` 或 `message`。

## queryBatch 当前语义

SDK 入参：

```typescript
store.queryBatch({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  items: [
    { concreteLineId: 1, holeCards: "AA" },
    { concreteLineId: 1, holeCards: "KK" },
  ],
})
```

返回：

```typescript
{
  results: [
    { concreteLineId: 1, holeCards: "AA", actions: [...] },
    { concreteLineId: 1, holeCards: "KK", actions: [...] },
  ],
}
```

当前 `range-store-core::StoreQueryService::query_batch()` 是严格 all-or-nothing，但成功路径会按 `concrete_line_id` 分组共享 pack 读取：

- 先解析全部 `holeCards`，记录最小失败 index，但不会立刻返回。
- 按 `concrete_line_id` 分组；同一 concrete line 的多个 hand 走一次 `DimensionReader::query_many_hands()`。
- 每个 concrete line group 只读一次 `.idx` record 和一次 `.bin` pack slice。
- 任一 item 失败时，整个 batch 抛出 `RangeStoreError`。
- 错误消息会包含 `Batch item requests[index] failed`。
- 成功 item 不带 `handCode`，失败 item 不会作为 `{ error }` 留在 results 里。

batch 的主要收益来自：一次 N-API 调用边界、一次 `HandlePool::get_or_open()`、同 concrete line 下共享 pack 读取和 schema 解析，以及 all-or-nothing 错误传播。

## handsByActions 当前语义

SDK 入参：

```typescript
store.handsByActions({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLineId: 1,
  actions: ["raise2.5", "call"],
  frequency: 0.005,
})
```

返回：

```typescript
{
  holeCards: ["AA", "AKs"]
}
```

链路：

1. SDK wrapper 把 `actions/frequency` 传给 N-API。
2. Rust 层把原始 action 字符串解析成 `ActionFilter`。
3. `StoreQueryService::query_hands_by_actions()` 打开维度 reader。
4. `DimensionReader::query_all()` 找到 pack，并完整解码该 concrete line 下的所有 hand/action cell。
5. `ActionSchemaCache` 把 action filter 转成 action bit mask。
6. `match_hands_by_actions()` 返回满足条件的 169-hand code。

关键业务语义：

| 字段                   | 当前语义                                                       |
| ---------------------- | -------------------------------------------------------------- |
| `actions` 缺省或空数组 | 不限制 action name，但仍要求至少一个存在的 action 超过频率阈值 |
| 多个 actions           | OR 语义，任意一个 filter 命中即可返回 hand                     |
| `fold/check/call`      | 不允许数值后缀                                                 |
| `bet/raise/allin`      | 可以带数值后缀，例如 `raise2.5`，按 `amountBb` 精确匹配        |
| `frequency` 缺省       | 使用 `frequency > 0.005`                                       |
| `frequency: x`         | 使用严格大于：`frequency > x`                                  |
| 无匹配手牌             | `RangeStoreFacade` 转成 `HANDS_NOT_FOUND` 错误                 |

注意：`handsByActions` 必须完整扫描一个 pack，因为它回答的是“哪些手牌满足 action/frequency 条件”，不是查询某一个 hand 的策略。

## 元数据查询路径

`getConcreteLines()` 和 `getAbstractLines()` 不读取 `.idx/.bin` 策略数据。

`getConcreteLines()` 走 `CachedMetadataReader::get_concrete_lines()`：

- 按 `abstractLine` 查询 concrete lines；
- 或按 `concreteLine` 精确查询 `concreteLineId`；
- 两个字段都传时按两个条件同时匹配；
- 首次 miss 读取 `meta.db`，结果写入内存 cache；
- 查不到时抛出 `CONCRETE_LINE_NOT_FOUND` 或相关 metadata 错误。

`getAbstractLines()` 走 `get_drill_scenario_lines()`：

- `strategy` 默认 `default`；
- `drillName` 默认 `rfi`；
- 首次 miss 读取 `meta.db` 中的 drill scenario 表；
- 查不到时抛出 `DRILL_SCENARIO_NOT_FOUND`。

## 与 SQLite 路径的差异

二进制策略查询的主要差异是：策略数据不再通过源 SQLite 的行式表和字符串字段读取，而是拆成三类专用结构：

| 职责                 | Binary runtime                          | SQLite baseline             |
| -------------------- | --------------------------------------- | --------------------------- |
| 定位 concrete line   | `.idx` dense record 固定偏移            | SQL 条件 + B-tree / 表访问  |
| 读取策略数值         | `.bin` mmap slice                       | SQLite 行读取和字段解析     |
| 定位 hand            | `hand_id` 二分查找                      | `hole_cards` 字符串条件     |
| 表示 action 是否存在 | `u32 action_mask`                       | 行存在性或过滤条件          |
| 动作语义             | `action_schema_id -> action_blob` cache | action 字段 / join / 行字段 |

这条路径减少了 SQL 解析、通用行格式解析、字符串比较和重复字段存储等开销。但具体快多少取决于 workload、维度、page cache、进程边界和 benchmark 口径，不能只从链路结构推导。

## 运行时状态速查

`stats()` 返回：

```typescript
{
  schemaCount: 0,
  openHandleCount: 0,
  knownDimensions: [
    "default_6max_100BB",
    "default_6max_200BB",
  ],
}
```

字段含义：

| 字段              | 含义                                          |
| ----------------- | --------------------------------------------- |
| `schemaCount`     | 已懒加载到内存的 action schema 数量           |
| `openHandleCount` | 当前 LRU pool 中打开的 `DimensionReader` 数量 |
| `knownDimensions` | manifest 中可查询维度的展示名，已排序         |

典型变化：

1. constructor 后：`schemaCount=0`，`openHandleCount=0`。
2. `getConcreteLines()` 后：metadata cache 可能增加，但 `schemaCount/openHandleCount` 不变。
3. `prewarm()` 后：`openHandleCount` 增加，`schemaCount` 不变。
4. 第一次 `queryHandStrategy()` 或 `queryBatch()` 后：`openHandleCount` 至少为 1，`schemaCount` 可能增加。
