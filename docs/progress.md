# Poker Hands Storage 实施进度

更新时间：2026-06-25

## 当前状态

| 阶段 | 状态 | 结果 |
|------|------|------|
| Phase 0 上游数据确认 | 完成 | 原有 9 个维度数据包 standalone verify 通过 |
| Phase 1 项目骨架 | 完成 | Rust workspace、core/service crates、README 已建立 |
| Phase 2 核心 reader | 完成 | `.idx/.bin` mmap reader、pack decoder、CRC32C、DimensionReader 已迁移 |
| Phase 3 服务核心 | 完成 | manifest、config、hand、action schema、metadata、LRU handle pool、QueryService 已实现 |
| Phase 3 离线构建扩展 | 完成 | `build` 可从旧 SQLite DB 生成 `manifest.json + meta.db + .idx + .bin` |
| Phase 4 HTTP | 完成 | axum 0.8、七个路由、JSON 错误、预热、graceful shutdown 已实现 |
| Phase 5 容器化 | 完成 | multi-stage Dockerfile、compose、只读 volume、healthcheck、启动预热配置和容器 smoke 已通过 |
| Phase 6 完整验收 | 进行中 | 47 个测试、真实进程 HTTP smoke 和容器 smoke 通过；全量构建与跨实现 diff 待完成 |

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
  config.rs
  error.rs
  hand_dict.rs
  http.rs
  manifest.rs
  meta_db.rs
  naming.rs
  pool.rs
  query_service.rs
  routes/
    health.rs
    metadata.rs
    query.rs
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
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc --offline -- -D warnings`
- `cargo test --workspace --target x86_64-pc-windows-msvc --offline`
- 47 个测试通过，其中包含 SQLite → binary store → QueryService 和 axum Router 端到端测试。
- 真实 `range.db` smoke：`default:6:100`，2 packs。
- smoke 输出：`.bin` 9818 bytes，`.idx` 60 bytes。
- `AA`、concrete line 1、checksum 查询与源 DB 的 float32 结果一致。
- 上游 standalone verifier：manifest/catalog/index/pack 全部通过，0 failure。
- 使用真实 `data/smoke` 启动 HTTP 进程，`/health`、`/ready`、`/query`、`/batch` 通过。
- 原始 SQLite 已复制到 `data/sqlite/range.db`，与源文件 SHA-256 一致。
- `docker compose up --build -d` 容器 smoke 通过，`/health`、`/ready`、`/query`
  返回正常，compose health 状态为 `healthy`。

## 当前注意事项

- Windows 默认 GNU target 会误用 32 位 `dlltool`，本机统一使用
  `x86_64-pc-windows-msvc`。
- SQLite 通过 `libloading` 动态加载。容器需要提供 `libsqlite3.so.0`；
  Windows 可通过 `PHS_SQLITE3_LIB` 指定 `sqlite3.dll`。
- 全量 9max 数据构建尚未执行；当前只完成小规模真实 smoke。
- 容器 smoke 已使用 `data/smoke` 验证；全量数据挂载仍需在 Phase 6 覆盖。

## 容器化配置

- `Dockerfile` 使用 multi-stage 构建 release binary，runtime 基于 Debian slim。
- runtime 安装 `libsqlite3-0`，提供动态加载所需的 `libsqlite3.so.0`。
- 容器默认执行 `poker-hands-storage-service serve`，监听 `0.0.0.0:8080`。
- `docker-compose.yml` 将 `./data/smoke` 挂载为 `/data:ro`，开启 checksum，并预热
  `default:6:100`。
- 镜像内置 `/health` healthcheck；compose 使用 `/ready` 作为更严格的服务检查。

## 下一步

1. 执行全量 9 个维度构建。
2. 与上游 standalone verifier 做全量跨实现 diff。
3. 使用全量数据挂载运行容器验收。
