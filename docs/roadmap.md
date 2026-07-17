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

Proto V3 首发发布门禁和不可变制品闭环已完成。当前 release root 为
`data/proto-v3-releases/2026-07-17T132350Z`，覆盖
`default_{6,8,9}max_{100,200,300}BB` 九个维度。对应
`reports/v3-release-20260717T132350Z/release-gate-summary.json` 已记录：9/9 standalone verify 零失败、9/9 SQLite cross verify
零差异，以及 9/9 `correctnessVerified=true` 的 benchmark。

数据 release 已打包为独立 `tar.zst`，并生成 63 个 payload 的 `SHA256SUMS`、数据包/证据包
SHA-256 和 artifact manifest。数据不内嵌进 npm 包；业务侧解压后把版本化目录作为 N-API SDK 的
`dataDir`。

HTTP service 与 Bun/Node native SDK 运行时只读取 V3 目录；线上不依赖源 SQLite、`meta.db`、
`lines.db`、Range Strata Binary 或 Proto V2 产物。V2 和历史 Binary 代码仅保留为参考与回归资产，
不是当前运行时或发布格式；V2 方案已冻结，不再安排功能、性能、benchmark 或兼容性开发。

## 后续工作

### 1. N-API 包交付与业务接入

目的：把 native SDK 打包成业务 Node/Bun 项目可安装的 N-API 包，并接入已验证的独立 V3 数据制品。

验收条件：

- 为实际业务运行平台生成对应的 `.node` 二进制，并由 npm 包自动加载正确平台产物。
- npm 包只包含 JS/TS wrapper 与 N-API 二进制，不包含 SQLite、V2 或 167 MB V3 数据。
- 业务侧解压并校验 V3 数据包后，通过 `dataDir` 初始化 SDK，完成查询、batch、prewarm 和错误码验收。
- 通过切换业务配置中的版本化 `dataDir` 完成 V3 release 升级与回滚；SDK 本身不要求 Docker。

### 2. 运行监控与常规发布门禁

目的：让每次 V3 数据发布可观测、可比较、可追溯。

验收条件：

- 发布流水线保存每维 standalone、SQLite cross verify 和 cold/hot benchmark 报告。
- 由业务进程采集 SDK 的 metadata/strategy cache hit/miss、resident bytes、eviction、打开维度数、查询延迟和 RSS。
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
- 不为 native SDK 增加 Docker 发布链；SDK 以 N-API npm 包交付。
- 不在当前阶段做 Java、Python、Go 等多语言 SDK。
