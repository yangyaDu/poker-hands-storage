# 项目 Roadmap 和验收状态

更新日期：2026-07-05

## 文档职责

本文只维护当前剩余工作、优先级和验收条件。已完成能力的命令、数据表和接口细节分别放在专项文档中：

- benchmark 结论：`binary-vs-sqlite-benchmark-and-verification-report.md`
- verification 结论：`data-verification-and-format-validation.md`
- API 契约：`api-business-contract.md`
- Docker 发布：`docker-deployment-guide.md`
- native SDK：`native-sdk.md`

## 当前判断

项目已经具备档位一核心能力：

- 345.5MB Range Strata Binary 运行目录。
- HTTP service 和 Bun native SDK 两条查询入口。
- 离线 build、resume、standalone verify、cross verify 和 benchmark 工具。
- Docker HTTP service 构建、运行、health/readiness 和只读数据挂载文档。

最终对外验收前还需要补三类工作。

## 1. 完整业务 line-transition benchmark

目的：验证真实业务里一个完整行动线拆成 prefix/full 两个查询节点后的组合耗时。

当前已覆盖：

- 单个 `concrete_line_id` 下的 `handsByActions`。
- `concrete_line -> concrete_line_id -> handsByActions` 单链路。

仍缺：

- 从真实 full concrete line 派生 prefix line。
- 对同一批样本分别测量 prefix 节点和 full 节点查询。
- 输出串行组合耗时。

验收条件：

- 生成正式 `reports/` JSON 和 Markdown。
- 输出 p50、p95、p99、错误数和 result count。
- 明确主要成本来自 HTTP 往返、metadata lookup 还是 `.idx/.bin` 查询。
- 根据结果决定是否需要 batch 原子接口或轻量 path index。

## 2. Bun native Linux 生产接入验证

目的：确认 native SDK 不是只在 Windows 本地可用，而是能在业务容器或等效 Linux 环境中稳定运行。

验收条件：

- 构建 Linux x64 `.node` 产物。
- 在容器内加载 `range-store-native/index.js`。
- 用只读数据目录验证 `PokerHandsRange` constructor、`stats`、`prewarm`、`getConcreteLines`、`handsByActions`。
- 验证多副本只读读取同一数据目录。
- readiness 在 native store 打开并完成必要 prewarm 后才通过。

## 3. 最终验收边界清单

目的：把发布前必须复核的边界 case 文档化，避免只依赖零散测试记忆。

建议覆盖：

- 空 action 列表。
- 多 action OR 过滤。
- `frequency` 缺省值、0、边界值和非法值。
- 不存在的 concrete line。
- 不存在的 hand 或非法 hand。
- pack checksum mismatch。
- manifest 版本不兼容。
- 缺失 `.idx/.bin` 或 `meta.db`。

验收条件：

- 边界 case 清单同步到 `data-verification-and-format-validation.md` 或 `api-business-contract.md` 的对应章节。
- 发布目录重跑必要 standalone/cross verify。
- 重跑必要 benchmark 并刷新报告日期和输入数据说明。

## 暂不做

- 不恢复已删除的额外 N-API 直连 benchmark 或 verification 入口。
- 不复制每个节点的 169 手牌策略 payload。
- 不把 `storage-tools` 放进 HTTP service runtime 镜像。
- 不在当前阶段做 Java、Python、Go 等多语言 SDK。
