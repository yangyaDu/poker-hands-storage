# 性能基准测试

## Hot 基准（mmap 缓存命中）

### Binary 基准

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --verify-results
```

### SQLite baseline

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-sqlite `
  --source data\sqlite\range.db
```

### 对比报告

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-compare `
  --binary reports\benchmark-range-strata-binary.json `
  --sqlite reports\benchmark-sqlite.json
```

### 控制参数

| 参数 | 说明 |
|---|---|
| `--seed` | 随机种子 |
| `--iterations` | 迭代次数 |
| `--hand-iterations` | 每手迭代次数 |
| `--batch-iterations` | 批量迭代次数 |
| `--batch-size` | 单一批量大小 |
| `--batch-sizes` | 多批量大小（如 `1,5,10,50,100`） |
| `--dimension` | 指定维度（如 `default:6:100` 或 `default_6max_100BB`） |
| `--workload-mode` | `random` 或 `abstract-local` |
| `--workload` | 加载已有 workload JSON |
| `--write-workload` | 导出 workload JSON（用于 SQLite baseline 复用） |
| `--verify-results` | 前 100 条查询结果与源 SQLite 校验 |

### 报告

- `reports/benchmark-range-strata-binary.json` / `.md`
- 包含 QPS、avg、p50、p95、p99、max、error count、内存近似

## Cold 基准（冷启动）

### Binary cold

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --mode process-cold `
  --runs 10
```

### SQLite cold

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-sqlite-cold `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --mode process-cold `
  --runs 10
```

### Cold 对比

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold-compare `
  --binary reports\benchmark-cold-start.json `
  --sqlite reports\benchmark-sqlite-cold-start.json
```

## 注意事项

- 不同 workload mode、dimension、sample set 的报告不可直接对比
- 对比前确保 binary 和 SQLite 使用相同 workload
- 冷启动结果需区分：进程启动、metadata 打开、mmap 创建、首次查询、OS page-cache 影响
