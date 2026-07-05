# Poker Hands Storage 文档地图

更新日期：2026-07-05

## 收束原则

`docs` 下文档按职责拆分，不再在多个文件里重复维护同一组命令、数据表或接口定义。

核心规则：

- benchmark 数字只在 `binary-vs-sqlite-benchmark-report.md` 里作为权威结果维护。
- API 请求体、响应体、状态码和错误码只在 `api-business-contract.md` 里维护。
- 二进制文件格式、编码方式和运行目录组成只在 `range-db-binary-storage-design.md` 里维护。
- 验证命令、校验项目和一致性结论只在 `data-verification-and-format-validation.md` 里维护。
- Docker 构建、部署、发布、回滚和 prewarm 内存策略只在 `docker-deployment-guide.md` 里维护。
- `tier1-gto-storage-optimization-assessment.md` 只做验收状态和缺口判断。
- `tier1-gto-storage-optimization-implementation-plan.md` 只做阶段计划、执行状态和下一步队列。

## 当前项目快照

当前项目的主链路是：

```text
1.45GB slim SQLite -> 345.5MB Range Strata Binary -> HTTP service / Bun native SDK
```

当前已完成：

- 存储格式：`manifest.json`、`meta.db`、9 个维度的 `.idx/.bin` 运行目录。
- 运行时：HTTP API、Swagger/OpenAPI、Docker/Compose/Kubernetes 模板、Bun/Node native SDK。
- 离线工具：SQLite 到二进制构建、`build --resume`、standalone/cross verify、hot/cold/native benchmark。
- 数据正确性：full cross verify 覆盖 23,806,716 条源记录，失败数为 0。
- 性能口径：已覆盖 SQLite vs Binary hot/cold、drill metadata、Rust core、Bun native SDK、HTTP service 和 `concrete_line -> handsByActions` 单链路。

当前剩余工作：

- 补完整业务 `line-transition` benchmark：一个 full line 派生 prefix/full 两个查询节点，量化串行组合耗时。
- 验证 Linux `.node` 产物、业务容器和 Kubernetes 只读 PVC 挂载。
- 最终验收前把边界 case 清单文档化，并按发布目录重跑必要 verify/benchmark。

## 文档职责

| 文档 | 职责 | 不应包含 |
| --- | --- | --- |
| `README.md` | 文档地图、职责边界、阅读路径 | 具体 benchmark 数字、完整命令清单 |
| `storage-architecture-research.md` | 方案调研、候选方案对比、语言选择、SDK/API 边界 | 具体冷启动耗时、发布步骤 |
| `range-db-binary-storage-design.md` | Range Strata 文件职责、二进制格式、编码、查询流程、体积组成 | HTTP 状态码、Docker 发布流程 |
| `api-business-contract.md` | HTTP API 业务语义、请求体、响应体、错误码、组合调用方式 | benchmark 结论、底层文件格式细节 |
| `data-verification-and-format-validation.md` | standalone/cross verify、Float32 策略、checksum、发布前验证建议 | 性能结论、接口契约 |
| `binary-vs-sqlite-benchmark-report.md` | SQLite vs Binary 的 hot/cold benchmark、内存、体积和结果一致性摘要 | API 契约、部署 SOP |
| `docker-deployment-guide.md` | 镜像构建、Compose/Kubernetes、版本化数据目录、发布、回滚、prewarm | 存储格式设计细节、benchmark 全量表 |
| `bun-native-sdk-implementation-draft.md` | Bun/TypeScript 进程内 native SDK 方案、N-API 边界、Kubernetes 只读数据挂载建议 | 正式 benchmark 结论 |
| `tier1-gto-storage-optimization-assessment.md` | 对档位一需求做当前状态评估，列剩余缺口 | 完整执行命令、长 benchmark 表 |
| `tier1-gto-storage-optimization-implementation-plan.md` | 小步实施计划、阶段状态、下一步工作 | 已完成阶段的详细报告正文 |

## 推荐阅读路径

业务接口接入：

1. `api-business-contract.md`
2. `bun-native-sdk-implementation-draft.md`
3. `docker-deployment-guide.md`
4. `tier1-gto-storage-optimization-assessment.md`

存储格式和体积判断：

1. `storage-architecture-research.md`
2. `range-db-binary-storage-design.md`
3. `binary-vs-sqlite-benchmark-report.md`

数据发布和验收：

1. `docker-deployment-guide.md`
2. `data-verification-and-format-validation.md`
3. `tier1-gto-storage-optimization-assessment.md`

后续开发排期：

1. `tier1-gto-storage-optimization-implementation-plan.md`
2. `tier1-gto-storage-optimization-assessment.md`

## 报告和数据口径

当前项目评估口径是：

```text
1.45GB slim SQLite -> 345MB Range Strata Binary
```

其中 4GB 原始 rangeDB 到 1.45GB slim SQLite 的字段瘦身属于上游历史过程，不作为当前 Rust workspace 的主要交付范围。

正式报告生成规则：

- 生成正式报告前删除同名旧报告，避免新旧数据混用。
- sampled cross verify 和 full cross verify 使用不同文件名。
- cold compare 重跑时同时重建 binary、sqlite、compare 三组报告。
- hot compare 重跑时保证 binary、sqlite、compare 使用同一 workload。
- 只删除 `reports/` 下目标报告文件，不删除 `data/` 下 SQLite 或 Range Strata 数据。

## 模块边界

当前 workspace 的职责边界：

| 模块 | 职责 |
| --- | --- |
| `range-store-core` | 核心存储格式、读取、校验和查询能力 |
| `range-store-native` | Bun/Node 进程内 native SDK，提供只读查询、prewarm、stats 和轻量启动校验 |
| `service` | HTTP API、OpenAPI、请求校验、错误映射、Docker 运行时入口 |
| `storage-tools` | 离线构建、验证、benchmark、存储方案分析工具 |

`service`、`storage-tools` 和 `range-store-native` 不互相依赖业务代码；三者只复用 `range-store-core` 能力。

## line-transition 边界

业务后端负责解析完整具体行动线、位置映射、当前行动者和前序节点。

本项目第一版只提供两个原子能力：

```text
concrete_line -> concrete_line_id
concrete_line_id + actions + frequency -> hole_cards
```

当前 `benchmark-native` 已覆盖单条 `concrete_line -> concrete_line_id -> handsByActions` 链路，用于量化 metadata lookup 加一次范围查询的成本。它还不是完整业务 `line-transition`：完整链路还需要从一个 full line 派生 prefix/full 两个节点，并分别查询两个节点的手牌范围。

当前不新建树形 range payload `.idx/.bin`。如果完整 `line-transition` benchmark 证明 HTTP 往返或 `concrete_line` lookup 是瓶颈，优先考虑 batch 原子接口或轻量 path index，而不是复制 169 手牌策略数据。
