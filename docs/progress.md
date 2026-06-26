# Poker Hands Storage 实施进度

更新时间：2026-06-26

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
| Phase 6 完整验收 | 进行中 | 47 个测试、真实进程 HTTP smoke、容器 smoke、全量 9 维度构建、API 契约化和全量 standalone verifier 通过；Rust verifier 7a/7b 已接入，benchmark 迁移待完成 |

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
  config/
  domain/
  errors/
  http/
  query/
  range_store_builder/
  routes/
    health_routes.rs
    hand_query_routes.rs
    metadata_routes.rs
  scripts/
  storage/
  verification/
    catalog_checks.rs
    float32_precision.rs
    cross/
    report/
    standalone/
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
- 全量 `data/sqlite/range.db` 构建到 `data/range-strata` 通过，用时约 2 分 12 秒。
- 全量输出包含 9 个维度，总大小 362,296,945 bytes（345.51 MiB），其中 `.bin`
  合计 272,110,768 bytes，`.idx` 合计 11,465,092 bytes，`meta.db`
  78,716,928 bytes。
- 9 个维度均通过 `concrete_line_id = 1`、`hole_cards = AA`、checksum 查询抽查。
- Phase 6a API 契约化已完成：`/swagger` Scalar API Reference、`/api-docs/openapi.json`
  OpenAPI 文档、请求体字段注释和统一 validation error 已接入。
- 上游 `preflop-storage` standalone verifier 已对全量 `data/range-strata` 通过 CRC 校验：
  9 个维度、manifest OK、catalog OK、index files 9/9、pack files 9/9、index-pack
  cross failures 0、total failures 0。
- verifier 报告已写入 `reports/range-strata-verify-standalone.json` 和
  `reports/range-strata-verify-standalone.md`。

## 全量构建结果

| 维度 | packs | `.bin` bytes | `.idx` bytes |
|------|------:|-------------:|-------------:|
| default:6:100 | 3,737 | 2,172,204 | 82,230 |
| default:6:200 | 2,363 | 1,666,509 | 52,002 |
| default:6:300 | 1,816 | 1,390,341 | 39,968 |
| default:8:100 | 8,892 | 4,635,494 | 195,640 |
| default:8:200 | 5,454 | 3,438,513 | 120,004 |
| default:8:300 | 3,643 | 2,865,913 | 80,162 |
| default:9:100 | 197,087 | 83,756,612 | 4,335,930 |
| default:9:200 | 203,028 | 108,969,070 | 4,466,632 |
| default:9:300 | 95,114 | 63,216,112 | 2,092,524 |

## 当前注意事项

- Windows 默认 GNU target 会误用 32 位 `dlltool`，本机统一使用
  `x86_64-pc-windows-msvc`。
- SQLite 通过 `libloading` 动态加载。容器需要提供 `libsqlite3.so.0`；
  Windows 可通过 `PHS_SQLITE3_LIB` 指定 `sqlite3.dll`。
- 全量 9 个维度数据已构建并通过上游 standalone verifier；Rust standalone/cross verifier
  已接入 CLI，真实全量报告需在发布验证链路中刷新。
- 容器 smoke 已使用 `data/smoke` 验证；全量 `data/range-strata` 挂载仍需在 Phase 6 覆盖。

## 容器化配置

- `Dockerfile` 使用 multi-stage 构建 release binary，runtime 基于 Debian slim。
- runtime 安装 `libsqlite3-0`，提供动态加载所需的 `libsqlite3.so.0`。
- 容器默认执行 `poker-hands-storage-service serve`，监听 `0.0.0.0:8080`。
- `docker-compose.yml` 将 `./data/smoke` 挂载为 `/data:ro`，开启 checksum，并预热
  `default:6:100`。
- 镜像内置 `/health` healthcheck；compose 使用 `/ready` 作为更严格的服务检查。

## API 契约化

- `utoipa` 生成 OpenAPI schema，`GET /swagger` 暴露 Scalar API Reference。
- `GET /api-docs/openapi.json` 提供原始 OpenAPI JSON。
- 所有 HTTP 接口已加入 request/response schema、字段说明和错误响应声明。
- `ValidatedJson<T>` 统一处理 JSON 解析和请求体校验。
- 当前运行时校验覆盖非空字符串、正整数、非空数组、batch 最大 500 项、prewarm
  最大 64 个维度。

## 下一步

1. 刷新 Rust verifier standalone/cross 报告。
2. 迁移 Rust benchmark / benchmark-cold / benchmark-compare。
3. 使用全量 `data/range-strata` 挂载运行容器验收。
