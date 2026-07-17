# Proto V3 运行与发布

更新日期：2026-07-17

Proto V3 是 HTTP service 和 native SDK 的默认存储。运行时只读取维度目录中的 manifest 与三组
`.pb/.idx` 文件，不打开 SQLite，不读取 V2，也不需要 `lines.db`。

2026-07-17 的完整源库 release gate 已完成：工具、HTTP service 和 native SDK 均以 V3 作为默认 Proto
格式。已验证 root 是 `data/proto-v3-releases/2026-07-17T000001Z`，九维 standalone/cross/benchmark
汇总位于 `reports/v3-release-20260717/release-gate-summary.json`；verify/cross 零失败和零差异，全部 benchmark 的
`correctnessVerified=true`。

## CLI

```text
v3-export         单维导出，随后执行 SQLite cross verify
v3-export-all     发现并导出全部维度，每维执行 standalone + cross verify
v3-verify         独立验证 manifest、文件、索引、引用和 payload 不变量
v3-cross-verify   以源 SQLite 为唯一基线逐映射、逐 action identity、逐 cell 比较
v3-benchmark      SQLite/V3 两方性能基线；计时前强制通过 cross correctness gate
```

示例：

```powershell
cargo run -p poker-hands-storage-tools -- v3-export-all `
  --source data\sqlite\range.db --out-root data\proto-v3

cargo run -p poker-hands-storage-tools -- v3-benchmark `
  --source data\sqlite\range.db --archive-root data\proto-v3 `
  --dimension default:6:100
```

Benchmark 报告包含 cold open、首次 metadata page、metadata hit、首次 strategy decode、strategy hit、
batch、hands-by-actions、handle reopen/eviction、P50/P95/P99、QPS、cache bytes 和 RSS。

## 发布门禁

- 计划发布的每个维度都必须存在七个文件，不能夹带 SQLite 或 V2 运行产物。
- standalone verify 必须通过。
- SQLite cross verify 的映射和 action cell 必须零差异，NULL EV 必须精确一致。
- benchmark correctness 必须通过，缓存 resident bytes 不得超过配置预算。
- `cargo fmt --all -- --check` 与 `cargo test --workspace` 必须通过。

若格式或数据失败，应修复后从 SQLite 重新导出新的 V3 release；不增加 V2 reader 或双读路径。
