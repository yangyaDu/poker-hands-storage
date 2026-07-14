# Poker Hands Storage 文档地图

更新日期：2026-07-10

## 收束原则

文档按职责维护，避免同一组状态、命令或结论在多个文件里漂移。

| 信息类型 | 权威位置 |
| --- | --- |
| 项目入口、模块职责、常用命令 | `README.md` |
| 文档职责和阅读路径 | `docs/README.md` |
| 当前剩余工作和验收标准 | `docs/roadmap.md` |
| 文件格式、pack 编码、查询流程 | `docs/range-db-binary-storage-design.md` |
| Proto V2 schema、字段语义、导出、查询与基准 | `docs/proto/README.md` |
| HTTP API 请求/响应、错误码、业务语义 | `docs/api-business-contract.md` |
| Bun/Node native SDK API、构建测试、生产接入边界和查询链路 | `docs/sdk-and-query-chain-explanation.md` |
| standalone/cross verify、Float32、checksum、发布前验证、benchmark 脚本介绍 | `docs/verification_and_benchmark.md` |
| Range Strata Binary 的性能、体积、内存和 benchmark 结论 | `docs/binary-vs-sqlite-benchmark-and-verification-report.md` |
| Proto V2 的格式、查询、导出与 benchmark 口径 | `docs/proto/README.md` |
| Docker/Compose/Kubernetes、发布、回滚、prewarm | `docs/docker-deployment-guide.md` |
| 代码级构建和查询数据流速查 | `docs/data-flow-overview.md` |

已删除的历史快照和草案类文档不再作为引用源维护；当前状态统一收敛到上表。

## 当前项目快照

```text
1.45GB slim SQLite -> 345.5MB Range Strata Binary -> HTTP service / Bun native SDK
```

当前已完成：

- `range-store-core`：只读存储格式、metadata lookup、LRU handle pool、业务查询 facade。
- `service`：HTTP API、OpenAPI、请求校验、错误码映射、Docker 运行入口。
- `range-store-native`：Bun/Node 进程内 native SDK，复用 core 查询语义。
- `storage-tools`：构建、standalone/cross verify、hot/cold/native/metadata benchmark。
- full cross verify：9 个维度、23,806,716 条源记录，失败数为 0。

剩余工作：

- 完整业务 `line-transition` benchmark：full line 派生 prefix/full 两个节点并量化串行组合耗时。
- Linux x64 `.node`、业务容器和只读 PVC/Kubernetes 挂载验证。
- 最终验收前补边界 case 清单，并按发布目录重跑必要 verify/benchmark。

## 阅读路径

业务接口接入：

1. `api-business-contract.md`
2. `sdk-and-query-chain-explanation.md`
3. `docker-deployment-guide.md`

存储格式和数据流：

1. `range-db-binary-storage-design.md`
2. `proto/README.md`
3. `data-flow-overview.md`
4. `verification_and_benchmark.md`

性能和验收：

1. `binary-vs-sqlite-benchmark-and-verification-report.md`
2. `verification_and_benchmark.md`
3. `roadmap.md`

部署和发布：

1. `docker-deployment-guide.md`
2. `verification_and_benchmark.md`
3. `roadmap.md`

## 模块边界

| 模块 | 职责 | 不做什么 |
| --- | --- | --- |
| `range-store-core` | 存储格式、reader、校验、metadata、查询 facade | HTTP、N-API、CLI 编排 |
| `service` | HTTP API、OpenAPI、错误映射、health/readiness、Docker 入口 | 离线构建、benchmark、native SDK 包装 |
| `range-store-native` | Bun/Node native SDK、直接 payload、`RangeStoreError`、singleton、SDK 测试 | 源 SQLite cross verify、报告生成 |
| `storage-tools` | 构建、验证、benchmark、报告 | 线上服务运行时 |

`service`、`range-store-native` 和 `storage-tools` 不互相依赖业务代码；三者只复用 `range-store-core`。

## 报告口径

- Range Strata Binary benchmark 数字只在 `binary-vs-sqlite-benchmark-and-verification-report.md` 更新。
- Proto V2 benchmark 口径在 `proto/export-and-benchmark.md`，当前运行数字在 `reports/proto-v2-nine-dimension-performance-report.md` 更新。
- verification 覆盖面和命令只在 `verification_and_benchmark.md` 更新。
- API 语义只在 `api-business-contract.md` 更新。
- 生产部署和回滚只在 `docker-deployment-guide.md` 更新。
- 下一步任务只在 `roadmap.md` 更新。

正式报告重跑时，删除同名旧报告后再生成，避免新旧数据混用。只清理目标 `reports/` 文件，不删除 `data/` 下源 SQLite 或 Range Strata 数据。
