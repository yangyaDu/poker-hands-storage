# benchmark 模块重构 + 7e 实现方案

## 问题

`benchmark/` 目前 13 个文件平铺，再加 7e 会膨胀到 17 个。文件命名前缀混乱，职责不清：

```
benchmark/
├── benchmark_models.rs    ← hot 专用的 workload 模型
├── benchmark_report.rs    ← hot 专用的报告
├── cache_eviction.rs      ← cold 专用
├── cold_report.rs         ← cold 专用
├── cold_runner.rs         ← cold 专用
├── cold_types.rs          ← cold 专用
├── cold_worker.rs         ← cold 专用
├── hot_runner.rs          ← hot 专用
├── memory_snapshot.rs     ← 共用
├── metrics.rs             ← 共用
├── mod.rs
├── result_verifier.rs     ← hot 专用
└── workload.rs            ← hot + sqlite 共用
```

## 方案：按子功能拆子目录

```
benchmark/
├── mod.rs                     ← pub use 入口
├── metrics.rs                 ← 共用：percentile, measure_benchmark_case, BenchmarkCaseResult, BenchmarkTotals
├── memory_snapshot.rs         ← 共用：RSS snapshot
│
├── hot/                       ← 7c hot-path benchmark
│   ├── mod.rs
│   ├── models.rs              ← ← benchmark_models.rs
│   ├── runner.rs              ← ← hot_runner.rs
│   ├── report.rs              ← ← benchmark_report.rs
│   ├── workload.rs            ← ← workload.rs
│   └── result_verifier.rs     ← ← result_verifier.rs
│
├── cold/                      ← 7d cold-start benchmark
│   ├── mod.rs
│   ├── types.rs               ← ← cold_types.rs
│   ├── runner.rs              ← ← cold_runner.rs
│   ├── worker.rs              ← ← cold_worker.rs
│   ├── report.rs              ← ← cold_report.rs
│   └── cache_eviction.rs      ← ← cache_eviction.rs
│
├── sqlite/                    ← 7e-Part1: SQLite baseline (NEW)
│   ├── mod.rs
│   └── runner.rs              ← SQLite query executor
│
└── compare/                   ← 7e-Part2: 对比报告 (NEW)
    ├── mod.rs
    ├── types.rs               ← CompareReport, CaseComparison
    ├── runner.rs              ← 读两份 JSON + join + ratio
    └── report.rs              ← JSON + Markdown 渲染
```

### 优势

1. **每个子目录 = 一个子命令**，职责一目了然
2. 子目录内的文件名统一（`runner.rs`, `report.rs`, `types.rs`），靠目录名区分
3. 共用模块（`metrics.rs`, `memory_snapshot.rs`）留在 `benchmark/` 根
4. 新增 7e 只是多两个干净的子目录，不污染已有结构

### 重构影响

纯文件移动 + `mod.rs` 重新 wire + `use` path 更新：

| 旧路径 | 新路径 |
|---|---|
| `benchmark::benchmark_models` | `benchmark::hot::models` |
| `benchmark::benchmark_report` | `benchmark::hot::report` |
| `benchmark::hot_runner` | `benchmark::hot::runner` |
| `benchmark::workload` | `benchmark::hot::workload` |
| `benchmark::result_verifier` | `benchmark::hot::result_verifier` |
| `benchmark::cold_types` | `benchmark::cold::types` |
| `benchmark::cold_runner` | `benchmark::cold::runner` |
| `benchmark::cold_worker` | `benchmark::cold::worker` |
| `benchmark::cold_report` | `benchmark::cold::report` |
| `benchmark::cache_eviction` | `benchmark::cold::cache_eviction` |

外部消费者（`main.rs`, `scripts/benchmark.rs`, `scripts/benchmark_cold.rs`）需要更新 `use` 路径。

---

## 执行顺序

### Phase 1: 重构现有结构（不改逻辑）

1. 创建 `benchmark/hot/` 和 `benchmark/cold/` 子目录
2. 移动文件（git mv）
3. 更新所有 `mod.rs` 和 `use` 路径
4. `cargo fmt && cargo clippy -D warnings && cargo test` 通过

### Phase 2: 实现 7e

1. 新建 `benchmark/sqlite/runner.rs` — SQLite query executor
2. 新建 `scripts/benchmark_sqlite.rs` — CLI 解析
3. `main.rs` 新增 `benchmark-sqlite` 子命令
4. 新建 `benchmark/compare/` — 对比类型、runner、报告
5. 新建 `scripts/benchmark_compare.rs` — CLI 解析
6. `main.rs` 新增 `benchmark-compare` 子命令
7. Validate + Smoke test

---

## scripts/ 目录也一并整理

当前 `scripts/` 有 3 个 CLI 解析文件，加上 7e 会有 5 个。建议重命名以保持一致：

```
scripts/
├── mod.rs
├── benchmark.rs             ← 已有 (hot path CLI)
├── benchmark_cold.rs        ← 已有 (cold start CLI)
├── benchmark_sqlite.rs      ← NEW
├── benchmark_compare.rs     ← NEW
└── verify_store.rs          ← 已有
```

`scripts/` 的文件数可接受（5 个 CLI 解析器），暂不需要子目录化。

---

## 预估

| Phase | 工作量 |
|---|---|
| Phase 1: 重构 | ~30 分钟（纯机械移动） |
| Phase 2: 7e 实现 | ~700 行新代码 |
