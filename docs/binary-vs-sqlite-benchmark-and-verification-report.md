# Range Strata Binary vs SQLite — 全方位性能对比报告

> **Generated**: 2026-07-05
> **Platform**: Windows x86_64 (MSVC)  
> **数据集**: 9 个维度 (default × {6,8,9}max × {100,200,300}BB)
> **说明**: 热查询和全量验证数据来自 2026-07-01 全量刷新；冷启动、9max native fair benchmark 和 drill metadata microbenchmark 已在 2026-07-05 使用当前 lazy schema 实现复测。

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

2026-07-05 针对 `default:9:100` 重新做了隔离 microbenchmark：`reports/benchmark-drill-metadata-9max-lazy.json`。该报告只测 drill metadata，不读取策略 `.idx/.bin` 数据。

| path | p50 | p95 | p99 | QPS | error |
| --- | ---: | ---: | ---: | ---: | ---: |
| `drill-raw-sqlite-schema-detect` | 0.88 ms | 1.37 ms | 1.74 ms | 1,050 | 0 |
| `drill-prepared-sqlite` | 0.27 ms | 0.49 ms | 0.68 ms | 3,217 | 0 |
| `drill-cached-metadata` | 0.01 ms | 0.02 ms | 0.04 ms | 97,279 | 0 |

结论：旧 `drill-scenarios-metadata` 慢的主要原因不是“runtime meta.db 用 SQLite 表存储所以慢”，而是 benchmark 旧路径每次做 schema 探测和 SQL prepare。真实 HTTP/native 路径复用 `CachedMetadataReader`，drill key 命中缓存后就是内存 HashMap 查询，9max 复测已经进入 0.01ms 量级。

需要注意：

- source 和 runtime 都是 SQLite 表，但不是同一个数据库文件、同一个 schema，也不是同一个进程上下文。
- runtime case 在已经打开 `StoreQueryService`、维护 action schema 懒加载 cache、预热 `.idx/.bin` handle 的进程里执行；source baseline 是单独 SQLite runner。
- 当前 runner 为了兼容 source 的 `depth` 字段和 runtime 的 `drill_depth` 字段，会做 schema 探测；HTTP service 的真实 `/range/drill-scenarios` 查询不会按这个兼容逻辑每次探测字段。
- 已确认 runtime `meta.db` 的唯一索引是 `(drill_name, player_count, drill_depth, abstract_line)`，并不比 source 的 `(drill_name, abstract_line, player_count, depth)` 更差。
- 当前 `CachedMetadataReader` 已改为 key-level lazy metadata path：`concrete_line`、`abstract_line` 和 drill key 首次访问时查 runtime `meta.db`，命中后写入内存 cache；HTTP service 和 native SDK 复用同一 core 实现。
- 2026-07-05 复测确认：旧 `data/range-strata/meta.db` 是早期 schema，`concrete_lines_default_9max_100BB` 缺少 `idx_*_concrete_line` 单列索引，`concrete_line = ?` 查询会全表扫描 197,087 行。用临时 indexed meta.db 复测后，同样 1,000 次 Bun 原始 SQLite lookup 从约 27.78s 降到约 9.06ms。fresh build 路径已会创建该索引；旧数据目录应 rebuild 或补索引后再做 metadata lookup 结论。

### 2.3 9max:100BB Native/Core 附注

2026-07-04/05 的 9max:100BB 单维度报告来自：

- SQLite: `reports/benchmark-sqlite-hot-9max-current.json`
- Rust Core: `reports/benchmark-core-hot-9max-current.json`
- Bun Native 旧报告: `reports/benchmark-native-hot-9max-current.json`
- Bun Native 最新公平复测: 2026-07-05 9max indexed meta run（正式口径见本节表格）

这些报告使用相同 seed、相同维度和相同 workload 模式。最新公平复测使用 `.tmp/range-strata-indexed-9max-rerun`，该目录复制 `manifest.json/meta.db`，对 `.bin/.idx` 使用硬链接，并只在副本 `meta.db` 上补齐 `concrete_line` 单列索引。`benchmark-native` 当前会生成一次共享 workload JSON，然后把 `core`、`native-sdk`、`http-service` 拆到独立子进程或独立 service 执行，并按 seed 随机入口顺序；但三组仍共享同一台机器的 OS page cache，所以不能当作严格冷机隔离测试。解读时需要固定以下口径：

- `Rust Core` 指 storage-tools 直接调用 `range_store_core::query::StoreQueryService`，不是 HTTP service。
- `native-sdk:*` 指 Bun worker 通过 `range-store-native/index.js` 包装层调用同一个 N-API 类，额外做 JS payload 转换和 `RangeStoreError` 包装。
- `http-service:*` 指 benchmark worker 通过 loopback HTTP 调用单独启动的 `poker-hands-storage-service`，包含 HTTP/JSON 边界成本；其 client worker RSS 不等于 service 进程 RSS。
- Native SDK 的策略查询最终仍落到 `RangeStoreFacade -> StoreQueryService`。因此，如果某次 9max 报告显示 SDK 的 `hand-strategy` 明显快于 Rust Core，不能解释为 SDK 绕过了 Core 算法或拥有更快的数据结构；更合理的解释是 page cache、运行时上下文、计时精度和样本局部性差异。
- `hand-strategy` 与 `batch-size-1` 不是同一个样本集。前者测 1,000 条 `hand_queries` 的单次 API，后者测 200 个一元素 batch 的 batch API sweep；它们都能说明趋势，但不能用来证明“一元素 batch 天然比单次查询快几十倍”。
- `batch-hand-strategy` 是 `--batch-size` 指定的主批量 case；当前默认 batch size 为 20。`batch-size-20` 是 batch-size sweep 中 size=20 的 case。两者语义相同，但采样序列不同，所以数值应接近但不要求完全一致。

当前 `benchmark-native` runner 只保留 `core`、`native-sdk`、`http-service` 三组正式对比。

最新公平复测的关键 case 如下：

| case | avg | p50 | p95 | QPS | error |
| --- | ---: | ---: | ---: | ---: | ---: |
| `core:hand-strategy` | 0.064 ms | 0.012 ms | 0.374 ms | 15,643 | 0 |
| `native-sdk:hand-strategy` | 0.060 ms | 0.013 ms | 0.333 ms | 16,605 | 0 |
| `http-service:hand-strategy` | 1.023 ms | 0.952 ms | 1.584 ms | 978 | 0 |
| `core:batch-hand-strategy` | 1.067 ms | 0.930 ms | 2.633 ms | 937 | 0 |
| `native-sdk:batch-hand-strategy` | 0.977 ms | 0.946 ms | 2.122 ms | 1,024 | 0 |
| `http-service:batch-hand-strategy` | 3.907 ms | 3.944 ms | 5.278 ms | 256 | 0 |
| `core:concrete-lines-exact` | 0.400 ms | 0.351 ms | 0.628 ms | 2,498 | 0 |
| `native-sdk:concrete-lines-exact` | 0.386 ms | 0.345 ms | 0.633 ms | 2,589 | 0 |
| `http-service:concrete-lines-exact` | 1.490 ms | 1.439 ms | 1.992 ms | 671 | 0 |
| `core:drill-scenarios-metadata` | 0.011 ms | 0.008 ms | 0.036 ms | 89,636 | 0 |
| `native-sdk:drill-scenarios-metadata` | 0.013 ms | 0.008 ms | 0.040 ms | 74,334 | 0 |
| `http-service:drill-scenarios-metadata` | 0.939 ms | 0.868 ms | 1.463 ms | 1,065 | 0 |
| `core:line-to-hands-by-actions` | 0.452 ms | 0.403 ms | 0.775 ms | 2,215 | 0 |
| `native-sdk:line-to-hands-by-actions` | 0.426 ms | 0.391 ms | 0.695 ms | 2,349 | 0 |
| `http-service:line-to-hands-by-actions` | 2.377 ms | 2.317 ms | 3.028 ms | 421 | 0 |

这说明在同 workload 和独立进程口径下，Core 与 SDK 基本同阶，小差异不应解释为数据结构差异。HTTP service 稳定慢一档，主要来自 loopback HTTP 和 JSON 序列化边界。`concrete_line -> concrete_line_id` metadata lookup 在走索引后为 sub-ms，但仍明显慢于旧 eager in-memory map 的 0.01ms 左右，换来的是不再把整张 concrete-line 表预加载进 native worker。

`line-to-hands-by-actions` 覆盖的是单条链路：先按 `concrete_line` 精确查 `concrete_line_id`，再用该 id 调 `handsByActions`。它能说明 metadata lookup 加一次 range 查询的运行时成本，但还不是完整业务 `line-transition` benchmark；完整业务链路还需要同一 full line 派生 prefix/full 两个节点，并分别查询两个节点的手牌范围。

---

## 3. 冷启动性能（process-cold, 10 runs/dimension, 跨 9 维度）

2026-07-05 已用当前 lazy action schema 实现重跑全 9 维 cold-start 对照：

- Binary: `reports/benchmark-cold-start.json`
- SQLite: `reports/benchmark-sqlite-cold-start.json`
- Compare: `reports/benchmark-cold-compare.json`
- 口径：`--release`、`process-cold`、每维度 10 runs、共 90 runs/engine。
- 结果：Binary 和 SQLite 错误数均为 0，compare compatible 为 `true`。

固定查询：

```text
concrete_line_id = 1
hand = 22
```

`process-cold` 每次启动 fresh process，但不强制驱逐 OS page cache；因此它衡量的是当前机器上“新进程打开存储并完成首查”的成本，不是严格冷机 I/O 成本。

### 3.1 聚合冷启动

| 指标 | SQLite | Binary | Binary/SQLite |
|---|---:|---:|---:|
| **Store open + first query P50** | 27.80 ms | 14.29 ms | 0.51x |
| **Store open + first query P95** | 30.42 ms | 15.27 ms | 0.50x |
| **Process elapsed P50** | 47.29 ms | 33.57 ms | 0.71x |
| **Process elapsed P95** | 50.59 ms | 35.40 ms | 0.70x |
| **First query P50** | 17.89 ms | 1.249 ms | 0.07x |
| **First query P95** | 19.51 ms | 1.583 ms | 0.08x |

### 3.2 阶段分解

| Phase | SQLite P50 | Binary P50 | 说明 |
|---|---:|---:|---|
| Service open (meta.db + schemas) | 10.02 ms | 11.55 ms | Binary 当前只懒加载必要 schema，open 成本已接近 SQLite |
| Dimension prewarm (mmap) | 0.000 ms | 0.581 ms | SQLite 无 per-dimension mmap prewarm；Binary 打开并 mmap `.idx/.bin` |
| First query sync decode | 17.89 ms | **1.249 ms** | Binary 首查解码约为 SQLite 的 7% |
| Service close | 0.259 ms | 0.211 ms | 两者同阶 |
| Worker measured total | 28.06 ms | 14.58 ms | Binary worker 内部 open+prewarm+query+close 更快 |
| Parent process overhead | 19.11 ms | 19.03 ms | 两边 Rust 进程启动/IPC 成本基本相同 |

解释：旧全维报告里 Binary 冷启动慢，主要因为当时 open 阶段会把 `meta.db` 中全部 `action_schemas` 载入内存。当前实现已改为按命中的 `schema_id` 懒加载，`Service open` 从旧报告的 56ms 量级降到 11.55ms P50；真正的首查阶段仍由 Binary 明显占优。

### 3.3 各维度冷启动明细

| 维度 | SQLite Store+Query P50 | Binary Store+Query P50 | SQLite Process P50 | Binary Process P50 |
|---|---:|---:|---:|---:|
| default:6:100 | 27.37 ms | 12.04 ms | 46.16 ms | 29.27 ms |
| default:6:200 | 27.75 ms | 13.11 ms | 47.51 ms | 33.49 ms |
| default:6:300 | 30.53 ms | 14.09 ms | 51.48 ms | 33.09 ms |
| default:8:100 | 30.48 ms | 13.41 ms | 50.34 ms | 30.48 ms |
| default:8:200 | 29.79 ms | 15.04 ms | 49.25 ms | 34.79 ms |
| default:8:300 | 27.35 ms | 14.03 ms | 48.03 ms | 34.05 ms |
| default:9:100 | 28.28 ms | 15.15 ms | 46.53 ms | 35.86 ms |
| default:9:200 | 27.08 ms | 14.29 ms | 44.94 ms | 33.24 ms |
| default:9:300 | 28.40 ms | 14.26 ms | 46.58 ms | 33.15 ms |

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

该旧 delta 不能直接写成“Native SDK 单实例内存增量”。这份 9max 报告生成时，Bun worker 同时构造多个 `PokerHandsRange` 实例，并在同一个进程里跑多组 case；各实例分别持有 `RangeStoreFacade`、`StoreQueryService`、metadata/action schema cache 和 mmap handle。该数值还包含 Bun runtime、native module、JIT/FFI 运行时状态，以及 9max 大文件随机访问触达的 mmap 工作集。

当前 runner 已拆成 `core`、`native-sdk`、`http-service` 独立 worker/service。每个入口只构造自己的 store，RSS 报告包含 `baseline / after import / after constructor / after warmup / after benchmark`。旧 +407 MB 只能作为旧 benchmark worker 的总 RSS 增量。

2026-07-05 9max indexed meta run 的新 RSS 观察值为：

| Phase | Core | Native SDK |
|---|---:|---:|
| Baseline | 15,908,864 bytes | 138,092,544 bytes |
| After import | n/a | 150,249,472 bytes |
| After constructor | 16,461,824 bytes | 150,982,656 bytes |
| After warmup | 41,005,056 bytes | 187,355,136 bytes |
| After benchmark | 99,033,088 bytes | 309,649,408 bytes |
| Delta | 83,124,224 bytes | 171,556,864 bytes |

HTTP service client worker 的 RSS 为 `baseline=15,953,920 bytes`、`afterBenchmark=17,301,504 bytes`、`delta=1,347,584 bytes`；这只代表压测客户端，不包含被测 service 进程 RSS。

因此当前更准确的说法是：SDK 单 worker 的完整 9max random benchmark RSS 增量约 163.6 MiB，不是旧报告表面上的 407.2 MiB。After-constructor 绝对 RSS 约 144.0 MiB，warmup 后约 178.7 MiB；完整 benchmark 结束后的更高 RSS 还包含 Bun worker workload、SQLite metadata lookup 的页缓存、mmap 工作集和 native/runtime 状态。action schema cache 已改为懒加载，本次公平复测中三组首查后 `schemaCount=1`，完整 benchmark 结束后 `schemaCount=2868`，不再是首次 miss 加载全表。

### 冷启动 RSS 增量（per dimension）

| 维度 | SQLite RSS P95 | Binary RSS P95 |
|---|---:|---:|
| default:6:100 | 576.00 KB | 634.20 KB |
| default:6:200 | 578.20 KB | 596.00 KB |
| default:6:300 | 578.20 KB | 584.00 KB |
| default:8:100 | 580.00 KB | 736.00 KB |
| default:8:200 | 578.20 KB | 666.20 KB |
| default:8:300 | 576.00 KB | 626.20 KB |
| default:9:100 | 576.00 KB | 4.67 MB |
| default:9:200 | 578.20 KB | 4.79 MB |
| default:9:300 | 578.20 KB | 2.53 MB |

这张表来自 2026-07-05 全 9 维 `process-cold` 重跑。9max 维度的 Binary RSS P95 明显更高，主要是 `.idx/.bin` mmap 和首查触达页面更多；6max/8max 维度与 SQLite 同阶。

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
| **冷启动首查** | 当前 lazy schema 实现下，全 9 维 process-cold P50/P95 约为 SQLite 的 0.71x/0.70x，first query P95 约为 SQLite 的 0.08x |
| **数据精度** | float32 比特精确，零损失 |
| **并发** | mmap + RwLock 无锁并发读，天然适合高并发 |

### SQLite 的优势

| 方面 | 优势 |
|---|---|
| **RSS 内存** | 运行时 RSS 增量更小（3 MiB vs 37-148 MiB） |
| **Drill metadata 查询** | 源 SQLite raw/prepared 查询可作为简单脚本基线；旧 runner 慢点已定位为 schema 探测和 SQL prepare 开销 |
| **灵活性** | SQL 查询，无需专用编解码器 |

### 适用场景建议

- **生产服务（常驻进程）**：策略数据查询推荐 Binary。冷启动仅发生一次，后续批量查询和 `hands-by-actions` 明显优于 SQLite。
- **Drill 高频 metadata 查询**：2026-07-05 隔离 microbenchmark 已确认旧 runner 慢点主要来自每次 schema 探测和 SQL prepare。当前 HTTP/native 真实路径复用 `CachedMetadataReader`，drill key 命中缓存后为内存查询；若后续真实业务以 cold metadata miss 为主，再评估 prepared statement cache 或额外索引。
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

当前 `benchmark-native` 已覆盖单条 `concrete_line -> concrete_line_id -> handsByActions` 链路。后续更贴近业务的完整 workload 应定义为 `line-transition`：

- 输入完整具体行动线，例如 6 人桌、100BB、2 人对战下的 `F-F-F-R2-F-R7-R15`。
- 业务解释为 `BB vs BTN 4bet` 一类节点。
- 查询前序行动线 `F-F-F-R2-F-R7` 中 BTN 的手牌范围。
- 查询完整行动线 `F-F-F-R2-F-R7-R15` 中 BB 的手牌范围以及 BB 当前可选 actions。
- 行动线到位置、行动者和下注尺度的解释由业务侧位置映射规则决定，不应通过“同一 abstract_line 下 concrete ids 轮转”近似。

> 当前 API 设计仍要求 batch 请求内所有 item 属于同一维度。
