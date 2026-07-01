# Range Strata Binary vs SQLite — 全方位性能对比报告

> **Generated**: 2026-07-01  
> **Platform**: Windows x86_64 (MSVC)  
> **数据集**: 9 个维度 (default × {6,8,9}max × {100,200,300}BB)
> **说明**: 热查询、冷启动对比和全量验证数据均已在 2026-07-01 刷新。

---

## 1. 磁盘占用

| 存储方案 | 总大小 | 明细 |
|---|---:|---|
| **SQLite** (`range.db`) | **1,447.4 MB** | 单文件数据库 |
| **Binary** (`range-strata/`) | **345.5 MB** | `.bin` 259.5 MB + `.idx` 10.9 MB + `meta.db` 75.1 MB + `manifest.json` < 1 KB |

| 指标 | 值 |
|---|---|
| Binary / SQLite 比率 | **23.9%** |
| **空间节省** | **76.1%（-1,101.9 MB）** |

Binary 格式通过紧凑的定长记录和专用索引文件，将 SQLite 的行式存储压缩到不到 1/4。

---

## 2. 热查询性能（mmap 缓存命中）

本节来自 2026-07-01 重新生成的 release hot benchmark：

- Binary: `reports/benchmark-range-strata-binary.*`
- SQLite: `reports/benchmark-sqlite.*`
- Compare: `reports/benchmark-compare.*`
- Workload: `reports/random-workload.json`
- 模式：`random`，seed=42，跨全部 9 个维度按数据量加权采样。
- 结果：Binary 和 SQLite 均为 10 个 case、4,400 次迭代、0 错误、result count 均为 242,179，compare workload compatible=true。

### 2.1 策略数据查询路径

以下 case 读取策略数据。Binary 走 `.idx/.bin` mmap 和 pack 解码；SQLite baseline 走源库 `range_data_*` 表。

| case | SQLite p50 | Binary p50 | SQLite p95 | Binary p95 | SQLite p99 | Binary p99 | Binary/SQLite QPS | result match |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `hand-strategy` | 0.06 ms | 0.01 ms | 0.13 ms | 0.02 ms | 0.22 ms | 0.04 ms | 6.42x | true |
| `batch-hand-strategy` | 1.19 ms | 0.14 ms | 1.83 ms | 0.24 ms | 2.04 ms | 0.45 ms | 8.60x | true |
| `batch-size-1` | 0.07 ms | 0.01 ms | 0.13 ms | 0.01 ms | 0.21 ms | 0.01 ms | 11.52x | true |
| `batch-size-5` | 0.32 ms | 0.03 ms | 0.55 ms | 0.06 ms | 0.67 ms | 0.09 ms | 10.19x | true |
| `batch-size-10` | 0.77 ms | 0.04 ms | 1.40 ms | 0.08 ms | 1.63 ms | 0.12 ms | 16.64x | true |
| `batch-size-20` | 1.53 ms | 0.04 ms | 2.22 ms | 0.06 ms | 2.72 ms | 0.10 ms | 36.22x | true |
| `batch-size-50` | 4.46 ms | 0.26 ms | 6.03 ms | 0.51 ms | 7.36 ms | 0.72 ms | 16.39x | true |
| `batch-size-100` | 6.57 ms | 0.43 ms | 9.23 ms | 0.72 ms | 9.99 ms | 0.91 ms | 15.08x | true |
| `hands-by-actions` | 0.11 ms | 0.01 ms | 0.27 ms | 0.04 ms | 0.37 ms | 0.06 ms | 9.45x | true |

`hands-by-actions` 覆盖“单个行动线下全部起手牌查询”。本次 workload 使用当前业务语义：

- 不传 `frequency` 时默认 `frequency > 0.005`。
- 多个 `action_name` 按 `IN (...)` / OR 语义匹配，命中任意一个 action 即返回该手牌。
- Binary 和 SQLite 的 result count 一致，说明该业务查询在二进制解码路径上与源库结果一致。

### 2.2 Drill Metadata 查询路径

`drill-scenarios-metadata` 覆盖 `/range/drill-scenarios` 高频随机查询，但它不读取 `.idx/.bin` 策略数据：

- SQLite baseline 读取源库 `drill_scenario_lines_*` 表。
- Binary runtime 读取 `data/range-strata/meta.db` 中的 `drill_scenario_lines_*` 表。

| case | SQLite p50 | Binary runtime p50 | SQLite p95 | Binary runtime p95 | SQLite p99 | Binary runtime p99 | Binary/SQLite QPS | result match |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `drill-scenarios-metadata` | 0.08 ms | 1.28 ms | 0.20 ms | 1.81 ms | 0.27 ms | 2.22 ms | 0.07x | true |

该结果只能说明当前 runtime `meta.db` 的 drill metadata 查询比源 SQLite 慢，不能作为 `.idx/.bin` 二进制策略数据性能结论。后续如果 drill 查询会成为高频接口，应优先检查 `meta.db` 上 `drill_scenario_lines_* (drill_name, player_count, drill_depth)` 的索引设计。

---

## 3. 冷启动性能（process-cold, 10 runs/dimension, 跨 9 维度）

本节使用 `--release` 并固定同一查询口径重跑：

```text
concrete_line_id = 1
hand = 22
```

Binary 和 SQLite cold-start 报告均为 9 个维度、每维度 10 次 fresh process 运行，错误数均为 0。

### 3.1 聚合冷启动

| 指标 | SQLite | Binary | 对比 |
|---|---:|---:|---:|
| **Store open + first query P50** | 26.99 ms | 56.68 ms | SQLite 快 2.1x |
| **Store open + first query P95** | 28.36 ms | 64.51 ms | SQLite 快 2.3x |
| **Process elapsed P50** | 45.90 ms | 76.67 ms | SQLite 快 1.7x |
| **Process elapsed P95** | 46.91 ms | 85.80 ms | SQLite 快 1.8x |

### 3.2 阶段分解

| Phase | SQLite P50 | Binary P50 | 说明 |
|---|---:|---:|---|
| Service open (meta.db + schemas) | 10.55 ms | 56.18 ms | Binary 当前会加载全部 action_schemas |
| Dimension prewarm (mmap) | 0.000 ms | 0.495 ms | Binary 需 mmap 映射 .idx/.bin |
| First query decode | 16.48 ms | **0.022 ms** | **Binary 快约 749x** |
| Service close | 0.212 ms | 2.738 ms | Binary 需释放 mmap handles |

> **关键发现**：Binary 冷启动总时间慢于 SQLite，瓶颈在 `Service open`（加载 meta.db 中的 action schemas 到内存）。但真正进入首个策略查询解码后，Binary 明显快于 SQLite。实际业务中服务常驻运行，应把这类 open/prewarm 成本放在容器 ready 之前，而不是放到用户请求里。

### 3.3 各维度冷启动明细

| 维度 | SQLite Store+Query P50 | Binary Store+Query P50 | SQLite Process P50 | Binary Process P50 |
|---|---:|---:|---:|---:|
| default:6:100 | 27.69 ms | 53.11 ms | 45.59 ms | 72.25 ms |
| default:6:200 | 26.99 ms | 56.19 ms | 44.97 ms | 76.11 ms |
| default:6:300 | 27.26 ms | 55.21 ms | 44.44 ms | 74.00 ms |
| default:8:100 | 26.85 ms | 62.71 ms | 45.69 ms | 86.70 ms |
| default:8:200 | 28.07 ms | 63.44 ms | 46.16 ms | 86.24 ms |
| default:8:300 | 28.78 ms | 58.54 ms | 46.62 ms | 80.49 ms |
| default:9:100 | 28.21 ms | 56.92 ms | 45.44 ms | 75.55 ms |
| default:9:200 | 27.71 ms | 56.72 ms | 44.22 ms | 77.38 ms |
| default:9:300 | 25.53 ms | 51.48 ms | 41.71 ms | 69.98 ms |

---

## 4. 运行时内存占用（热查询后，跨 9 维度聚合）

### abstract-local 模式

| 指标 | SQLite | Binary |
|---|---:|---:|
| Before RSS | 10.21 MiB | 7.46 MiB |
| After RSS | 13.19 MiB | 44.82 MiB |
| **Delta RSS** | **2.98 MiB** | **37.36 MiB** |
| Heap approximation | 7.14 MiB | 8.05 MiB |

### random 模式

| 指标 | SQLite | Binary |
|---|---:|---:|
| Before RSS | 7.41 MiB | 7.66 MiB |
| After RSS | 10.18 MiB | 155.35 MiB |
| **Delta RSS** | **2.77 MiB** | **147.69 MiB** |
| Heap approximation | 5.98 MiB | 7.34 MiB |

> **说明**：Binary 的 RSS 增量较大是因为 mmap 将 .idx/.bin 文件映射到虚拟内存地址空间。random 模式触及更多维度的不同页面，导致 RSS 更高。但这不是传统堆分配——OS 在内存压力时自动回收这些页面，不会导致 OOM。两种模式下**堆内存（heap approximation）都在 7-8 MiB**，差异很小。

### 冷启动 RSS 增量（per dimension）

| 维度 | SQLite RSS P95 | Binary RSS P95 |
|---|---:|---:|
| default:6:100 | 582 KB | **38 KB** |
| default:8:100 | 590 KB | **40 KB** |
| default:9:100 | 588 KB | **60 KB** |

---

## 5. 数据完整性验证

### 5.1 Cross Verification（Binary vs SQLite 源数据）

| 指标 | 结果 |
|---|---|
| 维度 | 9 / 9 ✅ |
| Manifest | OK ✅ |
| Catalog | OK ✅ |
| Index Files | 9 / 9 ✅ |
| Pack Files | 9 / 9 ✅ |
| Index-Pack 交叉失败 | **0** |
| Checked Source Records | **23,806,716** |
| Failed Source Records | **0** |
| Extra Binary Records | **0** |
| 精度策略 | `float32-bit-exact` |

### 5.2 各维度验证明细

| 维度 | Index 记录数 | 交叉验证记录 | 失败数 |
|---|---:|---:|---:|
| default:6max:100BB | 3,737 | 194,021 | 0 |
| default:6max:200BB | 2,363 | 142,742 | 0 |
| default:6max:300BB | 1,816 | 114,488 | 0 |
| default:8max:100BB | 8,892 | 398,839 | 0 |
| default:8max:200BB | 5,454 | 283,878 | 0 |
| default:8max:300BB | 3,643 | 225,292 | 0 |
| default:9max:100BB | 197,087 | 7,666,604 | 0 |
| default:9max:200BB | 203,028 | 9,594,303 | 0 |
| default:9max:300BB | 95,114 | 5,186,549 | 0 |

### 5.3 Hot Benchmark 结果验证（abstract-local）

| 指标 | 值 |
|---|---|
| 样本数 | 100 |
| 匹配 | **100** |
| 不匹配 | 0 |
| 错误 | 0 |

### 5.4 Hot Benchmark 结果验证（random）

| 指标 | 值 |
|---|---|
| 样本数 | 100 |
| 匹配 | **100** |
| 不匹配 | 0 |
| 错误 | 0 |

**Binary 与 SQLite 源数据 100% 比特精确一致**，零精度损失。

---

## 6. 总结

### Binary 的优势

| 方面 | 优势 |
|---|---|
| **磁盘占用** | 节省 76%（1,447 MB → 346 MB） |
| **策略数据热查询 QPS（random）** | `hand-strategy` 6.4x；batch-size 1/5/10/20/50/100 为 10.2x-36.2x；`hands-by-actions` 为 9.45x |
| **策略数据热查询 p99** | 单手、批量和 `hands-by-actions` 尾延迟均低于 SQLite |
| **数据精度** | float32 比特精确，零损失 |
| **并发** | mmap + RwLock 无锁并发读，天然适合高并发 |

### SQLite 的优势

| 方面 | 优势 |
|---|---|
| **冷启动** | 当前 process-cold 快约 1.7-1.8x，store+query 快约 2.1-2.3x（无 action_schemas 全量加载和 mmap 映射开销） |
| **RSS 内存** | 运行时 RSS 增量更小（3 MiB vs 37-148 MiB） |
| **Drill metadata 查询** | 当前源 SQLite 快于 runtime `meta.db`，需要单独优化 metadata 索引 |
| **灵活性** | SQL 查询，无需专用编解码器 |

### 适用场景建议

- **生产服务（常驻进程）**：策略数据查询推荐 Binary。冷启动仅发生一次，后续批量查询和 `hands-by-actions` 明显优于 SQLite。
- **Drill 高频 metadata 查询**：当前 runtime `meta.db` 慢于源 SQLite，若该接口成为高频路径，应补充 `(drill_name, player_count, drill_depth)` 组合索引或进一步缓存。
- **一次性脚本/临时查询**：SQLite 更简单，无需额外二进制文件。
- **内存受限环境**：如果物理内存极度有限，SQLite 的 RSS 更可控。但 Binary 的 mmap 由 OS 按需管理，不会导致 OOM。

---

## 附录：测试环境与方法

| 项 | 值 |
|---|---|
| OS | Windows |
| Target | x86_64-pc-windows-msvc |
| Build | release (optimized) |
| Hand iterations | 1,000 |
| Batch iterations | 200 per size |
| Batch sizes | 1, 5, 10, 20, 50, 100 |
| Warmup | 20 iterations |
| Cold start mode | process-cold, 10 runs/dimension |
| SQLite | 动态加载 (`PHS_SQLITE3_LIB`) |

### Workload 模式说明

| 模式 | 维度选择 | concrete_line_id 分布 | 适用场景 |
|---|---|---|---|
| **abstract-local** | 按数据量加权随机选维度 | 同一 abstract_line 下的 concrete_ids 轮转，高度聚集 | 贴近真实用户操作（浏览同一棵线路树） |
| **random** | 按数据量加权随机选维度 | 选中维度内 `[min_id, max_id]` 均匀随机，每个 item 独立采样，concrete_line_id 几乎不重复 | worst-case 压力测试 |

> 两种模式下每个 batch 请求内的 item 都在**同一维度**内（API 设计约束：batch 的维度参数是请求级别的）。
