# Proto V2 Range Storage

更新日期：2026-07-14

本目录是当前 Proto V2 Range Storage 的唯一文档入口，只描述
`zenithstrat.gto.v2.CompactLineMatrix`。V1 已删除，不能作为兼容目标或实现参考。

字段号、字段类型和 Protobuf 注释以
[`compact_matrix.proto`](../../storage-tools/proto/zenithstrat/gto/v2/compact_matrix.proto)
为准；本目录定义它在本仓库中的存储语义、校验规则和查询行为。实现位于
[`storage-tools/src/proto_range_storage/`](../../storage-tools/src/proto_range_storage/)。

文档与实现冲突时必须修正其中一方，不能另起一套格式说明。

1. [V2 格式规范](v2-format.md)：字段、位图、量化、文件布局和校验。
2. [运行时与查询](runtime-and-query.md)：解码索引、缓存、metadata 和 core 接口。
3. [导出与基准](export-and-benchmark.md)：导出、验证、三方基准及报告口径。
4. [Cache 与 Decode 优化实践方案](cache-and-decode-optimization-plan.md)：观测、容量扫描、预热与 decode accelerator 的决策门槛。

术语：**Proto V2 存储**是某一维度目录中的 `manifest.json`、`lines.db`、
`matrices.lmbin` 和 `matrices.lmidx`。`archive` 仅保留在现有 CLI/Rust 类型历史命名中，
业务与文档统一称“Proto V2 存储”。
