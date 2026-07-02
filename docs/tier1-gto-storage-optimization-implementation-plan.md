# 档位一：GTO 数据瘦身与查询性能优化实施方案

更新日期：2026-07-01

## 当前执行状态

| 阶段 | 状态 | 产物 |
| --- | --- | --- |
| 阶段 0：固定基线和报告口径 | 已完成 | sampled/full verify 文件名分离；正式 cold 报告按同名替换 |
| 阶段 1：补全量 cross verify 报告 | 已完成 | `reports/range-strata-verify-cross-full.*` |
| 阶段 2：刷新 cold compare 报告 | 已完成 | `reports/benchmark-cold-start.*`、`reports/benchmark-sqlite-cold-start.*`、`reports/benchmark-cold-compare.*` |
| 阶段 3：补 `hands-by-actions` benchmark | 已完成 | `reports/benchmark-range-strata-binary.*`、`reports/benchmark-sqlite.*`、`reports/benchmark-compare.*` |
| 阶段 4：补 drill 高频随机 metadata benchmark | 已完成 | 同阶段 3 hot benchmark/compare 报告 |
| 阶段 4.5：补真实业务 `line-transition` benchmark | 待实施 | - |
| 阶段 5：实现构建断点续跑 | 待实施 | - |
| 阶段 6：补发布和回滚流程 | 待实施 | - |
| 阶段 7：同步最终验收文档 | 部分完成 | 阶段 0-2 相关文档已同步 |

## 目标

本方案基于 `docs/tier1-gto-storage-optimization-assessment.md` 的评估结论，按小步可验证的方式补齐当前缺口，使项目达到“档位一”可验收状态。

当前项目口径：

```text
data/sqlite/range.db   约 1.45GB slim SQLite 输入
data/range-strata      约 345MB Range Strata Binary 运行目录
```

本阶段不重新覆盖历史 4GB 原始库到 slim SQLite 的上游瘦身过程，只处理当前 Rust workspace 已负责的构建、验证、benchmark、部署和文档闭环。

## 实施原则

- 每一步只解决一个明确缺口。
- 每一步都要有可落地的产物、命令和验收条件。
- 报告生成前删除同名旧报告，避免新旧数据混用。
- 不删除 `data/` 下 SQLite 或 Range Strata 数据文件。
- 涉及代码改动时先补测试，再跑对应测试，最后再跑 workspace 级检查。
- Docker 只作为最终服务验收，不把 `storage-tools` 放进运行镜像。

## 阶段 0：固定基线和报告口径

状态：已完成。

### 目的

先确认当前评估报告、主文档和报告目录的语义一致，避免后续 benchmark 或 verify 结果无法解释。

### 执行动作

1. 保留 `docs/tier1-gto-storage-optimization-assessment.md` 作为当前缺口清单。
2. 保留 sampled cross verify 报告：
   - `reports/range-strata-verify-cross.json`
   - `reports/range-strata-verify-cross.md`
3. 后续全量 cross verify 使用独立文件名：
   - `reports/range-strata-verify-cross-full.json`
   - `reports/range-strata-verify-cross-full.md`
4. 后续 cold compare 重跑时只删除并重建：
   - `reports/benchmark-cold-compare.json`
   - `reports/benchmark-cold-compare.md`

### 验收条件

- 评估报告中数据口径明确为 `1.45GB slim SQLite -> 345MB Range Strata Binary`。
- sampled verify 和 full verify 的文件名不互相覆盖。
- hot、cold、compare 报告的生成时间和输入文件能对应起来。

## 阶段 1：补全量 cross verify 报告

状态：已完成。

已生成：

- `reports/range-strata-verify-cross-full.json`
- `reports/range-strata-verify-cross-full.md`

结果：

| 指标 | 值 |
| --- | ---: |
| Checked Source Records | 23,806,716 |
| Failed Source Records | 0 |
| Extra Binary Records | 0 |
| Failures | 0 |

### 目的

把“查询结果与旧数据一致”从抽样通过推进到全量通过。

### 执行动作

1. 删除同名旧全量报告，如果存在：

```powershell
Remove-Item -LiteralPath reports\range-strata-verify-cross-full.json -ErrorAction SilentlyContinue
Remove-Item -LiteralPath reports\range-strata-verify-cross-full.md -ErrorAction SilentlyContinue
```

2. 执行全量 cross verify：

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

3. 将结果同步到：
   - `docs/data-verification-and-format-validation.md`
   - `docs/tier1-gto-storage-optimization-assessment.md`

### 验收条件

- `failedSourceRecords = 0`
- `extraBinaryRecords = 0`
- checksum 校验通过。
- Markdown 报告能说明：
  - 校验总记录数
  - 成功记录数
  - 失败记录数
  - 失败样例
  - 数据版本和 checksum

### 风险和处理

- 全量校验耗时较长：允许先在当前机器跑一次，后续发布流程固定为必跑项。
- SQLite 源库被占用：先确认没有服务或 benchmark 进程持有 `range.db`。

## 阶段 2：刷新 cold compare 报告

状态：已完成。

本阶段执行时发现旧 binary/sqlite cold 报告的查询样本不一致：

```text
binary: hand=22
sqlite: hand=AA
```

因此已使用 `--release` 重跑两份 cold-start 报告，并固定同一查询：

```text
concrete_line_id = 1
hand = 22
```

已生成：

- `reports/benchmark-cold-start.json`
- `reports/benchmark-cold-start.md`
- `reports/benchmark-sqlite-cold-start.json`
- `reports/benchmark-sqlite-cold-start.md`
- `reports/benchmark-cold-compare.json`
- `reports/benchmark-cold-compare.md`

聚合结果：

| 指标 | SQLite | Binary |
| --- | ---: | ---: |
| Store open + first query P50 | 26.99 ms | 56.68 ms |
| Store open + first query P95 | 28.36 ms | 64.51 ms |
| Process elapsed P50 | 45.90 ms | 76.67 ms |
| Process elapsed P95 | 46.91 ms | 85.80 ms |
| First query P95 | 17.36 ms | 0.040 ms |
| Errors | 0 | 0 |

### 目的

解决当前 cold binary、cold sqlite 和 cold compare 报告时间不一致的问题，避免使用旧 cold compare 数字做结论。

### 执行动作

1. 删除旧 compare 报告：

```powershell
Remove-Item -LiteralPath reports\benchmark-cold-compare.json -ErrorAction SilentlyContinue
Remove-Item -LiteralPath reports\benchmark-cold-compare.md -ErrorAction SilentlyContinue
```

2. 用现有最新 binary/sqlite cold JSON 重新生成 compare：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold-compare `
  --binary reports\benchmark-cold-start.json `
  --sqlite reports\benchmark-sqlite-cold-start.json
```

3. 更新总报告：
   - `docs/binary-vs-sqlite-benchmark-report.md`
   - `docs/tier1-gto-storage-optimization-assessment.md`

### 验收条件

- cold compare 报告的输入文件来自最新 binary cold 和 sqlite cold JSON。
- 报告中区分：
  - process cold start
  - store open + first query
  - first query result count
  - failed runs
- 不把失败样本的 0 值混进 p50/p95/p99。

## 阶段 3：补 `hands-by-actions` benchmark

状态：已完成。

已实现：

- `range-store-core::StoreQueryService::query_hands_by_action_names`
- workload 字段 `handsByActionsQueries`
- Binary hot case `hands-by-actions`
- SQLite baseline case `hands-by-actions`
- compare 报告同名 case

本次 2026-07-01 release benchmark 结果：

| 指标 | SQLite | Binary |
| --- | ---: | ---: |
| iterations | 1,000 | 1,000 |
| p50 | 0.11 ms | 0.01 ms |
| p95 | 0.27 ms | 0.04 ms |
| p99 | 0.37 ms | 0.06 ms |
| QPS | 7,654.75 | 72,340.06 |
| result count | 37,270 | 37,270 |
| errors | 0 | 0 |

结论：Binary 与 SQLite result count 一致，且 `hands-by-actions` 在当前 workload 下 QPS 约为 SQLite 的 9.45x。

### 目的

覆盖“单个行动线下全部起手牌查询”场景，满足验收要求。

### 实现范围

在 `storage-tools` 中新增 benchmark case，不改 HTTP API 业务逻辑。

建议覆盖：

- Binary `hands-by-actions`
- SQLite baseline `hands-by-actions`
- Compare 报告输出

### 实施步骤

1. 扩展 workload 结构，增加 `HandsByActionsBenchmarkItem`。
2. 从 SQLite 中抽取可用样本：
   - `strategy`
   - `player_count`
   - `depth_bb`
   - `concrete_line_id`
   - `action_name` 列表
   - `frequency`
3. Binary runner 通过 `range-store-core` 执行和 API 一致的查询逻辑。
4. SQLite runner 用源 SQLite 表执行等价查询。
5. Metrics 复用现有 avg/p50/p95/p99/max/qps 统计。
6. Compare 报告新增一节：
   - `hands-by-actions`
   - result count
   - latency summary
   - binary/sqlite ratio
7. 补单元测试：
   - workload 生成稳定。
   - binary runner 能返回符合 action `IN (...)` 并集语义的结果。
   - sqlite runner 与 binary result count 一致。
   - compare 能输出该 case。

### 验收条件

- 报告包含 `hands-by-actions` 的 P50/P95/P99。
- action_name 语义保持当前业务定义：
  - 不传 `frequency` 默认过滤 `> 0.005`。
  - 传 `frequency = x` 时过滤 `> x`。
  - 多个 `action_name` 按 SQL `IN (...)` 语义取并集，任意一个 action 满足频率条件即可。
- Binary 和 SQLite 的 result count 一致。

## 阶段 4：补 drill 高频随机 metadata benchmark

状态：已完成。

已实现：

- workload 字段 `drillScenarioQueries`
- Binary runtime metadata case `drill-scenarios-metadata`
- SQLite source metadata case `drill-scenarios-metadata`
- source 表字段兼容 `depth`，runtime `meta.db` 字段兼容 `drill_depth`
- compare 报告同名 case，并标记为 metadata path

本次 2026-07-01 release benchmark 结果：

| 指标 | SQLite source | Runtime meta.db |
| --- | ---: | ---: |
| iterations | 1,000 | 1,000 |
| p50 | 0.08 ms | 1.28 ms |
| p95 | 0.20 ms | 1.81 ms |
| p99 | 0.27 ms | 2.22 ms |
| QPS | 10,594.86 | 778.09 |
| result count | 62,149 | 62,149 |
| errors | 0 | 0 |

结论：结果数量一致，满足一致性验收；但当前 runner 口径下 runtime `meta.db` 的 drill metadata 查询慢于源 SQLite。该 case 不代表 `.idx/.bin` 二进制策略数据查询性能，也不能直接归因为 SQLite 表结构或索引缺失。后续如果 drill metadata 是高频路径，应先补隔离 microbenchmark，再决定是否需要 prepared statement 缓存、schema resolution 缓存、额外索引或 service 层缓存。

### 目的

覆盖 `/range/drill-scenarios` 高频随机查询，满足 Drill 查询验收要求。

该接口走的是 metadata path：

- 旧源库查询 `drill_scenario_lines_*` SQLite 表。
- 新运行目录查询 `data/range-strata/meta.db` 中的 `drill_scenario_lines_*` SQLite 表。
- 不涉及 `.idx/.bin`、mmap pack decode，也不代表二进制策略数据查询性能。

因此阶段 4 的报告定位为 metadata query benchmark 和一致性验证，不作为核心 Binary vs SQLite 存储性能结论。

### 实施步骤

1. 扩展 workload 结构，增加 `DrillScenarioBenchmarkItem`。
2. 从 SQLite 的 drill scenario 表抽样：
   - `strategy`
   - `drill_name`
   - `player_count`
   - `drill_depth`
3. Service/runtime runner 查询运行目录 `meta.db`。
4. SQLite runner 通过源 SQLite 查询对应 drill scenario。
5. Compare 报告新增 `drill-scenarios` 场景，但标题和说明必须标记为 metadata path。
6. 补单元测试：
   - 默认 `drill_name = rfi`
   - 默认 `drill_depth = 100`
   - missing scenario 计入错误，不混进成功 latency。

### 验收条件

- 报告包含 drill 场景 P50/P95/P99。
- `meta.db` 和源 SQLite 的 abstract line 数量一致。
- 默认参数和 Swagger/API 文档一致。
- 总 benchmark 报告中不得把 drill metadata 查询写成 `.idx/.bin` 二进制格式性能优势或劣势。

## 阶段 4.5：补真实业务 `line-transition` benchmark

状态：待实施。

### 目的

当前 `hands-by-actions` benchmark 覆盖的是“单个 concrete line 下按 action/frequency 筛选手牌”。真实业务还有一类更关键的访问模式：根据完整具体行动线推导当前节点和前序节点，然后分别查询不同玩家的范围和当前行动者 actions。

### 业务例子

6 人桌、100BB、2 人对战，完整具体行动线：

```text
F-F-F-R2-F-R7-R15
```

该行动线可解释为 `BB vs BTN 4bet` 节点。业务侧需要：

- 前序行动线 `F-F-F-R2-F-R7` 中 BTN 的手牌范围。
- 完整行动线 `F-F-F-R2-F-R7-R15` 中 BB 的手牌范围。
- 完整行动线中 BB 当前可选 actions。

下注尺度和位置归属通过具体行动线和位置映射规则解析。该模式不是同一 `abstract_line` 下 concrete ids 轮转，因此不能用 `abstract-local` 作为真实业务替代。

### 实施步骤

1. 新增 workload item：`LineTransitionBenchmarkItem`。
2. 从 `concrete_lines_*` 中抽样完整 `concrete_line`，生成：
   - `full_concrete_line`
   - `prefix_concrete_line`
   - `full_concrete_line_id`
   - `prefix_concrete_line_id`
   - 当前行动者位置
   - 前序范围所属位置
3. 增加位置/行动者解析模块，先支持当前 preflop line 语法。
4. Binary runner 执行：
   - prefix line 的 hand range 查询。
   - full line 的 hand range 查询。
   - full line 的 actions 查询。
5. SQLite baseline 执行等价查询。
6. compare 报告新增 `line-transition` case。

### 验收条件

- 报告包含 `line-transition` 的 P50/P95/P99。
- Binary 和 SQLite 的 prefix range result count 一致。
- Binary 和 SQLite 的 full line range/actions result count 一致。
- 文档明确该场景才是主要业务 workload，`abstract-local` 仅保留为非主验收压力场景。

## 阶段 5：实现构建断点续跑

### 目的

满足“失败中断后重新执行”的验收要求，避免构建到中途失败后只能全量重跑。

### 设计

新增：

- `storage-tools build --resume`
- `build-state.json`
- per-dimension 临时文件
- 完成后原子 rename

建议 `build-state.json` 记录：

```json
{
  "source_db": "data/sqlite/range.db",
  "source_checksum": "...",
  "output_dir": "data/range-strata",
  "started_at": "...",
  "updated_at": "...",
  "dimensions": [
    {
      "strategy": "default",
      "table_size": 6,
      "stack_depth": 100,
      "status": "completed",
      "idx_path": "...",
      "bin_path": "...",
      "record_count": 0,
      "pack_count": 0,
      "idx_size": 0,
      "bin_size": 0,
      "checksum": "..."
    }
  ]
}
```

### 实施步骤

1. CLI 增加 `--resume` 参数。
2. Builder 生成维度时先写 `.tmp` 文件。
3. 单个维度成功后写入 state，并把 `.tmp` 原子 rename 成正式文件。
4. `--resume` 模式下，跳过 state 中已完成且文件存在、size/checksum 匹配的维度。
5. 如果 source checksum 不一致，拒绝 resume。
6. 如果参数变更导致输出结构不一致，拒绝 resume。
7. 增加测试：
   - state 文件可写入。
   - 已完成维度会跳过。
   - checksum 不一致会拒绝。
   - 中断后的 `.tmp` 不会被当作正式产物。

### 验收条件

- 构建中断后重新执行 `--resume` 能继续处理未完成维度。
- 完整构建结果与不使用 `--resume` 的结果一致。
- 错误信息能说明为什么不能 resume。

## 阶段 6：补发布和回滚流程

### 目的

让 Docker 部署、数据版本切换和回滚流程可按文档执行。

### 建议目录规范

```text
data/
  range-strata-releases/
    2026-07-01T220000Z/
      manifest.json
      meta.db
      *.idx
      *.bin
    2026-07-02T010000Z/
      manifest.json
      meta.db
      *.idx
      *.bin
  range-strata-current -> range-strata-releases/2026-07-02T010000Z
```

Windows 本地开发如果不使用 symlink，可用环境变量直接切换：

```powershell
$env:PHS_DATA_DIR="C:\Users\Duyang\Desktop\elysia_project\poker-hands-storage\data\range-strata-releases\2026-07-02T010000Z"
```

Docker 部署通过 volume 映射和环境变量切换数据目录。

### 实施步骤

1. 更新 `docs/docker-deployment-guide.md`：
   - 数据版本目录
   - 发布前 verify
   - 启动容器
   - `/ready`
   - 回滚
2. 如果需要，补一个只负责整理发布目录的 storage-tools 子命令：
   - `release prepare`
   - `release validate`
3. Docker engine 可用时执行：

```powershell
docker compose -f .docker\docker-compose.yml build
docker compose -f .docker\docker-compose.yml up -d
```

4. 验证：

```powershell
Invoke-RestMethod http://127.0.0.1:3000/ready
```

### 验收条件

- 新版本发布前必须有 standalone verify 和 cross verify 结果。
- `/ready` 返回 ready 后才对外接流量。
- 回滚只需要切回上一版本数据目录并重启容器。

## 阶段 7：同步最终验收文档

### 目的

把实现结果汇总成可交付材料，而不是只留下命令输出。

### 需要更新的文档

- `docs/tier1-gto-storage-optimization-assessment.md`
- `docs/storage-architecture-research.md`
- `docs/range-db-binary-storage-design.md`
- `docs/api-business-contract.md`
- `docs/data-verification-and-format-validation.md`
- `docs/docker-deployment-guide.md`
- `docs/binary-vs-sqlite-benchmark-report.md`

### 验收条件

- 每份文档只写自己负责的内容。
- benchmark 数字只出现在 benchmark 报告和总报告中。
- 调研报告只写选型估算、方案取舍和风险，不写具体冷启动耗时。
- API 文档和 Swagger 默认值一致。
- 验证文档包含全量 cross verify 结果。

## 最终通过标准

完成以上阶段后，项目可以声明“档位一通过”的条件是：

1. `data/range-strata` 仍保持几百 MB 级别。
2. 全量 cross verify 失败数为 0。
3. benchmark 覆盖：
   - 单手牌查询
   - 单行动线全部手牌查询
   - 真实业务 line-transition 查询
   - drill 高频随机 metadata 查询
   - 批量查询
4. Binary vs SQLite 报告包含 P50/P95/P99、内存、冷启动、热查询。
5. `storage-tools build --resume` 可从中断点继续。
6. Docker 镜像可重建，容器可启动，`/ready` 返回 ready。
7. 发布和回滚流程可按文档执行。

## 建议执行顺序

建议从风险最低、收益最高的项开始：

1. 跑全量 cross verify。已完成。
2. 重生成 cold compare。已完成。
3. 补 `hands-by-actions` benchmark。已完成。
4. 补 drill metadata benchmark。已完成。
5. 补真实业务 `line-transition` benchmark。
6. 实现 `build --resume`。
7. 补发布和回滚文档。
8. 重建 Docker 镜像并启动容器做最终 smoke test。

每完成一项，都先更新对应报告，再更新评估文档状态。
