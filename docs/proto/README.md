# Proto Range Storage

更新日期：2026-07-17

本目录同时保留 Proto V2 参考实现文档，以及当前默认运行格式 Proto V3 的说明。
V1 已删除，不能作为兼容目标或实现参考。

V2 参考实现的字段号、字段类型和 Protobuf 注释以
[`compact_matrix.proto`](../../storage-tools/proto/zenithstrat/gto/v2/compact_matrix.proto) 为准。
V3 的目标 schema、文件布局、首发和验证要求以 V3 实施方案为准；实现仍位于
[`storage-tools/src/proto_range_storage/`](../../storage-tools/src/proto_range_storage/)。

V3 是当前主发布 Proto 格式；HTTP service 与 native SDK 运行时只读取 V3。2026-07-17 已使用完整源
SQLite 完成九维 release gate：release root 为 `data/proto-v3-releases/2026-07-17T000001Z`，汇总报告位于
`reports/v3-release-20260717/release-gate-summary.json`。`storage-tools` 的 V3 发布命令均显式使用 `v3-` 前缀，CLI 仍保留 V2
参考命令；无参数 CLI 不代表选择任何运行时格式。V2 只用于参考 mmap、索引、Protobuf、cache 和 benchmark
实现；V3 不读取 V2，不做 V2/V3 对比，也不提供回退到 V2 的发布路径。

文档与实现冲突时必须修正其中一方，不能另起一套格式说明。

1. [Proto V3 业务存储实施方案](v3-business-storage-implementation-plan.md)：定义 drill、action path、hand strategy 三条业务查询链及已完成的首发验证。
2. [Proto V3 运行与发布](v3-runtime-and-operations.md)：CLI、服务配置、目录结构和发布门禁。
3. [V2 格式规范](v2-format.md)：V2 参考实现的字段、位图、量化、文件布局和校验。
4. [运行时与查询](runtime-and-query.md)：V2 参考实现的解码索引、缓存、metadata 和 core 接口。
5. [导出与基准](export-and-benchmark.md)：V2 导出、验证、三方基准及报告口径。
6. [Cache 与 Decode 优化实践方案](cache-and-decode-optimization-plan.md)：V2 的观测、容量扫描、预热与 decode accelerator 参考。
7. [Replay 内存基准设计](replay-memory-benchmark-design.md)：历史 replay、等价缓存和 RSS 测量设计。
8. [Proto V2 存储方案汇报稿](protobuf-v2-storage-report-outline.md)：V2 历史汇报结构。

术语：**Proto V2 存储**是某一维度目录中的 `manifest.json`、`lines.db`、
`matrices.lmbin` 和 `matrices.lmidx`。`archive` 仅保留在现有 CLI/Rust 类型历史命名中，
业务与文档统一称“Proto V2 存储”。
