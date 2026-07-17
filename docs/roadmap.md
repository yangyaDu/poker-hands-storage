# 项目 Roadmap 和验收状态

更新日期：2026-07-17

## 文档职责

本文只维护当前剩余工作、优先级和验收条件。已完成能力的命令、数据表和接口细节分别放在专项文档中：

- benchmark 结论：`binary-vs-sqlite-benchmark-and-verification-report.md`
- verification 结论：`verification_and_benchmark.md`
- API 契约：`api-business-contract.md`
- Docker 发布：`docker-deployment-guide.md`
- native SDK：`sdk-and-query-chain-explanation.md`

## 当前判断

Proto V3 首发发布门禁已完成。2026-07-17 的不可变 release root 为
`data/proto-v3-releases/2026-07-17T000001Z`，覆盖
`default_{6,8,9}max_{100,200,300}BB` 九个维度。对应
`reports/v3-release-20260717/release-gate-summary.json` 已记录：9/9 standalone verify 零失败、9/9 SQLite cross verify
零差异，以及 9/9 `correctnessVerified=true` 的 benchmark。

HTTP service 与 Bun/Node native SDK 运行时只读取 V3 目录；线上不依赖源 SQLite、`meta.db`、
`lines.db`、Range Strata Binary 或 Proto V2 产物。V2 和历史 Binary 代码仅保留为参考与回归资产，
不是当前运行时或发布格式；V2 方案已冻结，不再安排功能、性能、benchmark 或兼容性开发。

## 后续工作

### 1. 发布部署与回滚演练

目的：把已通过门禁的 V3 release 接入实际环境，并验证多副本只读运行与 V3 release 间切换。

验收条件：

- service 和 native SDK 均使用只读挂载的版本化 V3 root。
- Linux x64 native SDK 在容器或等效环境完成加载、查询、prewarm 与 readiness 验证。
- 完成从一个已验证 V3 release 切换到另一个已验证 V3 release 的滚动发布/回滚演练；不引入 V2 reader。

### 2. 运行监控与常规发布门禁

目的：让每次 V3 数据发布可观测、可比较、可追溯。

验收条件：

- 发布流水线保存每维 standalone、SQLite cross verify 和 cold/hot benchmark 报告。
- 监控 metadata/strategy cache 的 hit/miss、resident bytes、eviction、打开维度数、查询延迟和 RSS。
- 出现校验失败、格式不兼容或资源门槛回退时阻止切换到新 release。

### 3. 结构性重构

目的：在格式和首发数据稳定后，降低 V3 内部存储模块的维护复杂度。

验收条件：

- 拆分 `metadata_store` 的导出、索引读写、mmap 读取和查询职责。
- 抽取 metadata/strategy 共用的 payload/index writer，并使用具名路径结构与强类型维度 key。
- 重构不改变已发布 V3 磁盘契约，并保持 V3 专属测试、九维验证与 benchmark 门禁通过。

### 4. 冻结的历史资产

Proto V2 已停止开发，不再作为实施方案或优化目标。若未来需要删除历史资产，必须另开独立清理任务，
不影响当前 V3 工作。

当前约束：

- 不新增或改造 V2 功能、性能优化、benchmark、导出格式或兼容路径。
- V2 仅保留为历史参考；任何删除或归档动作单独评审。

## 暂不做

- 不在 V3 runtime 中恢复 SQLite metadata、Range Strata Binary 或 V2 双读/回退路径。
- 不恢复 Proto V2 的实施计划或迭代开发。
- 不复制每个节点的 169 手牌策略 payload。
- 不把 `storage-tools` CLI 放进 HTTP service runtime 镜像。
- 不在当前阶段做 Java、Python、Go 等多语言 SDK。
