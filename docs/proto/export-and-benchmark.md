# Proto V2 导出与基准

更新日期：2026-07-14

导出单维度（默认 `default:6:100`）：

```powershell
cargo run -p poker-hands-storage-tools -- export-compact-line-matrix-archive `
  --source-db data\sqlite\range.db `
  --out-dir reports\proto-range-storage-default-6-100 `
  --dimension default:6:100 `
  --overwrite
```

导出全部已发现维度：

```powershell
cargo run -p poker-hands-storage-tools --release -- export-all-compact-line-matrix-archives `
  --source-db data\sqlite\range.db `
  --out-dir reports\proto-range-storage-all `
  --overwrite
```

完整验证某一维度：

```powershell
cargo run -p poker-hands-storage-tools -- verify-compact-line-matrix-archive `
  --dir reports\proto-range-storage-default-6-100
```

`--proto-root` / `--compact-dir` 必须指向全部维度导出根目录，不是一个 dimension 子目录。

三方基准命令为 `benchmark-three-way-hot`、`benchmark-three-way-cold` 和
`benchmark-three-way-stability`。hot 覆盖 hand strategy、batch（含各 batch size）、
hands-by-actions、concrete-lines-exact、drill-scenarios-metadata。三方均应用 V2 的 NULL
过滤和频率量化；drill metadata 均 materialize `abstract_line` 列表，SQLite 不能以 `COUNT`
替代。

cold 是 **process-cold**：每次使用新工具进程，报告 open/prewarm/first-query latency 与
RSS，但不会清空 OS page cache，不能宣称 OS-cache-cold。stability 至少重复两次固定 workload，
输出跨运行 P50/P95，以及 Proto 的 matrix 分段 profile 与 metadata 首次/命中/LRU 淘汰后访问。

当前三方 benchmark 仅用于开发观测，不能作为 Proto / SQLite 正式性能或内存基线：它尚未校验
完整策略返回值，且当前预热与缓存 profile 不对等。正式对照设计见
[`replay-memory-benchmark-design.md`](replay-memory-benchmark-design.md)；实现前不得生成或引用
Proto 快于 SQLite、或 Proto 更省 RSS 的结论。
