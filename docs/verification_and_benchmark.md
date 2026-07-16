# Proto V3 验证与 Benchmark

更新日期：2026-07-16

V3 只有两个正确性基线：归档自洽检查，以及以源 SQLite 为唯一事实来源的 cross verify。V2 不参与
结果或性能比较。

## Standalone

```powershell
cargo run -p poker-hands-storage-tools -- v3-verify `
  --archive data\proto-v3\default_6max_100BB `
  --out reports\v3-verify.json
```

检查 manifest、六个文件 size/CRC、header、section、locator 边界和连续性、hash index 一一对应、
concrete ID 连续性、引用可达性、Protobuf decode、bitmap/array、action identity 和 NULL-EV sentinel。

## SQLite cross verify

```powershell
cargo run -p poker-hands-storage-tools -- v3-cross-verify `
  --source data\sqlite\range.db `
  --archive data\proto-v3\default_6max_100BB `
  --out reports\v3-cross.json
```

比较 drill 映射、abstract/concrete 映射、V3 ID，以及每条 concrete path 的全部 action identity 与
169 手牌 cell。数值按 V3 量化规则比较，NULL 必须精确一致。任何差异命令返回非零。

## SQLite / V3 benchmark

```powershell
cargo run -p poker-hands-storage-tools -- v3-benchmark `
  --source data\sqlite\range.db `
  --archive-root data\proto-v3 `
  --dimension default:6:100 `
  --out reports\v3-benchmark.json `
  --md reports\v3-benchmark.md
```

Benchmark 在计时前强制执行 full cross correctness gate。报告覆盖：

- V3 cold open 和首次 metadata page；
- SQLite metadata 与 V3 metadata hit；
- SQLite strategy rows、V3 首次 decode 与 strategy hit；
- batch、hands-by-actions、handle eviction/reopen；
- P50/P95/P99、QPS、metadata/strategy cache bytes 和进程 RSS。

性能模式可关闭完整文件 CRC；发布 standalone/cross gate 必须验证完整文件 CRC。Fixture benchmark 只
验证工具链和报告字段，真实结论必须来自完整九维源库和固定机器环境。
