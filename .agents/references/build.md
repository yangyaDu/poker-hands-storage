# 构建二进制数据

从源 SQLite 生成 Range Strata 二进制存储（manifest.json + meta.db + .idx + .bin）。

## 命令

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- build `
  --source-db data\sqlite\range.db `
  --out-dir data\range-strata `
  --dimension default:6:100 `
  --overwrite
```

## 参数

| 参数 | 说明 |
|---|---|
| `--source-db` | 源 SQLite 数据库路径 |
| `--out-dir` | 输出目录 |
| `--dimension` | 维度选择（格式：`strategy:players:depth`），可重复；省略则构建全部 |
| `--overwrite` | 覆盖已有输出 |
| `--max-concrete-lines` | 限制每个维度的行数，用于 smoke 测试 fixture |

## 构建后验证

构建完成后应依次运行：

1. standalone 验证（见 `references/verify.md`）
2. cross 验证（如有源 SQLite）
