# Poker Hands Storage 文档地图

更新日期：2026-07-17

当前默认链路：

```text
源 SQLite -> Proto V3 export / cross verify -> V3 root -> HTTP service / Bun native SDK
```

## 当前权威文档

| 信息 | 文档 |
| --- | --- |
| 项目入口、常用命令、运行环境 | [`README.md`](../README.md) |
| V3 格式和实施不变量 | [`proto/v3-business-storage-implementation-plan.md`](proto/v3-business-storage-implementation-plan.md) |
| V3 CLI、服务配置和发布门禁 | [`proto/v3-runtime-and-operations.md`](proto/v3-runtime-and-operations.md) |
| HTTP API 契约 | [`api-business-contract.md`](api-business-contract.md) |
| Bun/Node SDK 与查询链 | [`sdk-and-query-chain-explanation.md`](sdk-and-query-chain-explanation.md) |
| standalone/cross verify 与 SQLite/V3 benchmark | [`verification_and_benchmark.md`](verification_and_benchmark.md) |
| Docker/Compose/Kubernetes 部署 | [`docker-deployment-guide.md`](docker-deployment-guide.md) |
| 代码级数据流 | [`data-flow-overview.md`](data-flow-overview.md) |

[`proto/README.md`](proto/README.md) 同时索引 V3 当前文档和 V2 历史参考。V2 文档中的
`lines.db`、过滤 NULL EV、三方 benchmark 与回退假设都不适用于 V3。

## 实现边界

- `storage-tools::proto_range_storage::v3` 包含 V3 writer、reader、facade、校验和 benchmark。
- `service` 与 `range-store-native` 默认复用 `V3Facade`。
- `range-store-core` 继续提供共享维度、手牌、查询类型和 SQLite 离线访问；其中旧 PFSP reader 是历史参考。
- 线上运行时只需要 V3 目录，不需要 SQLite 或 V2 产物。

当前代码、fixture 回归与真实九维首发数据验收均已完成。2026-07-17 的完整源库 release root 为
`data/proto-v3-releases/2026-07-17T000001Z`；九份 standalone verify、九份 SQLite cross verify 及九份
benchmark 汇总位于 `reports/v3-release-20260717/release-gate-summary.json`，verify/cross 均为零失败和零差异，benchmark 均为
`correctnessVerified=true`。
