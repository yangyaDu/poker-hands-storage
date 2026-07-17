# Proto V3 SDK 与查询链

更新日期：2026-07-17

Bun/Node native SDK 通过 N-API 包交付给业务项目，运行时复用 `V3Facade`，不需要 Docker。
构造参数中的 `dataDir` 必须指向已校验并解压的版本化 V3 根目录，而不是单维目录或旧 Range Strata
目录。

```js
const store = new PokerHandsRange({
  dataDir: "data/proto-v3-releases/2026-07-17T132350Z",
  maxOpenHandles: 2,
  verifyChecksums: true,
});
```

## 查询链

```text
dimension
  -> root 下 manifest 定位维度目录
  -> mmap 三组 .idx/.pb

drill_name
  -> drill hash index
  -> DrillScenarioPage
  -> abstract_action_path[]

concrete_action_path
  -> concrete hash index
  -> V3 concrete_action_path_id

concrete_action_path_id + hole_cards
  -> hand-strategies.idx 定位 payload
  -> HandStrategy decode/cache
  -> action[]
```

metadata page cache 和 decoded strategy cache 都按字节预算受限；维度 handle 由 LRU pool 管理。运行时
不打开 SQLite，也没有 V2/V3 format dispatch。

## 交付边界

- npm 包包含 JS/TypeScript wrapper 和目标平台 `.node` 二进制。
- V3 数据作为独立、带 SHA-256 的 `tar.zst` 制品发布，不塞入 npm 包。
- 业务部署先校验并解压数据制品，再通过配置把 release root 传给 `dataDir`。
- release 升级或回滚只切换 `dataDir` 指向的版本化目录，不增加 V2 reader 或双读逻辑。
- HTTP service 和 Docker 不是 native SDK 消费的前置条件。

## API

SDK 保留以下业务方法：

- `getConcreteLines`
- `getAbstractLines`
- `queryHandStrategy`
- `queryBatch`
- `handsByActions`
- `prewarm`
- `stats`

`hand_ev IS NULL` 的 V3 sentinel 对 SDK 解码为 `handEv: undefined`，频率为 `0`。错误继续包装为
`RangeStoreError`，错误码来自 V3 facade，例如 `DIMENSION_NOT_FOUND`、
`CONCRETE_LINE_NOT_FOUND`、`HAND_STRATEGY_NOT_FOUND` 或 `INVALID_V3_METADATA`。

发布前必须用相同源 SQLite 对目标 V3 release 执行 full cross verify；SDK 测试不是数据一致性门禁。
