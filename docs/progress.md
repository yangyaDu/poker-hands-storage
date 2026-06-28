# Poker Hands Storage 实施进度

更新时间：2026-06-28

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
| Phase 6 完整验收 | 完成 | 真实进程 HTTP smoke、全量 9 维度构建、API 契约化、Rust verifier、hot/sqlite/compare/cold benchmark 正式报告、Docker 全量容器验收均已通过 |
| Workspace 重构 Phase 0-9 | 完成 | 三 crate 拆分完成，service 纯 API，storage-tools 负责全部离线工具，range-store-core 提供共享核心 |

## Workspace 结构

```text
range-store-core/
  src/
    idx_reader.rs         .idx mmap reader
    bin_reader.rs         .bin mmap reader
    pack_codec.rs         Pack encoder/decoder
    crc32c.rs             CRC32C checksum
    types.rs              Shared types
    dimension_reader.rs   Dimension-level reader
    dimension.rs          Dimension naming, DimensionRef
    hole_cards.rs         Hole-card parsing and dictionary
    action_schema.rs      Action schema codec, load_action_schemas
    sqlite.rs             Dynamic SQLite connection
    query/                StoreQueryService, HandlePool (core query, no HTTP)

service/
  src/
    config/               Environment-based service configuration
    errors/               Unified AppError type
    http/                 Axum server setup, OpenAPI, validation
    query/                HTTP-aware QueryService and handle pool
    routes/               HTTP route handlers
    storage/              Manifest reader, metadata DB

storage-tools/
  src/
    benchmark/            Hot/cold/SQLite/compare benchmark runners
      hot/                Hot-path benchmark runner
      cold/               Cold-start benchmark runner and workers
      sqlite/             SQLite baseline benchmark runner
      compare/            Binary vs SQLite comparison
    range_store_builder/  SQLite source → PFSP/PFXI binary build flow
    verification/         Standalone and source-cross verification reports
      standalone/         Standalone verification
      cross/              Source-cross verification
      report/             Report generation
```

## 构建命令

```text
poker-hands-storage-tools build
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
- `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
- `cargo test --workspace --target x86_64-pc-windows-msvc`
- workspace 测试通过，其中包含 SQLite → binary store → QueryService、benchmark hot/sqlite/compare runner
  和 axum Router 端到端测试。
- 真实 `range.db` smoke：`default:6:100`，2 packs。
- smoke 输出：`.bin` 9818 bytes，`.idx` 60 bytes。
- `AA`、concrete line 1、checksum 查询与源 DB 的 float32 结果一致。
- 上游 standalone verifier：manifest/catalog/index/pack 全部通过，0 failure。
- 使用真实 `data/smoke` 启动 HTTP 进程，`/health`、`/ready`、
  `/range/hand-strategy`、`/range/hand-strategy-batch` 通过。
- 原始 SQLite 已复制到 `data/sqlite/range.db`，与源文件 SHA-256 一致。
- `docker compose up --build -d` 容器 smoke 通过，`/health`、`/ready`、
  `/range/hand-strategy` 返回正常，compose health 状态为 `healthy`。
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
- Rust verifier 正式报告已刷新：
  `reports/range-strata-verify-standalone.json/.md` 覆盖全量 9 个维度，
  manifest/catalog/index/pack 全部通过，total failures 0；
  `reports/range-strata-verify-cross.json/.md` 使用 `--sample-size 10000`
  对 `data/sqlite/range.db` 做 sampled cross check，实际检查 9996 条源记录，
  source records failed 0，extra binary records 0，total failures 0。
- Rust hot benchmark 7c 已接入 CLI：workload 生成/读取、random/abstract-local、
  hand-strategy、batch-hand-strategy、多 batch-size、warmup、QPS/avg/p50/p95/p99/max、
  errorCount/resultCount、内存近似、JSON/Markdown report、`--verify-results`
  action-count 对账和 `--write-workload` release workload 写出。
- 真实 `data/range-strata` benchmark smoke 通过：8 个 case、55 次迭代、0 error、
  result verification 20 match / 0 mismatch / 0 errors，报告写入
  `reports/benchmark-range-strata-binary-smoke.json` 和
  `reports/benchmark-range-strata-binary-smoke.md`。
- Rust benchmark 7d/7e 已接入 CLI：`benchmark-cold` 输出 process/store/query
  分层冷启动报告；`benchmark-sqlite` 复用 workload 跑 SQLite baseline；
  `benchmark-compare` 默认拒绝 workload mismatch 并输出 JSON/Markdown 对比报告。
- Phase 8 benchmark 正式报告已刷新：`reports/release-workload.json` 使用
  `abstract-local` 共享 workload；binary 报告 `reports/benchmark-range-strata-binary.json/.md`
  覆盖 8 个 case、2400 次迭代、error count 0、result action count 135531；
  SQLite 报告 `reports/benchmark-sqlite.json/.md` 复用同一 workload，8 个 case、
  2400 次迭代、error count 0、result action count 135531；compare 报告
  `reports/benchmark-compare.json/.md` 显示 `compatibleWorkload = true`，
  compatibility notes 0，所有 case result match。
- Phase 8 cold benchmark 正式报告已刷新：`reports/benchmark-cold-start.json/.md`
  使用 `process-cold`、固定 `concrete_line_id = 1` / `AA`，覆盖 9 个维度、
  每维度 10 runs、总 90 runs、errors 0。报告区分
  `storeOpenAndFirstQueryMs`、`workerTotalMs`、`processElapsedMs`，聚合
  store open + first query p50/p95 为 324.23 ms / 362.10 ms，
  process elapsed p50/p95 为 357.61 ms / 391.53 ms。
- Phase 8 Docker 全量容器验收已通过：`docker compose -f .docker/docker-compose.yml up --build -d`
  使用 Linux builder 重新编译 release binary，并挂载全量 `data/range-strata:/data:ro`。
  容器内 `/health` 返回 `ok`，`/ready` 返回 9 个维度和 `schema_count = 19404`。
  `/range/hand-strategy`、`/range/hand-strategy-batch`、`/range/prewarm`、
  `/range/concrete-lines`、`/range/drill-scenarios` 以及 OpenAPI paths
  均通过 smoke；`/range/drill-scenarios` 使用 `vsSqueeze / 9 / 100`
  作为非空样本。

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
  Windows 可通过 `PHS_SQLITE3_LIB` 指定 `sqlite3.dll`。Phase 8 smoke 发现
  默认 DLL 解析可能触发 SQLite `disk I/O error`，发布验证应固定到已知 64-bit DLL。
- 全量 9 个维度数据已构建并通过上游 standalone verifier；Rust standalone/cross verifier
  正式报告已刷新；hot/sqlite/compare/cold benchmark 正式报告已刷新；Docker 全量容器验收
  已通过。
- 容器 smoke 已使用 `data/smoke` 和全量 `data/range-strata` 验证；如果 Docker 配置
  或 runtime 镜像后续变更，需要重新跑全量容器验收。

## 容器化配置

- `Dockerfile` 使用 multi-stage 构建 release binary，runtime 基于 distroless
  Debian 12。
- runtime 从构建阶段复制 `libsqlite3.so.0`，提供动态加载所需的 SQLite 共享库。
- Linux 发布验收以 Docker build/run 为准；WSL `/home/ubuntu2204` 可作为本机 Linux
  调试和 benchmark 环境，但不是 Docker 发布的必需前置步骤。
- 容器默认执行 `poker-hands-storage-service serve`，监听 `0.0.0.0:8080`。
- `docker-compose.yml` 默认将 `data/range-strata` 挂载为 `/data:ro`，可通过
  `PHS_HOST_DATA_DIR` 覆盖，开启 checksum，并预热 `default:6:100`。
- 镜像内置 `/health` healthcheck；compose 使用 `/ready` 作为更严格的服务检查。

## API 契约化

- `utoipa` 生成 OpenAPI schema，`GET /swagger` 暴露 Scalar API Reference。
- `GET /api-docs/openapi.json` 提供原始 OpenAPI JSON。
- 所有 HTTP 接口已加入 request/response schema、字段说明和错误响应声明。
- `ValidatedJson<T>` 统一处理 JSON 解析和请求体校验。
- 当前运行时校验覆盖非空字符串、正整数、非空数组、batch 最大 500 项、prewarm
  最大 64 个维度。

## 下一步

1. 如需发布候选加严，运行 full cross verifier（`--sample-size 0`）。
2. 如 Docker 配置或 runtime 镜像变更，重新运行全量 `data/range-strata` 容器验收。
3. 提交、打 tag 或发布镜像前需要用户确认。
