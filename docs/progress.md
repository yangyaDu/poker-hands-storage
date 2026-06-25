# Poker Hands Storage 实施进度

更新时间：2026-06-25

## 当前状态

| 阶段 | 状态 | 结果 |
|------|------|------|
| Phase 0 上游数据确认 | 完成 | 原有 9 个维度数据包 standalone verify 通过 |
| Phase 1 项目骨架 | 完成 | Rust workspace、core/service crates、README 已建立 |
| Phase 2 核心 reader | 完成 | `.idx/.bin` mmap reader、pack decoder、CRC32C、DimensionReader 已迁移 |
| Phase 3 服务核心 | 进行中 | manifest、hand、action schema、metadata、LRU handle pool、QueryService 已实现；config 与 HTTP 接线待 Phase 4 前补齐 |
| Phase 3 离线构建扩展 | 完成 | `build` 可从旧 SQLite DB 生成 `manifest.json + meta.db + .idx + .bin` |
| Phase 4 HTTP | 未开始 | axum routes、配置与进程生命周期待实现 |
| Phase 5 容器化 | 未开始 | Dockerfile、compose、只读 volume 待实现 |
| Phase 6 完整验收 | 进行中 | 单元/端到端测试通过，真实 smoke 数据已生成 |

## 已实现模块

```text
crates/range-store-core/src/
  idx_reader.rs
  bin_reader.rs
  pack_codec.rs
  crc32c.rs
  types.rs
  dimension_reader.rs

crates/service/src/
  action_schema.rs
  builder.rs
  error.rs
  hand_dict.rs
  manifest.rs
  meta_db.rs
  naming.rs
  pool.rs
  query_service.rs
  sqlite.rs
  main.rs
```

## 构建命令

```text
poker-hands-storage-service build
  --source-db <range.db>
  --out-dir <output>
  [--dimension default:6:100]
  [--max-concrete-lines 2]
  [--overwrite]
```

构建行为：

- 发现 `range_data_*` 且存在对应 `concrete_lines_*` 的维度。
- 复制 drill/concrete metadata 到轻量 `meta.db`。
- 按 concrete line 聚合、排序 action、编码 sparse hand-major pack。
- 写 PFSP v1 `.bin`、PFXI v1 `.idx`、CRC32C 和 action schema catalog。
- 先写 `.tmp`，成功后改名，最后写 `manifest.json`。

## 已完成验证

- `cargo fmt --all -- --check`
- `cargo test --workspace --target x86_64-pc-windows-msvc --offline`
- 43 个测试通过，其中包含 SQLite → binary store → QueryService 端到端测试。
- 真实 `range.db` smoke：`default:6:100`，2 packs。
- smoke 输出：`.bin` 9818 bytes，`.idx` 60 bytes。
- `AA`、concrete line 1、checksum 查询与源 DB 的 float32 结果一致。
- 上游 standalone verifier：manifest/catalog/index/pack 全部通过，0 failure。

## 当前注意事项

- Windows 默认 GNU target 会误用 32 位 `dlltool`，本机统一使用
  `x86_64-pc-windows-msvc`。
- SQLite 通过 `libloading` 动态加载。容器需要提供 `libsqlite3.so.0`；
  Windows 可通过 `PHS_SQLITE3_LIB` 指定 `sqlite3.dll`。
- HTTP 层尚未实现，因此 Phase 3 完成不代表服务已可部署。
- 全量 9max 数据构建尚未执行；当前只完成小规模真实 smoke。
