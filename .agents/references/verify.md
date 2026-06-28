# 数据验证

检查构建输出的完整性和正确性。

## Standalone 验证

检查 manifest、meta.db、.idx、.bin 文件结构和 CRC32C 校验。
不需要源 SQLite。

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode standalone `
  --dir data\range-strata `
  --verify-checksum
```

## Cross 验证

在 standalone 基础上，比较源 SQLite 行与二进制 pack 的 float32 bit-exact 一致性。

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode cross `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --sample-size 10000 `
  --verify-checksum
```

## 参数

| 参数 | 说明 |
|---|---|
| `--mode` | `standalone` 或 `cross` |
| `--dir` | 二进制数据目录 |
| `--source` | 源 SQLite 路径（仅 cross 模式需要） |
| `--sample-size` | 抽样行数；`0` = 全量扫描（发布级验证） |
| `--verify-checksum` | 启用 CRC32C 校验 |

## 报告输出

- `reports/range-strata-verify-standalone.json` / `.md`
- `reports/range-strata-verify-cross.json` / `.md`
