# 性能基准测试

详细口径见 `docs/verification_and_benchmark.md`；正式结论只写入 `docs/binary-vs-sqlite-benchmark-and-verification-report.md`。

## 代码边界

- `storage-tools/src/benchmark/hot/runner.rs`：Binary hot benchmark。
- `storage-tools/src/benchmark/hot/sqlite_runner.rs`：同 workload 的 SQLite hot baseline。
- `storage-tools/src/benchmark/hot/compare.rs`：Binary hot vs SQLite hot 对比。
- `storage-tools/src/benchmark/cold/`：Binary/SQLite cold 和 cold compare。
- `storage-tools/src/benchmark/native/`：`core`、`native-sdk`、`http-service` 三路对比。
- `storage-tools/src/benchmark/report.rs`：hot、SQLite、metadata、native、compare、cold、cold compare 的 JSON/Markdown 报告入口。
- `storage-tools/src/benchmark/report_support.rs`：写文件、UTC 时间、耗时/字节格式化和 Markdown 表格 helper。

不要重新拆出外层 `benchmark/sqlite` 或 `benchmark/compare` 模块；hot 的 SQLite baseline 和 hot compare 都归在 `benchmark/hot/` 下。

## Hot 基准

先生成或复用同一份 workload，再分别跑 Binary、SQLite baseline、compare：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --write-workload reports\benchmark-workload.json `
  --verify-results

cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-sqlite `
  --source data\sqlite\range.db `
  --workload reports\benchmark-workload.json

cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-compare `
  --binary reports\benchmark-range-strata-binary.json `
  --sqlite reports\benchmark-sqlite.json
```

正式 hot cases：

| Case | 说明 |
|---|---|
| `concrete-lines-exact` | 按 `concrete_line` 精确 lookup `concrete_line_id`，要求只命中 1 行且 id 一致 |
| `hand-strategy` | 单 concrete_line_id + hand 查询 |
| `batch-hand-strategy` | 默认批量大小的 batch 查询 |
| `batch-size-{N}` | 多批量大小 sweep |
| `hands-by-actions` | action filter + frequency 阈值匹配手牌 |
| `drill-scenarios-metadata` | drill scenario metadata 查询 |

`concrete-lines-exact` 不单独存入 workload JSON；runner 从 `hand_queries` 的 `concrete_line_id` 派生并跳过空 `concrete_line`。Binary 读取运行目录 `meta.db`，SQLite baseline 读取源库 `concrete_lines_*` 表。

`--verify-results` 只做 benchmark 护栏：抽样比较 Binary vs SQLite action-count，并不替代 full cross verify。

常用参数：

| 参数 | 说明 |
|---|---|
| `--seed` | 随机种子 |
| `--iterations` | 外层迭代次数 |
| `--hand-iterations` | 单手查询迭代次数 |
| `--batch-iterations` | 批量查询迭代次数 |
| `--batch-size` | 默认批量大小 |
| `--batch-sizes` | 多批量大小，如 `1,5,10,50,100` |
| `--dimension` | 指定维度，如 `default:6:100` |
| `--workload-mode` | `random` 或 `abstract-local` |
| `--workload` | 加载已有 workload JSON |
| `--write-workload` | 导出 workload JSON，供 baseline 复用 |

## Cold 基准

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --mode process-cold `
  --runs 10

cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-sqlite-cold `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --mode process-cold `
  --runs 10

cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold-compare `
  --binary reports\benchmark-cold-start.json `
  --sqlite reports\benchmark-sqlite-cold-start.json
```

Cold 输出关注 `service_open`、`dimension_prewarm`、`first_query`、`close`、`worker_total`。`process-cold` 只代表新进程 open/query 成本，不驱逐 OS page cache；需要更冷的口径时使用 `os-best-effort` 或 Linux root 下的 `linux-drop-cache`。

## Native benchmark

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-native `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --native-entry range-store-native\index.js `
  --http-service-bin target\x86_64-pc-windows-msvc\debug\poker-hands-storage-service.exe
```

当前正式 native benchmark 只比较 `core:*`、`native-sdk:*`、`http-service:*`。额外 case `line-to-hands-by-actions` 覆盖 `concrete_line -> concrete_line_id -> handsByActions` 链路。

## 注意事项

- 不同 workload mode、dimension、sample set 的报告不可直接对比。
- Binary 和 SQLite 对比前必须使用同一份 workload。
- Benchmark 报告里的结果一致性校验是性能测试护栏，数据正确性仍以 `verify --mode standalone|cross` 为准。
