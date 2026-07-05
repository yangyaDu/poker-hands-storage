# 档位一：GTO 数据瘦身与查询性能优化实施方案

更新日期：2026-07-05

## 文档职责

本文只维护“接下来怎么一步一步做”和“每个阶段当前状态”。已完成阶段的详细命令、报告数据和接口定义不在这里重复展开，避免和专项文档产生漂移。

权威细节见：

- 文档职责边界：`docs/README.md`
- API 契约：`docs/api-business-contract.md`
- 存储格式：`docs/range-db-binary-storage-design.md`
- 验证报告：`docs/data-verification-and-format-validation.md`
- benchmark 结果：`docs/binary-vs-sqlite-benchmark-report.md`
- Docker 发布和回滚：`docs/docker-deployment-guide.md`
- 当前验收状态：`docs/tier1-gto-storage-optimization-assessment.md`

## 当前执行状态

| 阶段 | 状态 | 当前产物或权威文档 |
| --- | --- | --- |
| 阶段 0：固定基线和报告口径 | 已完成 | `docs/README.md`、`reports/*` 命名规则 |
| 阶段 1：补全量 cross verify 报告 | 已完成 | `docs/data-verification-and-format-validation.md` |
| 阶段 2：刷新 cold compare 报告 | 已完成 | `docs/binary-vs-sqlite-benchmark-report.md` |
| 阶段 3：补 `hands-by-actions` benchmark | 已完成 | `docs/binary-vs-sqlite-benchmark-report.md` |
| 阶段 4：补 drill 高频随机 metadata benchmark | 已完成 | 已补隔离 microbenchmark，旧慢点主要来自 schema 探测和 SQL prepare |
| 阶段 4.4：收束具体行动线 lookup 原子接口 | 已完成 | `docs/api-business-contract.md` |
| 阶段 4.5：补真实业务 `line-transition` 访问链路 benchmark | 部分完成 | 已覆盖 `concrete_line -> handsByActions` 单链路；待补 prefix/full 双节点链路 |
| 阶段 5：实现构建断点续跑 | 已完成 | `storage-tools build --resume`、`build-state.json` |
| 阶段 6：补发布和回滚流程 | 已完成 | `docs/docker-deployment-guide.md` |
| 阶段 7：同步最终验收文档 | 进行中 | 当前文档已同步到 2026-07-05 进展 |
| 阶段 8：Bun native 生产接入验证 | 待实施 | Linux `.node`、业务容器、只读 PVC/Kubernetes 验证 |

## 当前目标

当前项目只评估这条链路：

```text
1.45GB slim SQLite -> 345MB Range Strata Binary -> Docker HTTP API
```

不把历史 4GB 原始库到 1.45GB slim SQLite 的上游字段瘦身过程计入当前 Rust workspace 的核心交付。

本阶段目标是让以下内容形成闭环：

- 数据转换可重复执行，并支持中断后继续。
- standalone/cross verify 能证明二进制格式和 SQLite 源数据一致。
- benchmark 能分别说明 hot、cold、metadata、策略数据和真实业务访问链路。
- Docker 部署能通过 `/ready` 和 smoke 查询验证。
- Bun native SDK 能在业务进程内复用 core 查询语义，并与 HTTP service 保持结果一致。
- 文档职责清晰，避免同一结论在多处重复维护。

## 实施原则

- 一次只补一个可验证缺口。
- 新报告生成前先清理同名旧报告。
- 数据目录使用新版本目录发布，不原地覆盖运行中的 `.idx/.bin`。
- benchmark 结论按场景写，不把 metadata SQLite 查询结果混成 `.idx/.bin` 策略查询结论。
- 文档只在权威归属文件里维护细节，其他文档只引用或摘要。

## line-transition 实现边界

本项目第一版不实现新的决策树 range payload 存储，也不构建树形 `.idx/.bin` 来替代现有维度文件。

当前职责划分：

```text
业务后端:
  解析 full_line / prefix_line
  根据位置映射解释 BTN、BB、3bet、4bet、下注尺度
  决定需要查询哪些节点

本项目:
  /range/concrete-lines:
    concrete_line -> concrete_line_id

  /range/hands-by-actions:
    concrete_line_id + actions + frequency -> hole_cards
```

典型访问链路：

```text
prefix_line = F-F-F-R2-F-R7
full_line   = F-F-F-R2-F-R7-R15

prefix_id = /range/concrete-lines(prefix_line)
full_id   = /range/concrete-lines(full_line)

BTN range = /range/hands-by-actions(prefix_id, actions=[], frequency=0.005)
BB range  = /range/hands-by-actions(full_id, actions=[], frequency=0.005)
```

同一维度下连续查询通常能复用 handle pool、action schema 和 OS page cache。更可能的额外成本是多次 HTTP/JSON 往返，而不是 `.idx/.bin` 随机查询本身。

后续优化优先级：

1. 先补完整 `line-transition` benchmark，用真实 prefix/full 双节点访问链路量化延迟。
2. 如果主要成本来自 HTTP 往返，优先做 batch 原子接口。
3. 如果主要成本来自 `concrete_line -> concrete_line_id`，再评估轻量 path index。
4. 不复制每个节点的 169 手牌策略 payload。

## 下一步实施队列

### 1. 阶段 4.5：完整业务 `line-transition` benchmark

目的：覆盖用户预演行动线逐步拼接的真实访问模式，而不是使用同一 `abstract_line` 下的 concrete ids 轮转来近似。

当前状态：`benchmark-native` 已覆盖单条 `concrete_line -> concrete_line_id -> handsByActions` 链路，并同时测 core、native-direct、native-sdk、HTTP service。仍缺完整业务 prefix/full 双节点组合链路。

建议实现：

- 从 `meta.db` 中抽取真实 concrete line 样本。
- 对每条 full line 派生 prefix line。
- 分别测量：
  - `concrete_line -> concrete_line_id`
  - `hands-by-actions(prefix_id)`
  - `hands-by-actions(full_id)`
  - 串行组合耗时
- 输出 p50、p95、p99、错误数、result count。
- Binary 和 SQLite baseline 使用同一批 full/prefix line 样本。

验收：

- 报告进入 `reports/`。
- 结论同步到 `docs/binary-vs-sqlite-benchmark-report.md` 和 `docs/tier1-gto-storage-optimization-assessment.md`。
- 明确是否需要 batch 接口或 path index。

### 2. Bun native 生产接入验证

目的：确认生产形态不是只在 Windows 本地可用，而是能在业务后端容器中稳定加载 native addon 并读取只读 RangeDB。

建议实现：

- 构建 Linux x64 `.node` 产物。
- 在业务后端镜像或等效 smoke 容器中加载 `range-store-native/index.js`。
- 通过只读目录挂载验证 `PokerHandsRange` constructor、`stats`、`prewarm`、`getConcreteLines`、`handsByActions`。
- 在 Kubernetes 或等效环境验证只读 PVC 挂载、多副本读取和 readiness。

验收：

- 容器内 SDK smoke 通过。
- 多副本只读读取同一数据目录通过。
- readiness 只在 native store 打开并完成必要 prewarm 后通过。

### 3. 边界 case 清单和最终验收文档刷新

目的：阶段 4.5、native 生产接入验证和边界 case 清单完成后，更新最终验收状态。

需要同步：

- `docs/binary-vs-sqlite-benchmark-report.md`
- `docs/tier1-gto-storage-optimization-assessment.md`
- 必要时更新 `docs/api-business-contract.md`

## 已完成阶段保留说明

阶段 0-6 的详细命令不在本文重复维护：

- 全量 verify 命令和结果在 `data-verification-and-format-validation.md`。
- hot/cold benchmark 数字在 `binary-vs-sqlite-benchmark-report.md`。
- `build --resume` 的发布使用方式在 `docker-deployment-guide.md`。
- API 请求体、默认值和错误码在 `api-business-contract.md`。

## 最终通过标准

当前项目达到可验收状态时，应满足：

1. Range Strata 运行目录显著小于 slim SQLite，并保持几百 MB 级别。
2. full cross verify 失败数为 0。
3. 策略数据 hot query 不慢于 SQLite，且 benchmark 覆盖单手、批量、`hands-by-actions`。
4. cold-start 报告明确区分 open/prewarm 成本和首个查询成本。
5. Docker 镜像可构建，容器可启动，`/ready` 和 smoke 查询通过。
6. Bun native SDK 在 Linux 业务容器内可加载，并能通过只读数据挂载完成核心 smoke。
7. 发布使用版本化数据目录，支持回滚旧目录。
8. 文档职责清晰，专项细节只有一个权威位置。
