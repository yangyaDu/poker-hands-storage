# Bun 进程内 Native SDK 实现草案

更新日期：2026-07-05

## 状态

本文档是实现方案和阶段记录。

当前最小闭环已经实现：

- `range-store-core` 新增 `metadata` 模块。
- `range-store-core` 新增 `RangeStoreFacade`，封装 metadata lookup 和 range 查询。
- `service` 的 metadata 读取改为复用 `range-store-core::metadata`。
- 新增 `range-store-native` workspace crate 和 npm 包骨架。
- `range-store-native` 已暴露 `PokerHandsRange`、`getConcreteLines`、`getAbstractLines`、`handsByActions`、`queryHandStrategy`、`queryBatch`、`prewarm`、`stats`。
- `storage-tools benchmark-native` 已完成 Rust core、native direct、native SDK、HTTP service 的 fair runner，并覆盖 hot、cold、RSS、drill metadata 和 `line-to-hands-by-actions` 单链路。
- Windows MSVC 目标下已具备 Rust workspace test、clippy、Bun SDK contract 和 HTTP consistency 测试入口。

尚未完成：

- Linux 生产目标 `.node` 产物构建和容器化验证。
- Kubernetes PVC 挂载验证。
- 完整业务 `line-transition` prefix/full 双节点 benchmark。

当前项目已经具备 `range-store-core`、`service`、`storage-tools` 的清晰边界。新的方向不是推翻现有结构，而是在这个边界上新增一个面向 Bun/TypeScript 后端的进程内 native SDK，让业务后端像访问 SQLite 一样直接访问只读 RangeDB 文件。

## 核心结论

推荐新增一个与 `service`、`storage-tools` 平级的 native SDK 模块：

```text
backend_framework(Bun/TypeScript)
  -> @your-scope/range-store-native
  -> Node-API / N-API native addon
  -> range-store-core
  -> manifest.json + meta.db + .idx + .bin
```

这个 SDK 不是 HTTP SDK。

它不负责发请求到 `poker-hands-storage-service`，而是在 Bun 业务进程内部加载 Rust native addon，直接调用 `range-store-core` 的查询能力。

`service` 可以继续保留，用于：

- Swagger / OpenAPI 调试。
- 独立 HTTP 服务部署。
- 接口兼容验证。
- 和 native SDK 做 benchmark 对比。

线上主业务路径可以逐步切换为：

```text
用户请求
  -> backend_framework
  -> 本进程 PokerHandsRange native SDK
  -> 只读 RangeDB 文件
  -> 返回业务响应
```

## N-API 和 Node-API 的关系

本文中的 `N-API` 指的就是 `Node-API`。

它是 Node.js 的原生扩展 ABI。Rust 侧通常使用 `napi-rs` 构建 `.node` 文件，Bun 侧可以直接加载 Node-API native addon。

相关依据：

- Bun Node-API 文档说明 Bun 实现了 Node-API 的大部分接口，并支持直接 `require()` `.node` 文件：<https://bun.sh/docs/runtime/node-api>
- Bun FFI 文档说明 `bun:ffi` 仍有实验性质，生产环境与复杂对象绑定更推荐 Node-API module：<https://bun.sh/docs/runtime/ffi>

因此当前推荐：

```text
优先：Rust + napi-rs + .node native addon
不优先：bun:ffi + C ABI
```

原因是当前查询接口包含字符串、数组、对象返回、错误码和生命周期管理，不是简单的数值函数调用。Node-API 更适合作为长期维护的 SDK 边界。

## 目标

1. Bun/TypeScript 后端可以在进程内直接打开 RangeDB 运行目录。
2. 查询逻辑继续复用 `range-store-core`，不在 TypeScript 中重写 `.idx/.bin` 解码。
3. RangeDB 数据只读访问，支持多个 backend Pod 同时挂载同一份数据。
4. 查询语义和现有 HTTP API 保持一致，包括：
   - `abstract_line/concrete_line -> concrete line rows`
   - `concrete_line_id + actions + frequency -> hole_cards`
   - 单手牌策略查询
   - 批量查询
   - 错误码和错误消息
5. 保留 `service` 作为可选 HTTP 形态，不强制业务后端走 HTTP。
6. 后续 benchmark 能比较：
   - SQLite 本地访问
   - Rust HTTP service
   - Bun native SDK
   - Binary `.idx/.bin` 查询

## 非目标

1. 不把完整 RangeDB 数据加载进内存。
2. 不把 `.bin` 放进 `emptyDir.medium: Memory` 作为常规部署方式。
3. 不在 TypeScript 里重新实现二进制格式解析。
4. 不在 native SDK 中提供写入、修改、增量更新能力。
5. 不要求删除当前 Rust HTTP service。
6. 不在当前阶段做多语言 SDK，例如 Java、Python、Go。
7. 不把 `range_store_builder` 放进 native SDK。
8. 不把 full standalone/cross verification 放进业务后端运行时。
9. 不让 native SDK 负责生成 benchmark 报告。

## 推荐目录结构

新增模块后，顶层结构建议为：

```text
range-store-core
  核心存储格式、读取、校验、查询能力

range-store-native
  Bun/Node 可调用的 native addon
  依赖 range-store-core
  输出 .node 文件和 TypeScript 类型声明

service
  可选 HTTP API、OpenAPI、请求校验、错误映射、Docker 服务入口
  依赖 range-store-core

storage-tools
  离线构建、验证、benchmark、存储方案分析
  依赖 range-store-core
```

依赖方向保持单向：

```text
service ----------\
storage-tools ----- -> range-store-core
range-store-native /
```

`service`、`storage-tools`、`range-store-native` 三者之间不互相依赖业务代码。

## 能力边界规划

引入 `range-store-native` 后，项目能力需要按“生产运行时”和“离线数据生产/验收”分开。

`range-store-native` 只作为业务后端运行时依赖。它负责在 Bun/TypeScript 进程内打开只读 RangeDB，并提供查询、轻量启动校验、prewarm 和运行时 stats。

`storage-tools` 继续作为离线工具集合。它负责构建数据、验证数据、编排 benchmark 和生成报告，不进入业务后端运行镜像。

| 能力 | 推荐归属 | 是否进入业务后端运行时 | 说明 |
| --- | --- | --- | --- |
| `.idx/.bin` 读取和 pack decode | `range-store-core` | 间接进入 | native、service、tools 共同复用 |
| `concrete_line -> concrete_line_id` | `range-store-core` facade | 间接进入 | service/native 都调用同一实现 |
| `hands-by-actions` 查询语义 | `range-store-core` facade | 间接进入 | action/frequency 过滤不能分叉 |
| `range_store_builder` | `storage-tools` | 否 | 从 SQLite 生成 `manifest/meta/idx/bin`，会写文件 |
| build resume / build-state | `storage-tools` | 否 | 属于离线构建流程 |
| standalone verify | `storage-tools` | 否 | 发布前验证二进制目录自身格式和 checksum |
| source cross verify | `storage-tools` | 否 | 需要源 SQLite，比对新旧数据一致性 |
| benchmark 编排和报告 | `storage-tools` | 否 | 统一调度 SQLite/core/native/HTTP 多种被测对象 |
| native 查询 benchmark 被测入口 | `range-store-native` | 是 | 生产主路径性能验收要测这一层 |
| native smoke test | `range-store-native` | 否，测试期使用 | 验证 `.node` 能加载、API 能调用 |
| runtime lightweight validation | `range-store-native` | 是 | 启动时检查 manifest/meta/idx/bin 可打开 |
| runtime prewarm / stats | `range-store-native` | 是 | 业务进程需要观测和预热热点维度 |
| HTTP API / Swagger | `service` | 否，除非选择独立服务部署 | 保留调试、兼容和对比能力 |

边界原则：

- 数据是否正确，由 `storage-tools verify` 负责证明。
- native SDK 是否能正确消费数据，由 `range-store-native` 的 smoke/integration test 负责证明。
- 生产启动时数据是否可用，由 `range-store-native` 的 lightweight validation 负责证明。
- 生产主路径性能，由 `storage-tools` 编排 native benchmark 后在报告中证明。

不要把 `range_store_builder`、full verification 或报告生成塞进 `range-store-native`。否则业务后端依赖会同时包含读路径、写路径、源 SQLite 对比、报告生成和发布验收逻辑，职责会重新变乱。

## 需要下沉到 range-store-core 的能力

当前 `range-store-core` 已经包含 `.idx/.bin` 读取、pack decode、action schema 加载、部分查询服务。

为了让 native SDK 不依赖 `service`，还需要把以下 HTTP 无关能力下沉到 `range-store-core`：

| 能力 | 当前位置 | 建议归属 |
| --- | --- | --- |
| `meta.db` concrete line 查询 | `service` metadata/query 相关代码 | `range-store-core` |
| `ConcreteLineFilter` / `ConcreteLineRow` | `service` | `range-store-core` |
| drill scenario metadata 查询 | `service` | `range-store-core` |
| action filter 解析后的核心匹配语义 | `service` / `range-store-core` 有重复 | 统一到 `range-store-core` |
| `hands-by-actions` 业务过滤 | `service` 已有更完整语义 | 统一到 `range-store-core` |
| 业务错误分类 | `service` HTTP 映射内 | core 输出稳定错误枚举，service/native 分别映射 |

下沉原则：

- HTTP 请求体、HTTP 状态码、OpenAPI schema 仍留在 `service`。
- TypeScript 包装、`.node` 加载、TS 类型声明留在 `range-store-native`。
- 元数据查询、二进制查询、业务过滤、只读数据校验属于 `range-store-core`。

## TypeScript SDK 形态

建议 SDK 对业务后端暴露一个稳定对象：

```ts
import { getPokerHandsRangeSingleton } from "@your-scope/range-store-native";

const ranges = getPokerHandsRangeSingleton({
  dataDir: "/data/range-strata",
  maxOpenHandles: 2,
  verifyChecksums: false,
});

const concreteLineResult = ranges.getConcreteLines({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLine: "F-F-F-R2-F-R7-R15",
});
if (concreteLineResult.code !== 0) {
  throw new Error(concreteLineResult.message ?? "concrete line lookup failed");
}
const concreteLineId = concreteLineResult.data.lines[0].concreteLineId;

const result = ranges.handsByActions({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLineId,
  actions: [],
  frequency: 0.005,
});
if (result.code !== 0) {
  throw new Error(result.message ?? "range query failed");
}
const holeCards = result.data.holeCards;
```

第一阶段建议暴露这些接口：

```ts
type PokerHandsRangeOptions = {
  dataDir: string;
  maxOpenHandles?: number;
  verifyChecksums?: boolean;
};

type DimensionInput = {
  strategy?: string;
  playerCount: 6 | 8 | 9;
  depthBb: 100 | 200 | 300;
};

type HandsByActionsRequest = DimensionInput & {
  concreteLineId: number;
  actions?: string[];
  frequency?: number;
};

type HandsByActionsResponse = {
  holeCards: string[];
};

type ApiResponse<T> = {
  code: number;
  data: T | null;
  message: string | null;
};

type ActionResult = {
  actionName: string;
  actionSize: number;
  amountBb: number;
  frequency: number;
  handEv?: number;
};

type ConcreteLinesData = {
  lines: Array<{
    concreteLineId: number;
    abstractLine: string;
    concreteLine: string;
  }>;
};

type AbstractLinesData = {
  abstractLines: string[];
};

type QueryHandStrategyData = {
  actions: ActionResult[];
};

type QueryBatchData = {
  results: Array<{
    concreteLineId: number;
    holeCards: string;
    actions?: ActionResult[];
    error?: {
      code: number;
      message: string;
    };
  }>;
};
```

方法建议：

```ts
class PokerHandsRange {
  constructor(options: PokerHandsRangeOptions);

  getConcreteLines(request: {
    strategy?: string;
    playerCount: 6 | 8 | 9;
    depthBb: 100 | 200 | 300;
    abstractLine?: string;
    concreteLine?: string;
  }): ApiResponse<ConcreteLinesData>;

  getConcreteLinesRaw(request: {
    strategy?: string;
    playerCount: 6 | 8 | 9;
    depthBb: 100 | 200 | 300;
    abstractLine?: string;
    concreteLine?: string;
  }): ConcreteLinesData;

  getAbstractLines(request: {
    strategy?: string;
    drillName?: string;
    playerCount: 6 | 8 | 9;
    drillDepth: 100 | 200 | 300;
  }): ApiResponse<AbstractLinesData>;

  getAbstractLinesRaw(request: {
    strategy?: string;
    drillName?: string;
    playerCount: 6 | 8 | 9;
    drillDepth: 100 | 200 | 300;
  }): AbstractLinesData;

  handsByActions(request: HandsByActionsRequest): ApiResponse<HandsByActionsResponse>;
  handsByActionsRaw(request: HandsByActionsRequest): HandsByActionsResponse;

  queryHandStrategy(request: {
    strategy?: string;
    playerCount: 6 | 8 | 9;
    depthBb: 100 | 200 | 300;
    concreteLineId: number;
    holeCards: string;
  }): ApiResponse<QueryHandStrategyData>;

  queryBatch(request: {
    strategy?: string;
    playerCount: 6 | 8 | 9;
    depthBb: 100 | 200 | 300;
    items: Array<{
      concreteLineId: number;
      holeCards: string;
    }>;
  }): ApiResponse<QueryBatchData>;

  // Raw response variants are kept for diagnostics and lower-level benchmark work.
  queryHandStrategyRaw(request: {
    strategy?: string;
    playerCount: 6 | 8 | 9;
    depthBb: 100 | 200 | 300;
    concreteLineId: number;
    holeCards: string;
  }): {
    handCode: string;
    actions: Array<{
      actionName: string;
      actionSize: number;
      amountBb: number;
      frequency: number;
      handEv?: number;
    }>;
  };

  queryBatchRaw(request: {
    strategy?: string;
    playerCount: 6 | 8 | 9;
    depthBb: 100 | 200 | 300;
    items: Array<{
      concreteLineId: number;
      holeCards: string;
    }>;
  }): QueryBatchResponse;

  prewarm(request: DimensionInput): { openHandleCount: number };

  stats(): {
    schemaCount: number;
    openHandleCount: number;
    knownDimensions: string[];
  };

  close(): void;
}
```

命名建议：

- TS 侧使用 `camelCase`。
- Rust core 内部继续使用 Rust 风格命名。
- 数据文件、manifest 和表名不因 SDK 改动而改名。

## Rust Native Addon 设计

建议新增：

```text
range-store-native/
  Cargo.toml
  package.json
  index.ts
  index.d.ts
  src/lib.rs
  scripts/build.ts
  tests/
```

Rust crate 类型：

```toml
[lib]
crate-type = ["cdylib"]
```

核心依赖：

```toml
[dependencies]
range-store-core = { path = "../range-store-core" }
napi = "..."
napi-derive = "..."
serde = { version = "1", features = ["derive"] }
```

Rust 侧包装对象示意：

```rust
#[napi]
pub struct NativePokerHandsRange {
    inner: Arc<RangeStoreFacade>,
}

#[napi]
impl NativePokerHandsRange {
    #[napi(constructor)]
    pub fn new(options: NativePokerHandsRangeOptions) -> napi::Result<Self> {
        // open manifest/meta.db, validate files, initialize handle pool
    }

    #[napi(js_name = "getConcreteLines")]
    pub fn get_concrete_lines(
        &self,
        request: ConcreteLinesRequest,
    ) -> ConcreteLinesEnvelope {
        // meta.db abstract lookup or exact concrete-line lookup
    }

    #[napi(js_name = "handsByActions")]
    pub fn hands_by_actions(
        &self,
        request: HandsByActionsRequest,
    ) -> napi::Result<HandsByActionsResponse> {
        // idx direct lookup -> bin decode -> action/frequency filter
    }
}
```

`RangeStoreFacade` 建议放在 `range-store-core`，让 `service` 和 `range-store-native` 共用同一套业务能力。对外 TypeScript 类名使用 `PokerHandsRange`。

## 同步还是异步

第一阶段建议使用同步方法。

原因：

- 查询是本地只读文件访问，单次查询目标应在毫秒内甚至更低。
- 同步接口更接近 SQLite 的本地访问体验。
- 生命周期和错误处理更简单。
- 后续如批量查询或冷首次访问证明会阻塞事件循环，再新增 async 版本。

保守做法：

- `getConcreteLines`、`handsByActions`、`queryHandStrategy` 先提供同步接口。
- 大批量接口后续可以提供 `queryBatchAsync`，通过 `napi-rs` task 或 Bun worker 隔离。

## 只读数据和 mmap

RangeDB 运行目录应视为不可变只读数据：

```text
manifest.json
meta.db
range_data_default_6max_100BB.idx
range_data_default_6max_100BB.bin
...
```

SDK 打开数据时：

1. 读取 `manifest.json`。
2. 只读打开 `meta.db`。
3. 校验 `.idx/.bin` 文件存在。
4. 按需打开维度 reader。
5. `.idx/.bin` 通过 mmap 或 mmap-backed reader 读取。

需要强调：

- mmap 不等于把整个 `.bin` 复制进进程堆。
- 文件页通常在首次访问时进入 OS page cache。
- 同一节点上多个进程读同一只读文件，热点页可能由 OS page cache 复用。
- 跨节点没有共享内存，每个节点有自己的 page cache。

因此不建议用 `emptyDir.medium: Memory` 主动把完整 RangeDB 放进内存盘。

Kubernetes 官方说明，`emptyDir.medium: "Memory"` 会挂载 tmpfs，写入文件会计入写入容器的内存限制：<https://kubernetes.io/docs/concepts/storage/volumes/>

## Kubernetes 数据挂载建议

因为 RangeDB 是只读数据，推荐数据挂载使用只读卷。

### 方案 A：PVC 只读共享

适合第一阶段落地。

```text
backend pod 1 \
backend pod 2  -> ReadOnlyMany / ReadWriteMany PVC -> /data/range-strata
backend pod 3 /
```

Kubernetes `ReadOnlyMany` 表示卷可以被多个节点只读挂载：<https://kubernetes.io/docs/concepts/storage/persistent-volumes/>

Pod 中仍应显式设置：

```yaml
volumeMounts:
  - name: rangedb
    mountPath: /data/range-strata
    readOnly: true
```

优点：

- 数据只维护一份。
- 版本切换和回滚相对简单。
- 多个 backend Pod 可以同时读取。

风险：

- 如果底层是网络文件系统，首次访问或 cache miss 会受存储延迟影响。
- 需要确认云厂商或集群存储是否支持 ROX/RWX。

### 方案 B：节点本地缓存

适合后续性能优化。

```text
数据分发 Job / DaemonSet
  -> 每个 node 准备 /mnt/rangedb/v1

backend pod
  -> 挂载 node local path
```

优点：

- mmap 和 page cache 更接近本地磁盘访问。
- 避免网络文件系统尾延迟。

风险：

- 数据分发、版本一致性、节点扩容、节点故障都更复杂。
- 需要调度约束和发布流程配合。

第一阶段不建议直接上节点本地缓存，除非 PVC 压测证明是瓶颈。

## 发布和版本管理

数据目录建议版本化：

```text
/data/range-strata/
  v2026-07-04/
    manifest.json
    meta.db
    *.idx
    *.bin
  current -> v2026-07-04
```

backend 使用：

```text
RANGE_STORE_DATA_DIR=/data/range-strata/current
```

发布流程：

1. `storage-tools` 从 slim SQLite 构建新版本 RangeDB。
2. 跑 standalone verify。
3. 跑 source cross verify。
4. 跑 hot/cold/native benchmark。
5. 上传或挂载新版本目录。
6. backend 滚动发布，启动时打开新目录。
7. 旧版本保留一段时间用于回滚。

只读数据不建议原地覆盖，因为正在运行的进程可能已经 mmap 旧文件。

## 错误语义

`range-store-core` 应提供稳定错误分类，native SDK 再映射成 JS Error。

建议错误结构：

```ts
class PokerHandsRangeError extends Error {
  code: string;
  publicCode: number;
  details?: unknown;
}
```

建议错误码：

| code | publicCode | 说明 |
| --- | --- | --- |
| `INVALID_ARGUMENT` | 1000 | 请求参数不合法 |
| `DIMENSION_NOT_FOUND` | 404 | strategy/playerCount/depthBb 不存在 |
| `CONCRETE_LINE_NOT_FOUND` | 404 | concrete_line 或 concrete_line_id 不存在 |
| `HAND_NOT_FOUND` | 404 | 手牌不在当前行动线 range 中 |
| `NO_HANDS_FOUND` | 404 | `handsByActions` 筛选后为空 |
| `ACTION_SCHEMA_NOT_FOUND` | 404 | action schema 缺失 |
| `DATA_FILE_NOT_FOUND` | 404 | manifest/meta/idx/bin 文件缺失 |
| `INVALID_FORMAT` | 500 | `.idx/.bin/meta.db` 格式异常 |
| `CHECKSUM_MISMATCH` | 500 | checksum 校验失败 |
| `INTERNAL` | 500 | 未分类内部错误 |

HTTP service 可以继续把这些错误映射到 HTTP status；native SDK 则直接抛出带 `code/publicCode` 的 JS Error。

## Benchmark 需要补充

引入 native SDK 后，需要新增 benchmark 维度：

```text
SQLite local
Rust HTTP binary
Bun native binary
Rust core direct
```

benchmark 的验收口径应以 `Bun native binary` 为生产主路径，但 benchmark 编排和报告生成仍放在 `storage-tools`。`range-store-native` 只提供被测入口和必要的 smoke test。

建议报告分层：

| 层级 | 作用 | 结论用途 |
| --- | --- | --- |
| SQLite local | 旧方案基准 | 判断新方案是否不慢于 SQLite |
| Rust core direct | 纯存储核心基准 | 定位 `.idx/.bin` 和 decode 本身性能 |
| Bun native binary | 生产主路径 | 作为 Bun 后端内嵌接入的验收口径 |
| Rust HTTP binary | 兼容/调试路径 | 量化 HTTP 服务额外成本 |

至少覆盖：

- 单个场景 + 单手牌查询。
- 单个行动线下全部起手牌查询。
- `getConcreteLines({ concreteLine }) -> concreteLineId`。
- `getConcreteLines({ concreteLine }) -> concreteLineId -> handsByActions` 组合链路。
- batch 查询。
- 冷启动首次查询。
- 热查询 p50 / p95 / p99。
- RSS、page cache 观察值、open handle 数。

对比时需要分清：

- native addon 加载耗时。
- `PokerHandsRange` 构造和打开数据目录耗时。
- 首次访问某个维度的 page fault 成本。
- 热查询纯查询耗时。

## 实施步骤

当前阶段状态：

| 阶段 | 状态 | 说明 |
| --- | --- | --- |
| 阶段 1：核心能力下沉 | 已完成最小闭环 | metadata 和 `RangeStoreFacade` 已进入 `range-store-core` |
| 阶段 2：新增 range-store-native 最小版本 | 已完成最小闭环 | `PokerHandsRange` 已支持 concrete line lookup、hands-by-actions、单手牌查询、prewarm、stats |
| 阶段 3：业务接口补齐 | 已完成当前口径 | 默认业务方法 `queryHandStrategy`、`queryBatch`、`handsByActions`、metadata 查询均返回业务码 envelope；`getConcreteLines` 同时覆盖 abstract line 列表查询和 concrete line 精确查 id；直接返回/抛异常版本统一使用 `Raw` 后缀 |
| 阶段 4：native benchmark | 已完成 fair runner + drill + HTTP service 对比 | `storage-tools benchmark-native` 已输出 `core:*`、`native-direct:*`、`native-sdk:*`、`http-service:*` case、drill metadata case、冷启动分解和各 worker 内存观察；HTTP 一致性抽样由 `bun run test:http-consistency` 覆盖 |
| 阶段 5：Kubernetes 接入验证 | 未完成 | 需要 Linux `.node` 和业务后端容器验证 |

### 阶段 1：核心能力下沉

目标：让 `range-store-core` 可以独立完成 HTTP 无关的业务查询。

任务：

1. 把 `service` 中的 metadata reader 迁移到 `range-store-core`。已完成。
2. 在 `range-store-core` 中新增 `RangeStoreFacade`。已完成。
3. 统一 `hands-by-actions` 的 action/frequency 过滤语义。已完成，解析与匹配逻辑下沉到 `range-store-core`，native 和 HTTP 共用 amount-aware action filter 与 multi-action OR 语义。
4. `service` 改为调用 `RangeStoreFacade`，不再保留重复业务逻辑。已完成：HTTP `QueryService` 现在是 `RangeStoreFacade` 的薄 wrapper，错误码映射仍由 service 边界负责。
5. 保持现有 HTTP API 测试通过。已完成。

验收：

```text
cargo fmt --all -- --check
cargo test --workspace --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
```

### 阶段 2：新增 range-store-native 最小版本

目标：Bun 能加载 native addon，并完成核心查询。

任务：

1. 新增 `range-store-native` crate。已完成。
2. 接入 `napi-rs`。已完成。
3. 输出 `.node` 文件。已完成 Windows MSVC 本地构建。
4. 暴露 `PokerHandsRange` 构造函数。已完成。
5. 暴露 `getConcreteLines`。已完成。
6. 暴露 `handsByActions`。已完成。
7. 补 TypeScript 类型声明。已完成。
8. 补 Bun smoke test。已手动验证，后续需要沉淀成自动化测试。

验收：

```text
bun test
cargo test -p range-store-native --target x86_64-pc-windows-msvc
```

### 阶段 3：业务接口补齐

目标：native SDK 覆盖当前业务需要的主要能力。

任务：

1. 暴露 `queryHandStrategy`。已完成。
2. 暴露 `queryBatch`。已完成。
3. 默认 `queryHandStrategy` 返回 `{ code, data: { actions }, message }`，直接返回 full payload 的版本为 `queryHandStrategyRaw`。已完成。
4. 默认 `queryBatch` 返回 `{ code, data: { results }, message }`，每个 result 保留 `concreteLineId`、`holeCards`、`actions` 或 `error`；直接返回 full payload 的版本为 `queryBatchRaw`。已完成。
5. `handsByActions` 使用业务码 envelope。已完成，`data` 只包含 `{ holeCards }`。
6. 暴露 `prewarm`。已完成。
7. 暴露 `stats`。已完成。
8. metadata 高频路径加缓存。已完成：`RangeStoreFacade` 复用 `CachedMetadataReader`，对 `concrete_line` / `abstract_line` concrete-lines lookup 和 drill scenario abstract lines 做 key-level lazy cache。
9. 暴露 `getConcreteLines`、`getAbstractLines` 的 envelope 版本；`concreteLine -> concreteLineId` 统一通过 `getConcreteLines({ concreteLine })` 完成，业务 SDK 不再保留重复的 `getConcreteLineId` 方法。已完成。
10. 文档补充 Bun 后端接入示例。

验收：

- SDK 返回结构和 HTTP API 对齐。
- 同一批样本下 native SDK 与 HTTP service 返回一致。

### 阶段 4：native benchmark

目标：确认是否真正优于 HTTP service，并量化冷启动和热查询表现。

任务：

1. 在 `storage-tools` 中新增 native benchmark 编排入口。已完成：`benchmark-native`。
2. 由 `range-store-native` 提供 Bun native 被测入口。已完成：worker 同时加载 `range-store-native/index.node` 和 `range-store-native/index.js`。
3. 对比 SQLite local、Rust core direct、Bun native binary、Rust HTTP binary。已完成：`benchmark-native` 复用同一个 workload JSON，并把 core、native-direct、native-sdk、http-service 拆到独立 worker/service 执行。
4. 输出 p50 / p95 / p99。已完成，复用 `BenchmarkRunReport` case 指标。
5. 输出冷启动分解。已完成，记录 dynamic import、`PokerHandsRange` 构造、首次 hand query。
6. 输出内存观察结果。已完成，记录 Bun 子进程 `process.memoryUsage()` 前后值。
7. 输出 native SDK 与 HTTP service 的结果一致性抽样。已完成：`range-store-native/tests/http-consistency.test.js` 在同一数据目录下抽样比较 `concrete-lines`、`drill-scenarios`、`hand-strategy`、batch、`hands-by-actions`，命令为 `PHS_HTTP_URL=http://... bun run test:http-consistency`。

命令示例：

```powershell
cargo run -p poker-hands-storage-tools -- benchmark-native `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --dimension default:6:100 `
  --iterations 1000 `
  --batch-iterations 200 `
  --http-service-bin target\x86_64-pc-windows-msvc\debug\poker-hands-storage-service.exe `
  --out reports\benchmark-bun-native.json `
  --md reports\benchmark-bun-native.md
```

常用参数：

| 参数 | 说明 |
| --- | --- |
| `--native-entry` | Bun SDK JS 入口，默认 `range-store-native/index.js` |
| `--bun` | Bun 可执行文件，默认 `bun` |
| `--max-open-handles` | 传给 `PokerHandsRange` 的 handle pool 上限，默认 2 |
| `--verify-checksum` | 构造 native store 时启用 checksum 验证 |
| `--workload` / `--write-workload` | 读取或写出共享 workload，便于和 SQLite / Rust core direct 使用同一批查询 |
| `--http-service-bin` | 指定被测 HTTP service 可执行文件；未指定时 runner 会尝试使用当前工具同目录下的 `poker-hands-storage-service.exe` |

当前覆盖的 native case：

- `core:*`：直接调用 `range_store_core::query::RangeStoreFacade`，作为进程内 Rust 基线。
- `native-direct:*`：直接调用 `index.node` 的默认业务 envelope 方法，用于观察 N-API 固定成本。
- `native-sdk:*`：调用 `index.js` SDK 包装，用于观察 JS SDK 包装成本。
- `http-service:*`：通过 loopback HTTP 调用独立 `poker-hands-storage-service`，用于量化 HTTP/JSON 边界成本。
- `*:concrete-lines-exact`：通过 `getConcreteLines({ concreteLine })` 精确查询 concrete line 并读取 `concreteLineId`。
- `*:hand-strategy`：单个 `concrete_line_id + hand` 业务 envelope 查询。
- `*:batch-hand-strategy` 和 `*:batch-size-*`：批量业务 envelope 查询；前者是默认 batch size 主 case，后者是 batch size sweep。
- `*:hands-by-actions`：单行动线手牌范围查询。
- `*:drill-scenarios-metadata`：通过 `getAbstractLines` 查询 drill scenario abstract lines。
- `*:line-to-hands-by-actions`：`getConcreteLines({ concreteLine }) -> concreteLineId -> handsByActions` 组合链路。

附加 benchmark：

- `benchmark-drill-metadata`：隔离比较 raw SQLite schema-detect、prepared SQLite、CachedMetadataReader 三组 drill metadata 路径。
- `bun run test:http-consistency`：对 native SDK 和 HTTP service 做逐项结果一致性抽样；需要先启动 HTTP service 并设置 `PHS_HTTP_URL`。

验收：

- native SDK 热查询不慢于 HTTP service，并作为生产主路径验收口径。
- native SDK 冷启动成本可解释。
- benchmark 报告明确是否建议生产从 HTTP service 切换到 Bun 进程内 native SDK。

### 阶段 5：Kubernetes 接入验证

目标：验证只读 PVC 挂载下 backend 进程内查询可用。

任务：

1. backend 镜像内包含 Bun 业务代码和 native addon。
2. RangeDB 通过只读 PVC 挂载。
3. backend 启动时构造一个进程级 `PokerHandsRange` singleton 并打开数据目录，业务模块复用该实例，不要按请求或按模块重复构造。
4. readiness 在 native store 打开并完成可选 prewarm 后再通过。
5. 验证滚动发布和数据版本回滚。

验收：

- 多副本 backend 同时读取同一份只读数据。
- Pod 重启后能稳定打开数据。
- 数据目录只读挂载，业务容器不能写入 RangeDB。

## 主要风险

| 风险 | 影响 | 应对 |
| --- | --- | --- |
| Bun 对 Node-API 支持与 Node.js 不完全一致 | native addon 运行兼容问题 | 用 Bun 做真实 smoke test，不只跑 Node.js |
| native addon 构建链路复杂 | CI/CD 增加平台产物管理 | 第一阶段只支持 Linux x64 生产目标，Windows 用于本地开发验证 |
| service 和 native SDK 业务语义分叉 | 返回结果不一致 | 下沉到 `range-store-core`，service/native 只做边界映射 |
| PVC 网络存储尾延迟 | 首次访问或 cache miss 抖动 | benchmark 中单独观测 cold/page cache；必要时升级 node-local cache |
| mmap 文件版本原地覆盖 | 运行中进程读到不一致数据 | 版本目录不可变，发布只切换目录或配置 |
| native 同步调用阻塞 Bun 事件循环 | 大批量查询影响请求处理 | 单次查询同步，批量/重任务后续提供 async 或 worker 隔离 |

## 推荐第一步

先做阶段 1 和阶段 2 的最小闭环：

```text
range-store-core 下沉 metadata + facade
range-store-native 暴露 PokerHandsRange + getConcreteLines + handsByActions
Bun smoke test 验证能在进程内读现有 data/range-strata
```

在这个闭环完成前，不建议直接改业务后端项目，也不建议废弃当前 HTTP service。
