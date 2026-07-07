# Bun/Node Native SDK

更新日期：2026-07-07

## 文档职责

本文只描述当前 `range-store-native` 的实际 API、构建测试方式和生产接入边界。历史实现草案和阶段记录不再维护。

## 模块定位

`range-store-native` 是 Bun/Node 进程内只读 SDK：

- Rust 侧通过 `napi-rs` 暴露 `PokerHandsRange`。
- JavaScript 侧通过 `index.js` 加载 `index.node`，返回直接 payload，失败时抛出 `RangeStoreError`。
- 查询语义复用 `range-store-core::query::RangeStoreFacade`，与 HTTP service 保持一致。

它不负责：

- 从源 SQLite 构建 Range Strata Binary。
- source cross verify。
- benchmark 报告生成。
- HTTP 服务部署。

## 公开入口

包入口：

```js
import {
  PokerHandsRange,
  RangeStore,
  getPokerHandsRangeSingleton,
} from "./index.js";
```

构造参数：

```ts
interface PokerHandsRangeOptions {
  dataDir: string;
  maxOpenHandles?: number;
  verifyChecksums?: boolean;
}
```

当前公开方法：

| 方法 | 返回 | 说明 |
| --- | --- | --- |
| `getConcreteLines(request)` | `{ lines }` | 按 `abstractLine` 列 concrete lines，或按 `concreteLine` 精确查 id |
| `getAbstractLines(request)` | `{ abstractLines }` | 查询 drill 场景下的 abstract lines |
| `handsByActions(request)` | `{ holeCards }` | 按 concrete line id、actions、frequency 过滤手牌 |
| `queryHandStrategy(request)` | `{ actions }` | 查询单手牌策略 |
| `queryBatch(request)` | `{ results: [{ concreteLineId, holeCards, actions }] }` | 批量查询单手牌策略；任一 item 非法或找不到时整个调用抛错 |
| `prewarm(request)` | `{ openHandleCount }` | 打开指定维度并加载必要 metadata |
| `stats()` | `{ schemaCount, openHandleCount, knownDimensions }` | 查询 SDK 内部缓存和 handle 状态 |

`RangeStore` 是 `PokerHandsRange` 的别名。`getPokerHandsRangeSingleton(options)` 会复用同一组选项下的单例；如果重复初始化时选项不同，会抛出错误。

## 错误契约

Native SDK 不返回 `{ code, data, message }` envelope。调用成功时返回直接 payload；调用失败时抛出 `RangeStoreError`：

```ts
class RangeStoreError extends Error {
  name: "RangeStoreError";
  code: RangeStoreErrorCode;
}
```

`message` 是完整可展示错误信息。batch 失败时，message 会带上失败 item 的下标和业务上下文，例如：

```text
Batch item requests[1] failed: Invalid card format: AsXx from concrete_line_id=1, dimension=default:6:100
```

当前公开的 `RangeStoreErrorCode`：

| code | 语义 |
| --- | --- |
| `INVALID_ARGUMENT` | 参数语义非法，包括手牌字符串无法解析、action filter 或 frequency 非法 |
| `DIMENSION_NOT_FOUND` | 查询维度不存在 |
| `DATA_FILE_NOT_FOUND` | 数据目录或运行文件不存在 |
| `INVALID_FORMAT` | manifest、idx、bin 或 metadata 格式损坏 |
| `META_DB_ERROR` | `meta.db` 读取异常 |
| `ACTION_SCHEMA_NOT_FOUND` | action schema 不存在 |
| `ABSTRACT_LINE_NOT_FOUND` | abstract line 没有匹配结果 |
| `CONCRETE_LINE_NOT_FOUND` | concrete line 不存在 |
| `HAND_STRATEGY_NOT_FOUND` | 指定手牌在该 concrete line 下没有策略 |
| `DRILL_SCENARIO_NOT_FOUND` | drill scenario 没有匹配结果 |
| `HANDS_NOT_FOUND` | 没有手牌满足 actions/frequency 筛选 |
| `INTERNAL` | 未归类内部错误或无法识别的 native 异常 |

## 构建和测试

Windows 本地构建：

```powershell
Set-Location range-store-native
bun install
bun run build:native
bun run test:sdk
```

Linux x64 产物构建脚本已存在：

```powershell
bun run build:native:linux
```

HTTP consistency 测试需要先启动 HTTP service，并设置 `PHS_HTTP_URL`：

```powershell
$env:PHS_HTTP_URL = "http://127.0.0.1:8080"
bun run test:http-consistency
```

## 与 HTTP service 的关系

两者是平级运行入口：

| 入口 | 使用场景 | 边界成本 |
| --- | --- | --- |
| HTTP service | 跨进程、跨语言、容器化服务 | HTTP/JSON 序列化和 loopback/网络成本 |
| Bun native SDK | Bun/Node 业务进程内查询 | N-API 边界和 JS 包装成本 |

当前正式 benchmark 只保留 `core`、`native-sdk`、`http-service` 三组对比。

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
