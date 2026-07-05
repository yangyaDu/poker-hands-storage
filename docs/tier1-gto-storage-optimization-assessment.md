# 档位一：GTO 数据瘦身与查询性能优化验收评估

更新日期：2026-07-05

## 文档职责

本文只回答两个问题：

1. 当前项目距离“档位一”验收还差什么。
2. 每个需求项当前是已满足、部分满足还是待补齐。

具体命令、完整 benchmark 表、API 请求体和部署 SOP 不在本文重复维护。权威文档见 `docs/README.md`。

## 评估结论

当前项目已经完成核心路径：

```text
1.45GB slim SQLite -> 345.5MB Range Strata Binary -> Docker HTTP API / Bun native SDK
```

当前可确认：

- 数据体积已进入几百 MB 级别。
- full cross verify 已通过，失败数为 0。
- `hand-strategy`、batch、`hands-by-actions` 等策略数据 hot query 已明显优于 SQLite baseline。
- cold-start 报告已按同一查询口径比较 Binary 和 SQLite。
- Docker 服务、`/ready`、版本化数据目录、发布和回滚流程已具备。
- `storage-tools build --resume` 和 `build-state.json` 已补齐构建中断后继续能力。
- Bun/Node native SDK 已具备当前核心查询能力，并已有 core / native-direct / native-sdk / HTTP service fair benchmark。
- Drill metadata 隔离 microbenchmark 已完成，旧慢点主要定位为 schema 探测和 SQL prepare 开销；真实 HTTP/native 路径走 `CachedMetadataReader` key-level lazy cache。

仍需补齐或复核：

- 完整业务 `line-transition` 访问链路 benchmark：当前已有 `concrete_line -> handsByActions` 单链路，还缺 prefix/full 双节点组合链路。
- Linux `.node` 产物、业务容器和 Kubernetes 只读 PVC 挂载验证。
- 边界 case 验证清单可继续细化。

## 数据口径

| 层级 | 说明 | 是否为当前项目输入 |
| --- | --- | --- |
| 原始完整 rangeDB | 历史原始库，约 4GB，已做过上游字段瘦身 | 否 |
| 当前完整业务 SQLite | `data/sqlite/range.db`，约 1.45GB，字段瘦身后的完整业务输入库 | 是 |
| 当前线上运行数据 | `data/range-strata`，包含 `manifest.json`、`meta.db`、`.idx/.bin` | 是 |

当前验收只评估 `1.45GB slim SQLite -> 345MB Range Strata Binary`，不把历史 4GB 到 1.45GB 的上游过程算作本仓库交付。

体积和组成的权威数字见 `docs/range-db-binary-storage-design.md` 和 `docs/binary-vs-sqlite-benchmark-report.md`。

## 交付物状态

| 交付物 | 路径 | 状态 |
| --- | --- | --- |
| 存储核心 | `range-store-core` | 已实现 |
| HTTP API 服务 | `service` | 已实现 |
| 离线构建、验证、benchmark 工具 | `storage-tools` | 已实现 |
| 构建断点续跑 | `storage-tools build --resume`、`build-state.json` | 已实现 |
| 二进制运行目录 | `data/range-strata` | 已生成 |
| API 契约 | `docs/api-business-contract.md` | 已有 |
| 存储格式设计 | `docs/range-db-binary-storage-design.md` | 已有 |
| 验证说明 | `docs/data-verification-and-format-validation.md` | 已有 |
| Benchmark 总报告 | `docs/binary-vs-sqlite-benchmark-report.md` | 已有 |
| Docker 部署和回滚 | `docs/docker-deployment-guide.md` | 已有 |
| 架构调研 | `docs/storage-architecture-research.md` | 已有 |
| 实施计划 | `docs/tier1-gto-storage-optimization-implementation-plan.md` | 已收束 |
| Agent 操作说明 | `.agents/SKILL.md`、`.agents/references/*` | 已有 |

## 需求对照矩阵

| 需求项 | 当前状态 | 证据文档 | 后续动作 |
| --- | --- | --- | --- |
| 新数据体积显著低于 SQLite | 已满足 | `range-db-binary-storage-design.md` | 后续保持同一数据口径 |
| 数据体积进入几百 MB | 已满足 | `binary-vs-sqlite-benchmark-report.md` | 无 |
| 查询结果与旧数据一致 | 已满足 | `data-verification-and-format-validation.md` | 发布前继续 full cross verify |
| 全量转换校验 | 已满足 | `reports/range-strata-verify-cross-full.*` | 纳入发布流程 |
| 随机抽样校验 | 已满足 | `reports/range-strata-verify-cross.*` | 无 |
| 边界 case 校验 | 部分满足 | 单元测试、standalone verify、cross verify | 补更明确的边界 case 清单 |
| 数据版本校验 | 已满足基础能力 | `manifest.json`、`build_info`、source checksum | 发布时核对版本目录 |
| 数据损坏检测机制 | 已满足 | manifest、header、CRC32C、action schema checksum | 无 |
| 单个场景 + 单手牌查询 benchmark | 已满足 | `binary-vs-sqlite-benchmark-report.md` | 无 |
| 单个行动线下全部起手牌查询 benchmark | 已满足 | `hands-by-actions` case | 无 |
| Drill 高频随机 metadata benchmark | 已满足，性能原因已复核 | `drill-scenarios-metadata` case；`benchmark-drill-metadata` 隔离 raw/prepared/cached 三组路径 | 无 |
| 批量查询 benchmark | 已满足 | batch cases | 无 |
| P50/P95/P99 查询耗时 | 已满足 | benchmark 报告 | 无 |
| 查询性能不低于 SQLite | 策略数据路径满足，metadata 路径需单独解释 | benchmark 报告 | 用 microbenchmark 复核 metadata |
| 冷启动查询表现 | 已满足 | cold compare 报告 | 性能变更后重跑 |
| 热缓存查询表现 | 已满足 | hot benchmark 报告 | 无 |
| 内存占用对比 | 已满足基础报告 | benchmark 报告、Docker 文档 | 生产资源按部署环境复测 |
| 数据转换工具 | 已满足 | `storage-tools build` | 无 |
| 支持进度输出 | 部分满足 | 当前有维度级输出 | 可后续补百分比进度 |
| 支持失败中断后重新执行 | 已满足 | `--resume`、`build-state.json` | 发布流程继续使用版本目录 |
| 转换后校验 | 已满足 | standalone/cross verify | 无 |
| 查询 SDK / 查询接口 | 已满足 HTTP 和 Bun native 当前核心能力 | Docker HTTP API、OpenAPI、`range-store-native`、API 文档、native benchmark | Linux 生产产物和业务容器验证 |
| 明确错误码 | 已满足 | `api-business-contract.md` | 行为变更时同步 |
| Docker 部署流程 | 已满足 | `docker-deployment-guide.md` | 生产环境按资源重测 |
| 新增数据版本和回滚 | 已满足文档流程 | `docker-deployment-guide.md` | 实际发布时演练 |

## 剩余缺口

### 完整业务 `line-transition` benchmark

当前 benchmark 已覆盖两层原子能力：

- 单个 `concrete_line_id` 下的 `hands-by-actions`。
- `concrete_line -> concrete_line_id -> handsByActions` 单链路。

还没有覆盖业务后端按完整行动线拆 prefix/full 节点后的组合访问链路，例如先查前序节点 BTN range，再查当前节点 BB range。

该缺口不影响现有 API 正确性，但会影响我们判断是否需要 batch 接口或轻量 path index。

实施计划见 `docs/tier1-gto-storage-optimization-implementation-plan.md`。

### Native 生产接入验证

当前 native SDK 已在 Windows MSVC 本地完成核心功能、SDK contract、HTTP consistency 测试入口和 fair benchmark。生产接入前还需要：

- 构建 Linux x64 `.node` 产物。
- 在业务后端容器内验证 Bun 能稳定加载 native addon。
- 用只读 PVC 或等效只读数据挂载验证多副本读取。
- 将 native store singleton 纳入业务 readiness。

### 边界 case 清单

当前格式校验、单元测试和 cross verify 已覆盖大量错误场景，但验收文档层面还可以继续补明确清单，例如：

- 空 action 列表。
- 多 action OR 过滤。
- `frequency` 缺省值和边界值。
- 不存在的 concrete line。
- pack checksum mismatch。
- manifest 版本不兼容。

## 当前通过判断

按原始“档位一”要求，当前项目已经满足核心验收项：体积、正确性、查询接口、转换工具、验证机制、benchmark 和 Docker 文档均已具备。

更严格地按真实业务接入视角看，建议在对外最终验收前补完：

1. 完整 `line-transition` prefix/full 双节点访问链路 benchmark。
2. Linux native SDK 和业务容器只读数据挂载验证。
3. 边界 case 清单的文档化。

完成后即可把本文状态更新为“档位一通过”。
