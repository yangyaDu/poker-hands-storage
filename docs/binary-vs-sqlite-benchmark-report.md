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

该结果只能说明当前 storage-tools runner 口径下，runtime `meta.db` metadata case 慢于源 SQLite case，不能作为 `.idx/.bin` 二进制策略数据性能结论，也不能直接归因成“SQLite 表结构慢”。

需要注意：

- source 和 runtime 都是 SQLite 表，但不是同一个数据库文件、同一个 schema，也不是同一个进程上下文。
- runtime case 在已经打开 `StoreQueryService`、维护 action schema 懒加载 cache、预热 `.idx/.bin` handle 的进程里执行；source baseline 是单独 SQLite runner。
- 当前 runner 为了兼容 source 的 `depth` 字段和 runtime 的 `drill_depth` 字段，会做 schema 探测；HTTP service 的真实 `/range/drill-scenarios` 查询不会按这个兼容逻辑每次探测字段。
- 已确认 runtime `meta.db` 的唯一索引是 `(drill_name, player_count, drill_depth, abstract_line)`，并不比 source 的 `(drill_name, abstract_line, player_count, depth)` 更差。
- 当前 `CachedMetadataReader` 已改为 key-level lazy metadata path：`concrete_line`、`abstract_line` 和 drill key 首次访问时查 runtime `meta.db`，命中后写入内存 cache；HTTP service 和 native SDK 复用同一 core 实现。后续应重新生成 metadata benchmark，再决定是否还需要额外索引或更强的 prepared statement cache。
- 2026-07-05 复测确认：旧 `data/range-strata/meta.db` 是早期 schema，`concrete_lines_default_9max_100BB` 缺少 `idx_*_concrete_line` 单列索引，`concrete_line = ?` 查询会全表扫描 197,087 行。用临时 indexed meta.db 复测后，同样 1,000 次 Bun 原始 SQLite lookup 从约 27.78s 降到约 9.06ms。fresh build 路径已会创建该索引；旧数据目录应 rebuild 或补索引后再做 metadata lookup 结论。

### 2.3 9max:100BB Native/Core 附注

2026-07-04/05 的 9max:100BB 单维度报告来自：

- SQLite: `reports/benchmark-sqlite-hot-9max-current.json`
- Rust Core: `reports/benchmark-core-hot-9max-current.json`
- Bun Native 旧报告: `reports/benchmark-native-hot-9max-current.json`
- Bun Native 最新公平复测: `reports/benchmark-native-core-direct-sdk-9max-indexed-meta.json`

这些报告使用相同 seed、相同维度和相同 workload 模式。最新公平复测使用 `.tmp/range-strata-indexed-9max-rerun`，该目录复制 `manifest.json/meta.db`，对 `.bin/.idx` 使用硬链接，并只在副本 `meta.db` 上补齐 `concrete_line` 单列索引。`benchmark-native` 当前会生成一次共享 workload JSON，然后把 `core`、`native-direct`、`native-sdk` 拆到独立子进程执行，并按 seed 随机入口顺序；但三组仍共享同一台机器的 OS page cache，所以不能当作严格冷机隔离测试。解读时需要固定以下口径：

- `Rust Core` 指 storage-tools 直接调用 `range_store_core::query::StoreQueryService`，不是 HTTP service。
- `native-direct:*` 指 Bun worker 直接加载 `index.node` 后调用 N-API 类。
- `native-sdk:*` 指 Bun worker 通过 `range-store-native/index.js` 包装层调用同一个 N-API 类，额外返回业务 envelope。
- Native SDK 和 Direct 的策略查询最终仍落到 `RangeStoreFacade -> StoreQueryService`。因此，如果某次 9max 报告显示 SDK/Direct 的 `hand-strategy` 明显快于 Rust Core，不能解释为 SDK 绕过了 Core 算法或拥有更快的数据结构；更合理的解释是 page cache、运行时上下文、计时精度和样本局部性差异。
- `hand-strategy` 与 `batch-size-1` 不是同一个样本集。前者测 1,000 条 `hand_queries` 的单次 API，后者测 200 个一元素 batch 的 batch API sweep；它们都能说明趋势，但不能用来证明“一元素 batch 天然比单次查询快几十倍”。
- `batch-hand-strategy` 是 `--batch-size` 指定的主批量 case；当前默认 batch size 为 20。`batch-size-20` 是 batch-size sweep 中 size=20 的 case。两者语义相同，但采样序列不同，所以数值应接近但不要求完全一致。

当前 `benchmark-native` runner 已改为 `core`、`native-direct`、`native-sdk` 三个独立 worker/子进程执行，避免同一个 Bun 进程同时持有 direct/sdk 两套 store。旧 9max 汇总只能作为观察值，不能作为 SDK 架构优劣的定论。

最新公平复测的入口顺序为 `native-sdk, native-direct, core`，关键 case 如下：

| case | avg | p50 | p95 | QPS | error |
| --- | ---: | ---: | ---: | ---: | ---: |
| `core:hand-strategy` | 0.071 ms | 0.010 ms | 0.455 ms | 14,071 | 0 |
| `native-direct:hand-strategy` | 0.078 ms | 0.015 ms | 0.472 ms | 12,842 | 0 |
| `native-sdk:hand-strategy` | 0.103 ms | 0.019 ms | 0.506 ms | 9,730 | 0 |
| `core:batch-hand-strategy` | 0.962 ms | 0.866 ms | 1.976 ms | 1,039 | 0 |
| `native-direct:batch-hand-strategy` | 1.072 ms | 1.019 ms | 2.223 ms | 933 | 0 |
| `native-sdk:batch-hand-strategy` | 1.244 ms | 1.224 ms | 2.511 ms | 804 | 0 |
| `core:batch-size-20` | 0.085 ms | 0.076 ms | 0.136 ms | 11,824 | 0 |
| `native-direct:batch-size-20` | 0.176 ms | 0.173 ms | 0.267 ms | 5,686 | 0 |
| `native-sdk:batch-size-20` | 0.250 ms | 0.235 ms | 0.420 ms | 3,994 | 0 |
| `core:concrete-lines-exact` | 0.419 ms | 0.378 ms | 0.670 ms | 2,389 | 0 |
| `native-direct:concrete-lines-exact` | 0.431 ms | 0.382 ms | 0.716 ms | 2,320 | 0 |
| `native-sdk:concrete-lines-exact` | 0.616 ms | 0.582 ms | 0.999 ms | 1,624 | 0 |

这说明在同 workload 和独立进程口径下，SDK 没有再出现“明显快于 Core”的反常结论；SDK wrapper 有轻微额外成本，Direct 与 Core 基本同阶。`concrete_line -> concrete_line_id` metadata lookup 在走索引后为 sub-ms，但仍明显慢于旧 eager in-memory map 的 0.01ms 左右，换来的是不再把整张 concrete-line 表预加载进 native worker。

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

> **关键发现**：该历史冷启动报告的 Binary open 瓶颈在 `Service open`（当时会加载 meta.db 中全部 action schemas 到内存）。当前实现已把 action schema cache 改为按 `schema_id` 懒加载，首查只加载命中的 schema；因此本节 open/prewarm 数值需要重新生成后才能代表最新实现。但真正进入策略查询解码后，Binary 明显快于 SQLite 的结论仍成立。

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

### 当前 random workload

| 指标 | SQLite | Binary |
|---|---:|---:|
| Before RSS | 7.79 MiB | 7.39 MiB |
| After RSS | 10.63 MiB | 158.89 MiB |
| **Delta RSS** | **2.84 MiB** | **151.50 MiB** |
| Heap approximation | 6.31 MiB | 8.32 MiB |

> **说明**：Binary 的 RSS 增量较大是因为 mmap 将 .idx/.bin 文件映射到虚拟内存地址空间。当前 random workload 触及更多维度的不同页面，导致 RSS 更高。但这不是传统堆分配，OS 在内存压力时可以回收这些页面。两种模式下堆内存差异很小。

### Bun Native RSS 观察值

`reports/benchmark-native-hot-9max-current.json` 记录的 Bun worker RSS 为：

| 指标 | 值 |
|---|---:|
| Bun worker before native import | 137,871,360 bytes |
| Bun worker after native benchmark | 564,850,688 bytes |
| Delta RSS | 426,979,328 bytes |
| Delta heap used | 5,938,429 bytes |

该旧 delta 不能直接写成“Native SDK 单实例内存增量”。这份 9max 报告生成时，Bun worker 同时构造 `directStore` 和 `sdkStore` 两个 `PokerHandsRange` 实例，并在同一个进程里跑 direct 与 SDK 两组 case；两者各自持有 `RangeStoreFacade`、`StoreQueryService`、metadata/action schema cache 和 mmap handle。该数值还包含 Bun runtime、native module、JIT/FFI 运行时状态，以及 9max 大文件随机访问触达的 mmap 工作集。

当前 runner 已拆成 `native-direct` 和 `native-sdk` 两个 Bun worker，并新增 `core` 独立子进程。每个入口只构造自己的 store，RSS 报告包含 `baseline / after import / after constructor / after warmup / after benchmark`。旧 +407 MB 只能作为旧 benchmark worker 的总 RSS 增量。

`reports/benchmark-native-core-direct-sdk-9max-indexed-meta.json` 的新 RSS 观察值为：

| Phase | Core | Native Direct | Native SDK |
|---|---:|---:|---:|
| Baseline | 15,503,360 bytes | 138,063,872 bytes | 137,949,184 bytes |
| After import | n/a | 138,711,040 bytes | 144,326,656 bytes |
| After constructor | 16,109,568 bytes | 139,464,704 bytes | 145,113,088 bytes |
| After warmup | 40,525,824 bytes | 171,331,584 bytes | 181,178,368 bytes |
| After benchmark | 98,615,296 bytes | 268,541,952 bytes | 297,283,584 bytes |
| Delta | 83,111,936 bytes | 130,478,080 bytes | 159,334,400 bytes |

因此当前更准确的说法是：SDK 单 worker 的完整 9max random benchmark RSS 增量约 151.9 MiB，不是旧报告表面上的 407.2 MiB。After-constructor 绝对 RSS 约 138.4 MiB，warmup 后约 172.8 MiB；完整 benchmark 结束后的更高 RSS 还包含 Bun worker workload、SQLite metadata lookup 的页缓存、mmap 工作集和 native/runtime 状态。action schema cache 已改为懒加载，本次公平复测中三组首查后 `schemaCount=1`，完整 benchmark 结束后 `schemaCount=2868`，不再是首次 miss 加载全表。

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

### 5.3 Hot Benchmark 结果验证（当前 random workload）

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
| **Drill metadata 查询** | 旧 runner 口径下源 SQLite 快于 runtime `meta.db`；当前 core 已切到 key-level lazy cache，需要重新生成 metadata benchmark |
| **灵活性** | SQL 查询，无需专用编解码器 |

### 适用场景建议

- **生产服务（常驻进程）**：策略数据查询推荐 Binary。冷启动仅发生一次，后续批量查询和 `hands-by-actions` 明显优于 SQLite。
- **Drill 高频 metadata 查询**：旧 runner 口径下 runtime `meta.db` 慢于源 SQLite。当前 core metadata path 已改为 key-level lazy SQLite 查询加内存 cache；若该接口成为高频路径，应重新跑隔离 microbenchmark，再评估 prepared statement cache 或额外索引。
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

当前正式 hot benchmark 使用 `random` workload：按数据量加权选择维度，选中维度内随机采样 concrete_line_id。它用于覆盖跨维度随机访问和尾延迟，不声称完全等价于线上业务路径。

`abstract-local` 曾用于早期实验：同一 `abstract_line` 下 concrete_ids 轮转，高度聚集。根据当前业务理解，这不是主要访问模式：实际查询是按用户预演的行动线逐步拼接，只需要推出所有玩家最后一次的具体手牌范围；下注尺度等信息根据具体行动线和位置映射定位。因此 `abstract-local` 后续只保留为非主验收压力场景，不作为性能结论主口径。

后续更贴近业务的 workload 应定义为 `line-transition`：

- 输入完整具体行动线，例如 6 人桌、100BB、2 人对战下的 `F-F-F-R2-F-R7-R15`。
- 业务解释为 `BB vs BTN 4bet` 一类节点。
- 查询前序行动线 `F-F-F-R2-F-R7` 中 BTN 的手牌范围。
- 查询完整行动线 `F-F-F-R2-F-R7-R15` 中 BB 的手牌范围以及 BB 当前可选 actions。
- 行动线到位置、行动者和下注尺度的解释由业务侧位置映射规则决定，不应通过“同一 abstract_line 下 concrete ids 轮转”近似。

> 当前 API 设计仍要求 batch 请求内所有 item 属于同一维度。
