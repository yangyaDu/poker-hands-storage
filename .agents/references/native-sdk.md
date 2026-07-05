# Bun/Node Native SDK

`range-store-native` 是 Bun/Node 进程内只读 SDK，复用 `range-store-core::query::RangeStoreFacade`。

## 构建

```powershell
Set-Location range-store-native
bun install
bun run build:native
```

Windows 默认脚本构建 `x86_64-pc-windows-msvc` 产物。

Linux x64 生产产物脚本：

```powershell
bun run build:native:linux
```

## 测试

SDK contract：

```powershell
Set-Location range-store-native
bun run test:sdk
```

HTTP consistency 需要先启动 HTTP service：

```powershell
$env:PHS_HTTP_URL = "http://127.0.0.1:8080"
bun run test:http-consistency
```

## 公开入口

- `PokerHandsRange`
- `RangeStore`
- `getPokerHandsRangeSingleton(options)`

主要方法：

- `getConcreteLines`
- `getAbstractLines`
- `handsByActions`
- `queryHandStrategy`
- `queryBatch`
- `prewarm`
- `stats`

所有默认业务方法返回 `{ code, data, message }` envelope。

## 边界

- native SDK 不负责源 SQLite 构建。
- native SDK 不负责 source cross verify。
- native SDK 不生成 benchmark 报告。
- 正式 native benchmark 只保留 `core`、`native-sdk`、`http-service` 三组对比。

详细说明见 `docs/native-sdk.md`。
