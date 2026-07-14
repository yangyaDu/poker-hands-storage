# Proto V2 Cache Miss 与 Decode 优化实践方案

状态：计划，尚未实施  
更新日期：2026-07-14

## 目标与约束

目标是降低真实业务 workload 的 `hand-strategy` 尾延迟和重复解码开销，而不是人为提高随机
workload 的 cache hit rate。当前随机 500 请求样本有 20 次 matrix cache hit、480 次 miss；
这首先说明 matrix 复用度低，不能直接推导出“1024 条 LRU 容量不足”。

以下约束不可破坏：

- Proto V2 schema、`valid_hand_bitmap -> global_compact_index -> action_compact_index` 语义不变。
- `.lmbin + .lmidx` 仍是唯一策略数据事实来源；`lines.db` 仍是 metadata 事实来源。
- `hand_ev IS NULL` 继续在导出时过滤，所有对比继续使用同样的 NULL 过滤与 `x10000` 量化。
- cache hit 路径继续返回 `Arc<DecodedCompactLineMatrix>`，不复制已解码 payload 或紧凑索引。
- 所有候选优化必须同时报告延迟、RSS/估算缓存字节和 Core/SQLite 结果一致性。

## 阶段 0：冻结可比较基线

在现有 `benchmark-three-way-stability` 的固定 workload 上保留原始 JSON/Markdown。至少覆盖：

1. `random` workload：检验随机 line 访问下的真实首次访问比例。
2. `abstract-local` workload：检验同一抽象线附近的复用。
3. 可脱敏的生产 replay：保留请求顺序、dimension、`concrete_line_id` 和 hand，不记录用户信息。
4. 9 个 `default:{6,8,9}:{100,200,300}` 维度；分别报告每维度和汇总结果。

每次基线都记录：机器、构建 profile、`verify_checksums`、matrix cache 容量、max open handles、
workload 文件 hash、样本数和 process-cold/page-cache 限制。

## 阶段 1：先观测，再改 cache 策略

### 1.1 新增观测数据

在 `three_way_stability_benchmark.rs` 的 Proto hand-strategy profile 中加入下列字段：

| 指标 | 计算位置 | 含义 |
| --- | --- | --- |
| `matrix_cache_hit` | `CompactLineMatrixArchive::read_matrix_profiled` | decoded matrix 在当前 handle LRU 中。 |
| `matrix_first_seen_miss` | benchmark workload observer | 本次运行首次访问 `(dimension, concrete_line_id)`。 |
| `matrix_revisit_after_eviction_miss` | benchmark workload observer + reader cache event | 之前访问过但当前 LRU 已不在驻留集。 |
| `dimension_handle_open` / `dimension_handle_eviction` | `HandlePool` | matrix miss 是否由维度 handle LRU 淘汰引起。 |
| `unique_matrix_count` | workload observer | 当前 workload 的不同 `(dimension, concrete_line_id)` 数。 |
| `reuse_distance` | workload observer | 两次相同 matrix 访问之间经过的不同 matrix key 数，输出 P50/P95/max。 |
| `decoded_estimated_bytes` | `DecodedCompactLineMatrix` | matrix、bitmap index、action index 的 Vec 长度/容量估算。 |
| `resident_estimated_bytes` | `SimpleLru` | 当前/峰值 decoded cache 估算字节。 |

“首次访问”和“被淘汰后的重复访问”只能在 benchmark observer 中统计，不应为了这项诊断在生产
reader 中维护无界的全历史 key 集合。observer 的历史只覆盖当前有限 workload。

### 1.2 cache capacity sweep

为 facade 增加显式 open options，至少包含：

```text
max_open_handles
matrix_cache_entry_capacity
matrix_cache_byte_budget
verify_checksums
```

现有公开 `open(...)` 可保留为默认包装；benchmark 使用 `open_with_options(...)`，避免将测试
参数隐藏为常量。对固定 workload 运行以下组合：

```text
entry capacity: 128, 512, 1024, 2048, 4096
byte budget:    unbounded-baseline, 16 MiB, 32 MiB, 64 MiB
```

entry count 只用于兼容与对比；最终生产约束应以 byte budget 为主，因为 9max matrix 的已解码
大小不均匀。每组输出 hit rate、三类 miss、峰值 resident bytes、RSS、matrix read P50/P95、
Protobuf decode P50/P95 以及端到端 facade P50/P95。

### 1.3 决策规则

| 测量结果 | 后续实现 |
| --- | --- |
| `first_seen_miss` 占主导 | 不修改通用 LRU；进入业务级预热。增大容量不会让首次访问命中。 |
| 重复访问存在，且 `revisit_after_eviction_miss` 显著 | 先选择满足 P95 working set 的 byte budget；只有仍存在 scan pollution 时才引入 SLRU。 |
| 少量热点与大量一次性扫描混合 | 采用 probation/protected 两段 LRU；新 entry 先进入 probation，第二次命中后再提升。 |
| dimension handle 淘汰主导 | 调整 `max_open_handles` 或按请求 session 固定 dimension，优先于 matrix cache 算法变更。 |

“显著”必须在报告里给出绝对请求数、占比和对应 P95 改善，不以单独 hit rate 作为上线依据。
建议只有当重复淘汰 miss 至少占该 workload 的 10%，且候选方案能在不超过约定 RSS budget 下
降低 matrix-read P95 时，才承担 SLRU 的实现复杂度。

## 阶段 2：业务级 workset prewarm

如果阶段 1 证明首次访问占主导，优化点应从通用缓存转为已知业务 workset。

### 2.1 API 设计

新增显式、可限额的预热接口，而不是让普通 metadata 查询隐式解码大量 matrix：

```text
prewarm_matrices(dimension, concrete_line_ids, options) -> PrewarmSummary

options:
  max_matrices
  max_decoded_bytes
  stop_on_error
```

`PrewarmSummary` 至少返回 requested、deduplicated、loaded、cache_hits、failed、耗时和估算
缓存字节。输入按 `concrete_line_id` 去重，并受 entry/byte budget 双重限制。

### 2.2 调用边界

- drill 场景已经返回候选 abstract lines 时，由上层解析出即将访问的 concrete lines，再异步预热。
- batch 请求继续依赖现有的按 `concrete_line_id` 分组；不得为单个 batch 同步预热无关 line。
- session/页面切换可在已知下一步 line 集合时预热，但取消或淘汰时无需持久化。
- 不在 `get_concrete_lines`、`get_drill_scenario_lines` 内自动预热，避免 metadata 读取突然产生大 RSS。

### 2.3 验收

对 replay workload 报告“预热前后首个业务请求”与“预热本身”两段耗时；不能把预热耗时藏入
warmup。只有在总成本和 RSS 都在预算内、且用户可感知的首个请求 P95 有改善时，才启用默认
预热策略。

## 阶段 3：Protobuf decode 优化

### 3.1 先细化现有 profile

当前 profile 已分为 index/payload、`CompactLineMatrix::decode` 和紧凑索引构建。下一步将
`DecodedCompactLineMatrix::new` 内部继续拆为 validation、global hand map、per-action map，
并按 action count、valid hand count 和 payload size 分桶。这样可以确认不同 6/8/9max matrix
是否具有不同 decode 尾部来源。

### 3.2 不做的事情

- 不为命中路径重新计算 bitmap rank；当前预建 map 的语义和查询复杂度正确。
- 不把 V2 的 packed `uint32`/`sint32` 改为 fixed-width 字段；这会改变 schema、体积和兼容性。
- 不立刻手写 Protobuf wire decoder。它需要重新承担未知字段、packed varint、边界校验和格式
  演进责任；在 169 手牌规模下，除非测量证明收益足够，否则风险高于收益。

### 3.3 进入 decode accelerator 的门槛

先完成阶段 1/2。只有在已调优 cache/workset 后，仍同时满足以下条件，才评审 accelerator：

1. `protobuf_decode_ms` 在 matrix-read P95 中持续占主要部分；
2. 真实 replay 的首次 matrix 访问仍是端到端 P95 的主要来源；
3. 业务 SLO 无法仅靠预热满足；
4. 已定义可接受的额外磁盘、RSS 和发布校验成本。

候选方案优先级：

1. **进程内 prepared cache**：只对显式 prewarm 的 workset 保留 `Arc<DecodedCompactLineMatrix>`。
   这是现有设计的直接延伸，不增加磁盘格式。
2. **可选派生 sidecar**：只有第一项不足时，引入版本化 `.lmcache`/类似命名的只读预解码索引。
   它必须绑定 source `manifest` 版本、每条 `.lmidx` CRC 和 schema version；不一致时忽略并回退
   到 Protobuf decode。sidecar 不是事实来源，可删除并由导出过程重建。
3. **直接 wire-level decoder**：最后才评审。必须保留与 `prost` decoder 的逐 matrix cross verify，
   并证明其 P95 收益大于代码维护和格式演进成本。

## 实施顺序与验证

1. 实施阶段 1 observer、decoded byte estimate、open options 和 capacity sweep。
2. 在 9 维度的 random/abstract-local/replay workload 上生成报告，选择缓存分支。
3. 若需要，实施 SLRU 或显式 prewarm；不同时实施两者，避免无法归因。
4. 再运行稳定性、冷启动和三方 hot benchmark；确认所有 Core/Proto/SQLite 结果数和错误数一致。
5. 仅在 decoder 门槛满足后，单独撰写 sidecar 或 wire decoder 的格式设计与 rollout/rollback 方案。

每个阶段至少执行：

```powershell
cargo test -p poker-hands-storage-tools
cargo fmt --all -- --check
benchmark-three-way-stability ...
benchmark-three-way-cold ...
```

并更新 [导出与基准](export-and-benchmark.md) 中的报告链接和口径。任何缓存变化不得改变
`QueryResult`、batch 输入顺序、错误码或 bitmap 映射结果。
