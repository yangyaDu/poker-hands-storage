# Proto Range Storage

更新日期：2026-07-16

本目录同时保留当前已发布的 Proto V2 文档，以及当前最高优先级的 Proto V3 业务存储实施方案。
V1 已删除，不能作为兼容目标或实现参考。

V2 的字段号、字段类型和 Protobuf 注释以
[`compact_matrix.proto`](../../storage-tools/proto/zenithstrat/gto/v2/compact_matrix.proto) 为准。
V3 的目标 schema、文件布局、迁移和验证要求以 V3 实施方案为准；实现仍位于
[`storage-tools/src/proto_range_storage/`](../../storage-tools/src/proto_range_storage/)。

文档与实现冲突时必须修正其中一方，不能另起一套格式说明。

1. [Proto V3 业务存储实施方案](v3-business-storage-implementation-plan.md)：当前最高优先级；定义 drill、action path、hand strategy 三条业务查询链及迁移验证。
2. [V2 格式规范](v2-format.md)：当前已发布 V2 的字段、位图、量化、文件布局和校验。
3. [运行时与查询](runtime-and-query.md)：当前 V2 解码索引、缓存、metadata 和 core 接口。
4. [导出与基准](export-and-benchmark.md)：导出、验证、三方基准及报告口径。
5. [Cache 与 Decode 优化实践方案](cache-and-decode-optimization-plan.md)：V2 的观测、容量扫描、预热与 decode accelerator 参考。
6. [Replay 内存基准设计](replay-memory-benchmark-design.md)：尚未实施的真实业务 replay、等价缓存和 RSS 测量设计。
7. [Proto V2 存储方案汇报稿](protobuf-v2-storage-report-outline.md)：面向汇报的主题结构、文件结构和真实业务时序图。

术语：**Proto V2 存储**是某一维度目录中的 `manifest.json`、`lines.db`、
`matrices.lmbin` 和 `matrices.lmidx`。`archive` 仅保留在现有 CLI/Rust 类型历史命名中，
业务与文档统一称“Proto V2 存储”。
