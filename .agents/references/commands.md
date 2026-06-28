# 命令参考

本文件包含所有离线工具和服务命令的完整参数。
仅在需要具体命令参数时加载此文件。

## 构建二进制数据

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- build `
  --source-db data\sqlite\range.db `
  --out-dir data\range-strata `
  --dimension default:6:100 `
  --overwrite
```

- `--dimension`：可重复指定多个维度，省略则构建全部
- `--max-concrete-lines`：限制行数，用于 smoke 测试 fixture
- `--overwrite`：覆盖已有输出

## Standalone 验证

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode standalone `
  --dir data\range-strata `
  --verify-checksum
```

## Cross 验证

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode cross `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --sample-size 10000 `
  --verify-checksum
```

- `--sample-size 0`：全量扫描（发布级验证）
- 报告输出到 `reports/range-strata-verify-*.json` 和 `.md`

## Hot 基准测试

Binary 基准：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --verify-results
```

SQLite baseline：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-sqlite `
  --source data\sqlite\range.db
```

对比报告：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-compare `
  --binary reports\benchmark-range-strata-binary.json `
  --sqlite reports\benchmark-sqlite.json
```

Hot 基准控制参数：
- `--seed`, `--iterations`, `--hand-iterations`, `--batch-iterations`
- `--batch-size`, `--batch-sizes 1,5,10,50,100`
- `--dimension default:6:100` 或 `--dimension default_6max_100BB`
- `--workload-mode random|abstract-local`
- `--workload <workload.json>` / `--write-workload <workload.json>`

## Cold 基准测试

Binary cold：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --mode process-cold `
  --runs 10
```

SQLite cold：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-sqlite-cold `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --mode process-cold `
  --runs 10
```

Cold 对比：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold-compare `
  --binary reports\benchmark-cold-start.json `
  --sqlite reports\benchmark-sqlite-cold-start.json
```

## 启动 HTTP 服务

```powershell
$env:PHS_DATA_DIR = "data\range-strata"
$env:PHS_META_DB = "data\range-strata\meta.db"
$env:PHS_PREWARM = "default:6:100"
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- serve
```

## Docker 部署

```powershell
docker compose -f .docker\docker-compose.yml up --build -d
docker compose -f .docker\docker-compose.yml ps
Invoke-RestMethod -Uri http://127.0.0.1:8080/ready
```

## 代码质量检查

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```
