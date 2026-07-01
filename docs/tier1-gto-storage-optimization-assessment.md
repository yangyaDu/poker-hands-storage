# 档位一：GTO 数据瘦身与查询性能优化验收评估

更新日期：2026-07-01

## 评估结论

当前项目已经完成第二阶段数据瘦身主路径：从字段瘦身后的完整 SQLite 输入库 `data/sqlite/range.db` 生成 Range Strata 运行目录 `data/range-strata`。

当前可复现结果：

| 项目 | 体积 |
| --- | ---: |
| 字段瘦身后的完整 SQLite：`data/sqlite/range.db` | 1,517,748,224 bytes |
| Range Strata 运行目录：`data/range-strata` | 362,296,945 bytes |
| Binary / SQLite 比率 | 23.87% |
| 体积减少 | 76.13% |

结论：

- 数据体积已经进入几百 MB 级别。
- 热查询主路径已经具备，并且批量查询明显优于 SQLite baseline。
- Docker 部署后的 HTTP API 已满足“查询 SDK / 查询接口”中的查询接口要求。
- 全量 cross verify 已通过，源 SQLite 与 Range Strata Binary 当前产物一致。
- 冷启动对比报告已基于同一查询口径重生成。
- 后续还需要补齐缺失业务场景 benchmark、构建工具断点续跑、发布和回滚流程细化。

## 数据口径

本项目的验收口径必须区分两层数据：

| 层级 | 说明 | 是否为当前项目输入 |
| --- | --- | --- |
| 原始完整 rangeDB | 历史原始库，约 4GB，已通过删除或减少字段做过上游 SQLite 瘦身 | 否 |
| 当前完整业务 SQLite | `data/sqlite/range.db`，约 1.45GB，是字段瘦身后的完整业务输入库 | 是 |
| 当前线上运行数据 | `data/range-strata`，由当前 SQLite 构建得到，包含 `manifest.json`、`meta.db`、`.idx/.bin` | 是 |

因此当前项目评估的是：

```text
1.45GB slim SQLite -> 345MB Range Strata Binary
```

原始 4GB 到 1.45GB 的上游字段瘦身不属于当前 Rust workspace 的核心实现范围。如果验收方需要覆盖这段历史，应单独补充上游 SQLite 字段瘦身说明。

## 当前交付物

| 交付物 | 路径 | 状态 |
| --- | --- | --- |
| 存储核心 | `range-store-core` | 已实现 |
| HTTP API 服务 | `service` | 已实现 |
| 离线构建、验证、benchmark 工具 | `storage-tools` | 已实现，需增强 |
| 二进制运行目录 | `data/range-strata` | 已生成 |
| API 契约文档 | `docs/api-business-contract.md` | 已有 |
| 二进制存储设计 | `docs/range-db-binary-storage-design.md` | 已有 |
| 验证说明 | `docs/data-verification-and-format-validation.md` | 已有 |
| Docker 部署说明 | `docs/docker-deployment-guide.md` | 已有 |
| 架构调研 | `docs/storage-architecture-research.md` | 已有 |
| SQLite vs Binary benchmark 总报告 | `docs/binary-vs-sqlite-benchmark-report.md` | 已刷新 cold compare |
| standalone verify 报告 | `reports/range-strata-verify-standalone.*` | 已有 |
| sampled cross verify 报告 | `reports/range-strata-verify-cross.*` | 已有 |
| full cross verify 报告 | `reports/range-strata-verify-cross-full.*` | 已有 |
| hot benchmark 报告 | `reports/benchmark-range-strata-binary.*`、`reports/benchmark-sqlite.*`、`reports/benchmark-compare.*` | 已有 |
| cold benchmark 报告 | `reports/benchmark-cold-start.*`、`reports/benchmark-sqlite-cold-start.*`、`reports/benchmark-cold-compare.*` | 已刷新，同一查询口径 |
| Agent 操作说明 | `.agents/SKILL.md`、`.agents/references/*` | 已有 |

## 需求对照矩阵

| 需求项 | 当前状态 | 证据 | 结论 | 后续动作 |
| --- | --- | --- | --- | --- |
| 新数据体积显著低于当前 SQLite | 已满足 | 1.45GB -> 345MB，约 23.87% | 通过 | 后续报告保持同一数据口径 |
| 数据体积进入几百 MB | 已满足 | `data/range-strata` 约 345.5MB | 通过 | 无 |
| 查询结果与旧数据一致 | 已满足 | full cross verify 通过，失败数 0 | 通过 | 后续发布继续执行全量验证 |
| 全量转换校验 | 已满足 | `reports/range-strata-verify-cross-full.*` | 通过 | 纳入发布流程 |
| 随机抽样校验 | 已满足 | `reports/range-strata-verify-cross.*` sampleSize=10000 | 通过 | 无 |
| 边界 case 校验 | 部分满足 | 单元测试和格式校验覆盖部分边界 | 部分通过 | 在 verify 文档中补边界 case 清单 |
| 数据版本校验 | 已满足基础能力 | `manifest.sourceDbChecksum`、`build_info.source_checksum`、`builtAt` | 通过 | 发布流程补版本目录规范 |
| 数据损坏检测机制 | 已满足 | manifest、idx/bin header、CRC32C、action schema checksum | 通过 | 无 |
| 单个场景 + 单手牌查询 benchmark | 已满足 | `hand-strategy` benchmark | 通过 | 无 |
| 单个行动线下全部起手牌查询 benchmark | API 已有，benchmark 缺失 | `/range/hands-by-actions` 已实现 | 部分通过 | 补 hands-by-actions benchmark |
| Drill 高频随机查询 benchmark | API 已有，benchmark 缺失 | `/range/drill-scenarios` 已实现 | 部分通过 | 补 drill benchmark |
| 批量查询 benchmark | 已满足 | `batch-hand-strategy` 和 batch-size cases | 通过 | 无 |
| P50/P95/P99 查询耗时 | 已满足主路径 | hot benchmark 报告包含 avg/p50/p95/p99/max/qps | 通过 | 新增场景需同样输出 |
| 查询性能不低于 SQLite | 大部分满足，需要明确口径 | 批量和 p95/p99 优势明显；random 单手 p50 SQLite 更低 | 部分通过 | 在总报告中按场景解释 |
| 冷启动查询表现 | 已满足 | cold binary/sqlite/compare 已按同一查询口径刷新 | 通过 | 后续性能变更时重跑 |
| 热缓存查询表现 | 已满足 | hot benchmark 报告 | 通过 | 无 |
| 内存占用对比 | 已满足基础报告 | benchmark 报告包含 RSS 和 heap approximation | 通过 | Docker 内存可后续补 |
| 数据转换工具 | 已满足基础能力 | `storage-tools build` | 通过 | 增强进度和 resume |
| 支持进度输出 | 部分满足 | 当前输出维度 summary，不是持续进度百分比 | 部分通过 | 补 per-dimension progress |
| 支持失败中断后重新执行 | 未完整满足 | 当前 `--overwrite` 可重跑，但无 checkpoint | 未通过 | 实现 `--resume` 和 `build-state.json` |
| 转换后校验 | 已满足 | standalone/cross verify | 通过 | 将全量 verify 纳入发布流程 |
| 查询 SDK / 查询接口 | 已满足查询接口 | Docker HTTP API + OpenAPI/Swagger | 通过 | 不单独做 SDK |
| 明确错误码 | 已满足 | `docs/api-business-contract.md` | 通过 | 行为变更时同步文档 |
| Docker 部署流程 | 已满足 | `.docker/*`、`docs/docker-deployment-guide.md` | 通过 | Docker engine 可用时重跑 smoke |
| 新增数据版本和回滚 | 部分满足 | 文档已有方向，版本目录规范可更具体 | 部分通过 | 补发布/回滚流程 |

## 已满足能力

### 数据瘦身

当前 Range Strata 运行目录包含：

- `manifest.json`
- `meta.db`
- 9 个维度的 `.idx`
- 9 个维度的 `.bin`

`.idx/.bin` 承担高频策略读取，`meta.db` 保留 drill scenario、concrete line 和 action schema 等元数据。相较 slim SQLite，当前运行数据约减少 76.13%。

### 查询接口

当前 Docker 部署后的 HTTP API 是业务侧统一查询接口，不额外提供语言 SDK。

已覆盖：

- `POST /range/hand-strategy`
- `POST /range/hand-strategy-batch`
- `POST /range/hands-by-actions`
- `POST /range/drill-scenarios`
- `POST /range/concrete-lines`
- `POST /range/prewarm`
- `GET /health`
- `GET /ready`

接口契约、请求体、响应体和错误码见 `docs/api-business-contract.md`。

### 正确性机制

当前已具备：

- `manifest.json` 格式和版本检查。
- `meta.db` catalog 检查。
- `.idx` header 和定长记录检查。
- `.bin` header 和 pack 边界检查。
- pack CRC32C。
- action schema CRC32C。
- source DB SHA-256 checksum。
- Float32 bit-exact cross verify。

当前全量 cross verify 报告：

```text
sampleSize = 0
checkedSourceRecords = 23806716
failedSourceRecords = 0
extraBinaryRecords = 0
```

### Benchmark

当前已覆盖：

- Binary hot benchmark。
- SQLite hot baseline。
- Binary vs SQLite hot compare。
- Binary cold benchmark。
- SQLite cold benchmark。
- Binary vs SQLite cold compare。

仍需要补充：

- `hands-by-actions` 查询场景。
- drill 高频随机查询场景。

## 阶段 0-2 已完成项

### 全量 cross verify 报告

已生成：

- `reports/range-strata-verify-cross-full.json`
- `reports/range-strata-verify-cross-full.md`

执行命令：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode cross `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --sample-size 0 `
  --verify-checksum `
  --out reports\range-strata-verify-cross-full.json `
  --md reports\range-strata-verify-cross-full.md
```

结果：

| 指标 | 值 |
| --- | ---: |
| Checked Source Records | 23,806,716 |
| Failed Source Records | 0 |
| Extra Binary Records | 0 |
| Failures | 0 |

### cold compare 报告刷新

已重跑：

- `reports/benchmark-cold-start.*`
- `reports/benchmark-sqlite-cold-start.*`
- `reports/benchmark-cold-compare.*`

本轮发现旧 binary/sqlite cold 报告的查询样本不一致，compare 工具拒绝比较。已使用 `--release` 重跑两份 cold-start 报告，并固定同一查询：

```text
concrete_line_id = 1
hand = 22
```

结果：

| 指标 | SQLite | Binary | 结论 |
| --- | ---: | ---: | --- |
| Store open + first query P50 | 26.99 ms | 56.68 ms | SQLite 快 2.1x |
| Store open + first query P95 | 28.36 ms | 64.51 ms | SQLite 快 2.3x |
| Process elapsed P50 | 45.90 ms | 76.67 ms | SQLite 快 1.7x |
| Process elapsed P95 | 46.91 ms | 85.80 ms | SQLite 快 1.8x |
| First query P95 | 17.36 ms | 0.040 ms | Binary 查询解码更快 |
| Errors | 0 | 0 | 通过 |

## 剩余主要差距

### 业务场景 benchmark 缺失

当前 benchmark 主要覆盖 `hand-strategy` 和 batch。需要新增：

- `hands-by-actions`：单行动线下全部起手牌查询。
- `drill-scenarios`：drill 高频随机查询。

新增场景必须同时实现 binary 和 SQLite baseline，并进入 compare 报告。

### 构建工具缺断点续跑

当前 `build --overwrite` 可以完整重跑，但不能保留已完成维度。建议新增：

- `--resume`
- `build-state.json`
- per-dimension `.tmp` 文件
- 完成后原子 rename
- 重跑时跳过 checksum、size、pack count 均匹配的维度

### 发布和回滚流程还需更具体

建议补充：

- 数据目录命名规范。
- `current` 指针或环境变量切换策略。
- 发布前 verify 阶段。
- `/ready` 验证。
- 回滚到上一版本目录。

## 后续实施顺序

1. 补 `hands-by-actions` benchmark。
2. 补 drill 高频随机 benchmark。
3. 为 `build` 增加 `--resume` 和 `build-state.json`。
4. 补版本发布和回滚说明。
5. 可选做进一步压缩实验。

## 报告清理规则

后续用脚本或命令生成报告时，应避免同一语义下的新旧报告混在一起。

原则：

- 生成正式报告前，删除同名旧报告。
- 保留带明确阶段后缀的历史报告，例如 `*-phase8-smoke.*`、`*-pre-opt.*`，除非本轮任务明确替换它们。
- 新增全量验证报告使用独立文件名：`range-strata-verify-cross-full.*`，不覆盖 sampled cross verify。
- 重跑 cold compare 时，应删除并重建 `benchmark-cold-compare.json` 和 `benchmark-cold-compare.md`。
- 重跑 hot benchmark compare 时，应按同一 workload 同步重建 binary、sqlite、compare 三组报告。
- 删除报告前只删除 `reports/` 下目标文件，不删除 `data/` 下 SQLite 或 Range Strata 数据文件。

建议后续把这些规则落到脚本中，避免手工清理遗漏。

## 通过标准建议

当前项目可对外声明“档位一通过”前，建议至少满足：

1. 本评估文档和现有五份主文档完成同步。
2. 全量 cross verify 通过，失败数为 0。
3. Benchmark 覆盖单手、行动线全部手牌、drill 高频随机、批量查询。
4. Binary 与 SQLite 的性能结论按场景描述，不混用新旧报告。
5. Docker 服务可重建、启动，并且 `/ready` 返回 ready。
6. 数据发布和回滚流程可按文档执行。
