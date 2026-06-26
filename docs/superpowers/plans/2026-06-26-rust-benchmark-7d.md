# Rust Benchmark 7d Implementation Plan — Cold Start

## 目标

实现 Rust 版冷启动 benchmark，覆盖上游 `cold-benchmark.ts` 的核心能力：

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark-cold \
  --dir data/range-strata \
  --source data/sqlite/range.db
```

7d 要做的事：

- 子进程隔离：每次 run 启动全新进程，保证 mmap/heap 全部 cold
- 3 种缓存驱逐模式：`process-cold` / `os-best-effort` / `linux-drop-cache`
- 精细 phase timing：`serviceOpenMs` / `dimensionPrewarmMs` / `firstQueryMs` / `closeMs` / `workerTotalMs`
- 进程级 elapsed 和 overhead 统计
- 内存 RSS 快照（prewarm 前后）
- per-dimension + aggregate 报告
- Phase accounting 审计（unaccounted ratio）
- JSON / Markdown 报告

不在本阶段实现：

- 7e `benchmark compare`
- 多 hand 查询策略（`--query-policy random-sample`）
- Windows 原生 cache eviction（`RAMMap` / `SetProcessWorkingSetSize`）
- 硬性能阈值或自动回归检测

## 上游参考 & 改进点

只读参考同级仓库：

- `E:\idea_project\preflop-storage\src\range-strata-binary\cli\cold-benchmark.ts`
- `E:\idea_project\preflop-storage\src\range-strata-binary\cli\cold-worker.ts`
- `E:\idea_project\preflop-storage\src\range-strata-binary\cli\cold\*`

### 相对上游的改进

| 项 | 上游 | Rust 7d |
|---|---|---|
| Phase 数量 | 9 phases（含 3 个 Bun 运行时开销） | 5+2 phases（去掉无意义的 import 阶段） |
| Percentile 算法 | ceil-based（和 hot benchmark 不一致） | 线性插值（复用 7c `metrics.rs`） |
| filler 大小 | Win 256MB / Linux 512MB | `max(512MB, dataset_size * 2)` |
| mmap RSS 追踪 | 无 | prewarm 前后各取 RSS 快照，量化 page fault I/O |
| 进程开销 | Bun 启动 ~20-50ms | Rust binary ~1-2ms |

## CLI 合约

新增命令：

```text
poker-hands-storage-service benchmark-cold
  --dir <range-strata-dir>
  --source <range.db>
  [--meta <meta.db>]
  [--out <report.json>]
  [--md <report.md>]
  [--mode <process-cold|os-best-effort|linux-drop-cache>]
  [--runs <number>]
  [--dimension <strategy:players:bb>]
  [--query-policy <first|fixed>]
  [--concrete-line-id <number>]
  [--hand <string>]
  [--cache-filler-mb <number>]
  [--max-errors-per-dimension <number>]
  [--fail-fast]
  [--verify-checksum]
```

新增内部子命令（不暴露在帮助文档中）：

```text
poker-hands-storage-service cold-worker
  --dir <range-strata-dir>
  --meta <meta.db>
  --strategy <string>
  --player-count <number>
  --depth-bb <number>
  --concrete-line-id <number>
  --hand <string>
  [--verify-checksum]
```

`cold-worker` 通过 stdout 输出单行 JSON 结果，不输出其他内容到 stdout。日志走 stderr。

默认值：

| Option | Default |
| --- | --- |
| `--dir` | required |
| `--source` | required |
| `--meta` | `<dir>/meta.db` |
| `--out` | `reports/benchmark-cold-start.json` |
| `--md` | `reports/benchmark-cold-start.md` |
| `--mode` | `process-cold` |
| `--runs` | `10` |
| `--query-policy` | `first` |
| `--cache-filler-mb` | Windows: `512` / Linux: `1024` |
| `--max-errors-per-dimension` | `∞` |

退出码：

- 有 run error：非 0。
- report 写入失败：非 0。
- cache eviction 失败不影响退出码（记录到 notes）。

## 模块落点

新增 service 模块：

```text
crates/service/src/benchmark/
  cold_runner.rs          # 主控循环：维度遍历、子进程调度、结果收集
  cold_worker.rs          # worker 进程内逻辑：phase timing、JSON 输出
  cold_types.rs           # 冷启动特有类型
  cold_report.rs          # JSON/Markdown 报告渲染
  cache_eviction.rs       # 3 种缓存驱逐策略

crates/service/src/scripts/
  benchmark_cold.rs       # CLI 参数解析
  cold_worker_cmd.rs      # cold-worker 子命令入口
```

更新：

```text
crates/service/src/benchmark/mod.rs     # 新增 mod 声明
crates/service/src/scripts/mod.rs       # 新增 mod 声明
crates/service/src/main.rs              # 新增 benchmark-cold / cold-worker 分发
crates/service/src/lib.rs               # 按需导出
crates/service/Cargo.toml               # 按需加 test target
```

复用的已有模块：

- `benchmark::metrics` — `percentile()` 线性插值、`LatencySummary` 构建
- `benchmark::memory_snapshot` — RSS 采集
- `domain::dimension` — `DimensionRef`
- `storage::manifest` — `load_manifest()`
- `storage::sqlite` — source DB 查询

## 数据结构

### Worker 输出（stdout JSON）

```rust
pub struct ColdWorkerOutput {
    pub ok: bool,
    pub store_open_and_first_query_ms: f64,
    pub result_count: usize,
    pub memory_before: MemorySnapshotData,
    pub memory_after: MemorySnapshotData,
    pub timings: ColdWorkerTimings,
    pub error: Option<String>,
}

pub struct ColdWorkerTimings {
    pub service_open_ms: f64,          // meta.db + action schema
    pub dimension_prewarm_ms: f64,     // idx/bin mmap open
    pub first_query_ms: f64,           // 首次查询
    pub close_ms: f64,                 // 资源关闭
    pub worker_total_ms: f64,          // 进程内全程
}

pub struct MemorySnapshotData {
    pub rss_bytes: Option<u64>,
}
```

### Run 结果（主进程侧）

```rust
pub struct ColdStartRunResult {
    pub worker: ColdWorkerOutput,
    pub run_index: usize,
    pub process_elapsed_ms: f64,       // 父进程计时
    pub process_overhead_ms: f64,      // elapsed - worker_total
    pub eviction: EvictionResult,
    pub exit_code: i32,
    pub valid_json: bool,
    pub phase_accounting: PhaseAccounting,
}

pub struct PhaseAccounting {
    pub phase_sum_ms: f64,
    pub worker_total_ms: f64,
    pub unaccounted_ms: f64,
    pub unaccounted_ratio: f64,
}
```

### 缓存驱逐

```rust
pub enum ColdStartMode {
    ProcessCold,
    OsBestEffort,
    LinuxDropCache,
}

pub struct EvictionResult {
    pub requested: bool,
    pub method: ColdStartMode,
    pub succeeded: bool,
    pub duration_ms: f64,
    pub filler_size_bytes: u64,
    pub dataset_size_bytes: u64,
    pub notes: Vec<String>,
}
```

### 报告

```rust
pub struct ColdStartBenchmarkReport {
    pub generated_at: String,
    pub mode: String,
    pub platform: String,
    pub runs_per_dimension: usize,
    pub source_db_path: String,
    pub binary_dir: String,
    pub meta_db_path: String,
    pub verify_checksums: bool,
    pub cache_filler_size_bytes: u64,
    pub dimensions: Vec<DimensionColdStartReport>,
    pub aggregate: AggregateReport,
    pub notes: Vec<String>,
}

pub struct DimensionColdStartReport {
    pub dimension: String,
    pub query: DimensionQueryInfo,
    pub runs: usize,
    pub success_count: usize,
    pub error_count: usize,
    pub store_open_and_first_query_ms: LatencySummary,
    pub process_elapsed_ms: LatencySummary,
    pub phase_timings: ColdStartPhaseSummaries,
    pub memory_delta_rss_bytes: LatencySummary,
    pub phase_accounting: PhaseAccounting,
    pub failures: Vec<ColdStartRunFailure>,
}
```

## 缓存驱逐实现

### `process-cold`

不做任何驱逐。子进程自然没有 mmap 映射，但 OS 页缓存可能还在。

### `os-best-effort`

```text
1. 在 temp dir 创建 filler 文件
2. 按 1MB chunk 写入非零数据（填满 OS 页缓存）
3. 按 1MB chunk 读回（确保写入被提交到页缓存）
4. 删除 filler 文件
```

filler 大小：`max(512MB, dataset_size * 2)`（上游 256MB 太小）。

### `linux-drop-cache`

```text
1. 调用 sync (Command::new("sync"))
2. 写 "3\n" 到 /proc/sys/vm/drop_caches
3. 需要 root 权限
```

所有模式的失败都记录到 `EvictionResult.notes`，不中断 benchmark。

## 子进程架构

### 为什么需要子进程

- mmap 文件描述符绑定到进程。在同一进程内 close + reopen 不会清 OS 页缓存。
- 保证每次 run 的 heap、mmap 状态都是全新的。
- 与上游 Bun worker 架构对齐，方便 7e 做对比。

### 自我执行

主进程通过 `std::env::current_exe()` 获取自身路径，spawn `cold-worker` 子命令：

```rust
let child = Command::new(current_exe()?)
    .args(["cold-worker", "--dir", &dir, "--meta", &meta, ...])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;
```

Worker 内流程：

```text
1. 记录 start time
2. QueryService::open(dir, meta, max_handles=2, verify_checksums)    → service_open_ms
3. 采集 memory_before
4. service.prewarm(dimension)                                         → dimension_prewarm_ms
5. service.query(dimension, concrete_line_id, hole_cards)             → first_query_ms
6. 采集 memory_after
7. service.close / drop                                               → close_ms
8. 计算 worker_total_ms
9. 输出 JSON 到 stdout
```

## 报告格式

### Markdown

```text
# Range Strata Binary Cold-Start Benchmark

## Summary
- Mode / Platform / Dimensions / Runs / Errors / ...
- Aggregate store open+query p50/p95
- Aggregate process elapsed p50/p95
- Phase accounting (worst)

## Aggregate Phase Breakdown
| Phase | P50 | P95 | Avg | Max |
| --- | --- | --- | --- | --- |

## Dimensions
| Dimension | Runs | Errors | Store+Query P50 | P95 | Process P50 | P95 | RSS Delta P95 | Query |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |

## Failures
（无则 "None"）

## Dimension Phase Breakdown
| Dimension | Service Open P95 | Prewarm P95 | Query P95 | Worker Total P95 | Process Overhead P95 |
| --- | --- | --- | --- | --- | --- |

## Notes
```

### JSON

`serde_json::to_writer_pretty`，字段命名用 snake_case（Rust 惯例），在 notes 中说明与上游 camelCase 的映射。

## TDD 实施步骤

### Step 1: cold_types + cold_worker output

新增：

```text
crates/service/src/benchmark/cold_types.rs
```

测试（内联 unit test）：

- `ColdWorkerOutput` serde JSON round trip。
- `PhaseAccounting` 计算 unaccounted ratio。
- `ColdWorkerTimings` 全零时 ratio = 0。

### Step 2: cache_eviction

新增：

```text
crates/service/src/benchmark/cache_eviction.rs
crates/service/tests/benchmark/cache_eviction.test.rs
```

测试：

- `process-cold` 不写文件，直接返回 succeeded。
- `os-best-effort` 在 tempdir 创建并删除 filler，returned bytes 正确。
- `linux-drop-cache` 在非 Linux 返回 succeeded=false。
- filler 大小计算：`max(512MB, dataset*2)`。

### Step 3: cold_worker

新增：

```text
crates/service/src/benchmark/cold_worker.rs
crates/service/src/scripts/cold_worker_cmd.rs
```

测试：

- 使用 build_test_store fixture，运行 cold_worker 逻辑，验证输出 JSON 包含正确字段。
- `first_query_ms > 0`。
- `store_open_and_first_query_ms ≈ service_open_ms + dimension_prewarm_ms + first_query_ms`。
- 查询不存在的维度时 `ok=false` + 非空 error。

### Step 4: cold_runner

新增：

```text
crates/service/src/benchmark/cold_runner.rs
crates/service/tests/benchmark/cold_runner.test.rs
```

测试：

- 小 fixture + 1 维度 + 2 runs：结果包含 2 个 run，aggregate 正确。
- phase accounting 的 unaccounted ratio < 10%（放宽阈值，fixture 太小不稳定）。
- `--fail-fast` 遇到错误后只有 1 个 run。
- `--max-errors-per-dimension 1` 遇到第 2 个错误后停止。

### Step 5: cold_report

新增：

```text
crates/service/src/benchmark/cold_report.rs
crates/service/tests/benchmark/cold_report.test.rs
```

测试：

- JSON 包含 `generated_at`, `mode`, `dimensions`, `aggregate`, `notes`。
- Markdown 包含 Summary / Phase Breakdown / Dimensions / Notes 段落。
- failure 列表为空时 Markdown 显示 "None"。

### Step 6: CLI parser + main 集成

新增：

```text
crates/service/src/scripts/benchmark_cold.rs
```

更新：

```text
crates/service/src/scripts/mod.rs
crates/service/src/main.rs
```

测试：

- 默认值正确。
- `--mode` 只接受 3 个值。
- `--query-policy fixed` 需要 `--concrete-line-id` 和 `--hand`。
- `--runs 0` 报错。

### Step 7: 端到端 smoke

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark-cold \
  --dir data/range-strata \
  --source data/sqlite/range.db \
  --runs 3 \
  --dimension default:6:100
```

期望：

- 写出 `reports/benchmark-cold-start.json`
- 写出 `reports/benchmark-cold-start.md`
- aggregate.error_count = 0
- 3 个 run 全部 ok
- 退出码 0

## Cargo test entries

```toml
[[test]]
name = "benchmark_cache_eviction_test"
path = "tests/benchmark/cache_eviction.test.rs"

[[test]]
name = "benchmark_cold_runner_test"
path = "tests/benchmark/cold_runner.test.rs"

[[test]]
name = "benchmark_cold_report_test"
path = "tests/benchmark/cold_report.test.rs"
```

`cold_worker` 和 `cold_types` 使用内联 `#[cfg(test)]` 模块测试。
CLI parser 在 `benchmark_cold.rs` 内用内联测试或并入已有 `scripts_benchmark_test`。

## 验收命令

基础验证：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

Smoke：

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark-cold \
  --dir data/range-strata \
  --source data/sqlite/range.db \
  --runs 3 \
  --dimension default:6:100
```

## 风险和处理

- **冷启动结果受 OS 页缓存影响**：`process-cold` 只保证进程级 cold，不保证 OS 级 cold。报告中诚实说明。
- **子进程失败排查困难**：worker 的 stderr 全量捕获并写入 failure 记录，方便调试。
- **Windows filler I/O 慢**：512MB filler 写入+读回可能需要 2-3 秒。每个 run 都执行一次。`--runs 10` 在 9 个维度上约需 3-5 分钟。Notes 中说明 `os-best-effort` 的时间成本。
- **CI 环境 page cache 波动**：不设硬性能阈值。phase accounting ratio > 5% 在 notes 中 warn，但不导致失败。
- **`current_exe()` 在某些环境可能失败**：提供 `--worker-binary` 覆盖选项作为后备（low priority，遇到再加）。
