# Rust Benchmark 7c Implementation Plan

## 目标

实现 Rust 版热路径 benchmark，并接入 `--verify-results`，覆盖上游 `benchmark` 的核心能力：

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark \
  --dir data/range-strata \
  --source data/sqlite/range.db \
  --verify-results
```

这一步只做 7c：

- 热路径 benchmark
- workload 生成和读取
- warmup
- 单手查询、batch 查询、多 batch size 查询
- QPS、avg、p50、p95、p99、max、errorCount、resultCount
- RSS / heap 或 Rust 侧内存近似指标
- JSON / Markdown report
- `--verify-results` 抽样校验

不在本阶段实现：

- 7d `benchmark-cold`
- 7e `benchmark compare`
- 硬性能阈值或自动回归失败策略
- HTTP 路由 benchmark

## 下一步顺序

1. 7c：实现 `benchmark` hot path + `--verify-results`。
2. 7d：实现 `benchmark-cold`，独立做 process cold / OS best effort / linux drop cache 和 phase accounting。
3. 7e：实现 `benchmark compare`，支持 SQLite vs binary、Rust current vs baseline、TS/Bun vs Rust report。
4. 结构后续清理：再拆分仍然偏大的 `build_orchestrator.rs`、`sqlite/mod.rs`、verification report/runner 文件。

当前优先级选择 7c，因为 verifier 已经证明数据正确，下一条发布链路需要证明查询链路正确且性能可观测。

## 上游参考

只读参考同级仓库：

- `E:\idea_project\preflop-storage\src\range-strata-binary\cli\benchmark.ts`
- `E:\idea_project\preflop-storage\src\range-strata-binary\benchmark\runner.ts`
- `E:\idea_project\preflop-storage\src\benchmark\common.ts`

Rust 实现应保持报告字段和 workload JSON 尽量兼容上游，方便后续 7e 做 TS/Bun report vs Rust report。

## CLI 合约

新增命令：

```text
poker-hands-storage-service benchmark
  --dir <range-strata-dir>
  --source <range.db>
  [--meta <meta.db>]
  [--out <report.json>]
  [--md <report.md>]
  [--workload <workload.json>]
  [--seed <number>]
  [--iterations <number>]
  [--hand-iterations <number>]
  [--batch-iterations <number>]
  [--batch-size <number>]
  [--batch-sizes <list>]
  [--dimension <strategy:players:bb>]
  [--workload-mode <random|abstract-local>]
  [--warmup-iterations <number>]
  [--verify-checksum]
  [--verify-results]
```

默认值：

| Option | Default |
| --- | --- |
| `--dir` | required |
| `--source` | required, unless `--workload` is used only for dry parsing tests |
| `--meta` | `<dir>/meta.db` |
| `--out` | `reports/benchmark-range-strata-binary.json` |
| `--md` | `reports/benchmark-range-strata-binary.md` |
| `--seed` | `42` |
| `--iterations` | `1000` |
| `--hand-iterations` | `--iterations` |
| `--batch-iterations` | `min(--iterations, 200)` |
| `--batch-size` | `20` |
| `--batch-sizes` | `1,5,10,50,100` |
| `--workload-mode` | `random` |
| `--warmup-iterations` | `20` |

`--batch-size` 必须自动并入 `--batch-sizes`，并按数值升序去重。

退出码：

- benchmark case 内有查询 error：非 0。
- `--verify-results` 有 mismatch 或 verification error：非 0。
- report 写入失败：非 0。
- 纯性能波动不导致失败。

## 模块落点

新增 service 模块：

```text
crates/service/src/benchmark/
  mod.rs
  benchmark_models.rs
  benchmark_report.rs
  hot_runner.rs
  memory_snapshot.rs
  metrics.rs
  result_verifier.rs
  workload.rs

crates/service/src/scripts/
  benchmark.rs
```

更新：

```text
crates/service/src/lib.rs
crates/service/src/main.rs
crates/service/src/scripts/mod.rs
crates/service/Cargo.toml
```

边界：

- `scripts::benchmark`：只负责 CLI 参数解析和默认值。
- `benchmark::workload`：负责 source SQLite 采样、workload JSON 读写和 deterministic workload。
- `benchmark::hot_runner`：负责 warmup、单手/batch/multi-batch 计时。
- `benchmark::metrics`：负责 percentile、QPS、total 计算。
- `benchmark::memory_snapshot`：负责 RSS/heap 近似。
- `benchmark::result_verifier`：负责 `--verify-results` 的 source DB action count 对账。
- `benchmark::benchmark_report`：负责 JSON/Markdown report。

不要把 benchmark 逻辑塞进 `main.rs`，`main.rs` 只做命令分发和 summary 输出。

## 数据结构

核心类型：

```rust
pub struct BenchmarkCommand {
    pub source: PathBuf,
    pub dir: PathBuf,
    pub meta: PathBuf,
    pub out_path: PathBuf,
    pub md_path: PathBuf,
    pub workload_path: Option<PathBuf>,
    pub seed: u64,
    pub hand_iterations: usize,
    pub batch_iterations: usize,
    pub batch_size: usize,
    pub batch_sizes: Vec<usize>,
    pub requested_dimensions: Vec<DimensionRef>,
    pub workload_mode: WorkloadMode,
    pub warmup_iterations: usize,
    pub verify_checksums: bool,
    pub verify_results: bool,
}

pub enum WorkloadMode {
    Random,
    AbstractLocal,
}

pub struct HandBenchmarkItem {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub concrete_line_id: u32,
    pub hole_cards: String,
}

pub struct BatchBenchmarkItem {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub requests: Vec<BatchBenchmarkRequest>,
}
```

Report 顶层保持上游形态：

```text
generatedAt
engine = "binary"
sourceDbPath
binaryDir
metaDbPath
options
workload
workloadSource
workloadPath
cases
totals
memory
notes
```

7c 的 report 不填真正 cold-start phase accounting；如需要兼容字段，可把 `coldStart` 设为 `null`，并在 notes 说明 cold-start 已拆到 `benchmark-cold`。

## Workload 生成

输入：

- `--source`
- `--seed`
- `--iterations`
- `--hand-iterations`
- `--batch-iterations`
- `--batch-size`
- `--batch-sizes`
- `--dimension`
- `--workload-mode random|abstract-local`
- `--workload`

`--workload`：

- 读取固定 workload JSON。
- 不重新采样。
- 保留上游字段：`seed`、`mode`、`dimensions`、`handQueries`、`batchQueries`、`batchSize`、`batchQueriesBySize`。
- 支持旧 workload 文件缺少 `batchQueriesBySize` 时从 `batchQueries` + `batchSize` 回退。

`random`：

- 从 source SQLite 发现 range dimensions。
- 应用 `--dimension` 过滤。
- 用 seeded random 生成可复现 workload。
- 按 source row count 加权选择维度。
- 单手查询使用分层采样，减少只命中相近 `id` 的概率。
- batch 查询在同一维度内采样多个 `(concrete_line_id, hole_cards)`。

`abstract-local`：

- 先采样 `concrete_lines_*` 的 `abstract_line`。
- 找出同 abstract_line 的 concrete ids。
- batch 内尽量围绕同一个 abstract line 的 concrete ids，模拟局部访问。
- 当 source 表缺数据时回退到 random batch。

SQLite helper：

- 复用现有 `storage::sqlite::Connection`。
- 新增业务层查询 helper，不改 SQLite FFI 的公共行为。
- 表名必须使用受控 quote helper，不能直接拼接未校验用户输入。

## Hot Runner

运行流程：

1. `QueryService::open_with_meta(dir, meta, max_open_handles=100, verify_checksums)`。
2. 对 workload dimensions 执行 `service.prewarm(dimension)`。
3. 执行 `hand-strategy` case。
4. 执行 `batch-hand-strategy` case。
5. 对每个 `batch-size-N` 执行 case。
6. 采集 memory before/after。
7. 如启用 `--verify-results`，抽样校验前 100 个 hand query。
8. 写 JSON/Markdown report。

Case 计时：

- warmup 先执行 `min(warmup_iterations, items.len())` 次，不计入结果。
- 正式迭代每次记录 elapsed。
- 允许 case 继续跑完并累计 errorCount。
- `resultCount` 是返回 action 数总和，不只是请求条数。

case 列表：

- `hand-strategy`
- `batch-hand-strategy`
- `batch-size-1`
- `batch-size-5`
- `batch-size-10`
- `batch-size-50`
- `batch-size-100`
- 以及用户自定义 `--batch-sizes` 中的值。

## 指标计算

每个 case 输出：

- `iterations`
- `warmupIterations`
- `totalMs`
- `avgMs`
- `p50Ms`
- `p95Ms`
- `p99Ms`
- `maxMs`
- `qps`
- `resultCount`
- `errorCount`
- `firstError`

percentile 使用上游相同的线性插值：

```text
index = (p / 100) * (len - 1)
lower = floor(index)
upper = ceil(index)
value = sorted[lower] * (1 - frac) + sorted[upper] * frac
```

totals：

- `iterations = sum(cases.iterations)`
- `totalMs = sum(cases.totalMs)`
- `avgQps = iterations / (totalMs / 1000)`
- `errorCount = sum(cases.errorCount)`
- `resultCount = sum(cases.resultCount)`

## Memory Snapshot

上游 Bun report 有 RSS 和 heap。Rust 侧按平台做近似：

- Windows：优先用 `GetProcessMemoryInfo` 读取 working set / pagefile usage。
- Linux：解析 `/proc/self/status` 的 `VmRSS`，heap 字段可为空或用 allocator 不可得说明。
- 其他平台：字段允许 `null`，notes 说明不支持。

建议 report 使用：

```text
memory.before.rssBytes
memory.after.rssBytes
memory.deltaRssBytes
memory.before.heapApproxBytes
memory.after.heapApproxBytes
memory.deltaHeapApproxBytes
memory.notes
```

如果实现 Windows FFI，优先局部封装在 `memory_snapshot.rs`，不要引入全局平台依赖扩散。若添加依赖，优先选择 `windows-sys` 并通过 `cfg(windows)` 限定。

## --verify-results

行为：

- 只抽样 workload 中前 100 个 hand query。
- 对每个 item 查询 source SQLite：

```sql
SELECT action_name, action_size, amount_bb, frequency, hand_ev
FROM range_data_{strategy}_{player_count}max_{depth_bb}BB
WHERE concrete_line_id = ?
  AND hole_cards = ?
ORDER BY action_name, action_size, amount_bb
```

- SQLite action 行数必须等于 binary 查询返回 action 数。
- mismatch 计入 verification mismatch。
- 查询异常、表不存在、binary 查询异常计入 verification error。
- notes 写入 summary 和前 10 条 mismatch / error。
- 有 mismatch 或 error 时，benchmark 命令退出码非 0。

本阶段只比对 action 数，不比对 frequency/EV。完整 bit-exact 数据正确性由 7a/7b verifier 负责。

## Report

JSON：

- `serde_json::to_writer_pretty`
- 自动创建父目录。

Markdown：

- Summary
- Workload
- Latency Results
- Memory
- Result Verification
- Notes

表格列：

```text
case | iters | avg | p50 | p95 | p99 | max | qps | errors | resultCount
```

notes 必须包含：

- benchmark engine = Rust binary
- resultCount 统计 action entries
- cold-start 已拆到 `benchmark-cold`，本命令只做 hot benchmark
- `--verify-results` 的 match/mismatch/error 摘要

## TDD 实施步骤

### Step 1: CLI parser

新增：

```text
crates/service/src/scripts/benchmark.rs
crates/service/tests/scripts/benchmark.test.rs
```

测试：

- 默认值正确。
- `--batch-size` 自动并入 `--batch-sizes`。
- `--workload-mode` 只接受 `random|abstract-local`。
- `--dimension` 支持 `default:6:100` 和 `default_6max_100BB`。
- 缺 `--dir` / `--source` 报错。

### Step 2: metrics

新增：

```text
crates/service/src/benchmark/metrics.rs
crates/service/tests/benchmark/metrics.test.rs
```

测试：

- percentile 空数组返回 0。
- p50/p95/p99 线性插值和上游一致。
- totals 汇总 errorCount/resultCount/QPS。

### Step 3: workload model and JSON

新增：

```text
crates/service/src/benchmark/benchmark_models.rs
crates/service/src/benchmark/workload.rs
crates/service/tests/benchmark/workload.test.rs
```

测试：

- 固定 seed 生成稳定 hand query。
- `batchQueriesBySize` 读写 round trip。
- 旧 workload 文件能从 `batchQueries` 回退。
- batch sizes 去重排序。
- abstract-local batch 尽量复用同一 abstract_line 的 concrete ids。

### Step 4: report rendering

新增：

```text
crates/service/src/benchmark/benchmark_report.rs
crates/service/tests/benchmark/benchmark_report.test.rs
```

测试：

- JSON 字段包含 options/workload/cases/totals/memory/notes。
- Markdown latency 表包含所有 case。
- 有 verification mismatch 时 notes 可见。

### Step 5: result verifier

新增：

```text
crates/service/src/benchmark/result_verifier.rs
crates/service/tests/benchmark/result_verifier.test.rs
```

测试：

- source action count 等于 binary action count 时 match。
- source 多一行或少一行时 mismatch。
- source 表不存在或 hand 非法时 verification error。
- mismatch/error 被截断为前 10 条 notes。

### Step 6: hot runner

新增：

```text
crates/service/src/benchmark/hot_runner.rs
crates/service/tests/benchmark/hot_runner.test.rs
```

测试：

- 小 fixture 上 `hand-strategy` 返回 resultCount。
- batch case 累计每个 request 的 action 数。
- warmup 不计入 iterations。
- 查询 error 不中断 case，但最终 totals.errorCount > 0。

### Step 7: CLI command integration

更新：

```text
crates/service/src/main.rs
crates/service/src/lib.rs
crates/service/src/scripts/mod.rs
crates/service/Cargo.toml
```

测试：

- `cargo test -p poker-hands-storage-service --test scripts_benchmark_test --target x86_64-pc-windows-msvc`
- service workspace tests 通过。

Smoke：

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark \
  --dir data/range-strata \
  --source data/sqlite/range.db \
  --verify-results
```

期望：

- 写出 `reports/benchmark-range-strata-binary.json`
- 写出 `reports/benchmark-range-strata-binary.md`
- totals.errorCount = 0
- result verification mismatch = 0
- result verification error = 0
- 退出码 0

## Cargo test entries

因为 service crate 当前使用 `autotests = false`，需要新增显式 test target：

```toml
[[test]]
name = "scripts_benchmark_test"
path = "tests/scripts/benchmark.test.rs"

[[test]]
name = "benchmark_metrics_test"
path = "tests/benchmark/metrics.test.rs"

[[test]]
name = "benchmark_workload_test"
path = "tests/benchmark/workload.test.rs"

[[test]]
name = "benchmark_report_test"
path = "tests/benchmark/benchmark_report.test.rs"

[[test]]
name = "benchmark_result_verifier_test"
path = "tests/benchmark/result_verifier.test.rs"

[[test]]
name = "benchmark_hot_runner_test"
path = "tests/benchmark/hot_runner.test.rs"
```

所有 test 文件继续遵守 `<对应代码文件>.test.rs` 命名。

## 验收命令

基础验证：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

发布链路补齐：

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify \
  --mode standalone \
  --dir data/range-strata \
  --verify-checksum

cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify \
  --mode cross \
  --dir data/range-strata \
  --source data/sqlite/range.db \
  --sample-size 10000 \
  --verify-checksum

cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark \
  --dir data/range-strata \
  --source data/sqlite/range.db \
  --verify-results
```

## 风险和处理

- Benchmark 结果会受本机状态影响：报告观测，不设置硬阈值。
- `--verify-results` 只验证 action count：数值正确性仍由 `verify --mode cross` 负责。
- Workload 采样依赖 source SQLite 表结构：表名发现和 quote helper 必须有测试覆盖。
- Windows memory snapshot 需要平台 FFI 或依赖：先做隔离模块，失败时返回 unsupported note，不影响 benchmark 主流程。
- `data/sqlite/range.db-wal` / `range.db-shm` 是 SQLite 运行期 sidecar，benchmark 运行后可能继续出现，不纳入实现失败条件。
