# Workspace 拆分重构方案

## 目标

当前项目要从“两个 crate + service 内包含 API 和工具”的结构，调整为三个平级模块：

```text
poker-hands-storage/
  range-store-core/      # 存储核心能力
  service/               # 后端 API 服务
  storage-tools/         # benchmark / verification / 对比分析 / 存储方案分析
  data/
  docs/
  reports/
  .docker/
  Cargo.toml
  Cargo.lock
```

核心依赖关系固定为：

```text
range-store-core
   ↑          ↑
service   storage-tools
```

约束：

- `service` 不能依赖 `storage-tools`。
- `storage-tools` 不能依赖 `service`。
- 两者共享能力只能来自 `range-store-core`。
- 不为了拆分而拆分，优先保持中等粒度，避免目录数量过多。
- 每一阶段都必须可以独立验证，避免一次性大搬导致上下文过长。

## 职责边界

### range-store-core

提供存储核心能力。它可以包含线上 API 和离线工具都会用到的存储协议、基础模型和底层读写能力。

建议承载：

- `.idx/.bin` mmap reader。
- pack decode、hand id 查询、CRC32C。
- dimension 命名和维度模型。
- hole cards 解析和 169 hand 映射。
- action schema decode。
- manifest 读取和 queryable dimension 解析。
- 轻量 SQLite 动态加载封装，如果 `service` 和 `storage-tools` 都需要访问 `meta.db` 或源 SQLite。
- metadata reader，如果它只是读取存储目录里的 `meta.db`，且同时被 API 和工具使用。

不应该承载：

- HTTP、Swagger、Axum response。
- benchmark report、workload 策略、QPS 统计。
- verification 报告格式。
- CLI 参数解析。
- Docker runtime 逻辑。

### service

只做线上后端 API 服务，按 MVC-ish 分层理解：

```text
service/src/
  config/
  errors/
  http/          # router/server/response/error/openapi/healthcheck
  routes/        # Controller: HTTP handler + request DTO + validation
  query/         # Service/Application: API 查询业务
  storage/       # 仅保留 API runtime 特有的存储访问
  main.rs        # serve / healthcheck / help
```

长期目标：

- `poker-hands-storage-service serve`
- `poker-hands-storage-service healthcheck`

不再承担：

- benchmark。
- verification。
- cold-start compare。
- SQLite vs binary compare。
- 离线 build store。

### storage-tools

离线工具和研发分析工具。它是正式工具 crate，不是临时脚本目录。

建议承载：

```text
storage-tools/src/
  benchmark/
    hot/
    cold/
    sqlite/
    compare/

  verification/
    standalone/
    cross/
    report/

  analysis/
    storage_layout.rs
    sqlite_vs_binary.rs

  range_store_builder/

  cli/
    benchmark.rs
    benchmark_cold.rs
    benchmark_sqlite.rs
    benchmark_compare.rs
    verify.rs
    build.rs

  main.rs
```

长期目标命令：

```text
poker-hands-storage-tools build ...
poker-hands-storage-tools verify ...
poker-hands-storage-tools benchmark ...
poker-hands-storage-tools benchmark-sqlite ...
poker-hands-storage-tools benchmark-compare ...
poker-hands-storage-tools benchmark-cold ...
poker-hands-storage-tools benchmark-sqlite-cold ...
poker-hands-storage-tools benchmark-cold-compare ...
```

## 实施原则

1. 每次只做一个 phase。
2. 每个 phase 都保持行为不变，除非该 phase 明确是 CLI 迁移。
3. 每个 phase 结束都跑完整验证。
4. 不新增过细目录；一个目录至少应该代表一个稳定业务区域。
5. 不做“一个 struct 一个文件”的拆分。
6. 跨 crate 移动前先下沉共享依赖，避免 `storage-tools` 临时依赖 `service`。
7. 如果某一步需要临时兼容 re-export，必须在同一阶段或下一阶段清掉。

## Phase 0：基线确认 ✅

目的：确认当前仓库是可验证状态。

操作：

- 查看 `git status --short`。
- 跑 workspace validate。
- 确认 Docker 可以重建并启动。
- 确认 `/ready` 返回 `code = 0`。

验证：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
docker compose -f .docker\docker-compose.yml up --build -d
GET http://127.0.0.1:8080/ready
```

完成标准：

- 本阶段不改代码。
- 记录当前 Docker 和测试均通过。

## Phase 1：去掉 crates 目录 ✅

目的：先把最终顶层形态定下来，不改模块职责。

移动：

```text
crates/range-store-core -> range-store-core
crates/service          -> service
```

更新：

- 根 `Cargo.toml` workspace members：

```toml
[workspace]
members = [
    "range-store-core",
    "service",
]
resolver = "2"
```

- `service/Cargo.toml` 中 `range-store-core` 的 path：

```toml
range-store-core = { path = "../range-store-core" }
```

- `.docker/Dockerfile`：

```text
COPY range-store-core ./range-store-core
COPY service ./service
```

- README、docs、测试配置、CI 文档中的 `crates/service` 和 `crates/range-store-core` 路径。

不做：

- 不新增 `storage-tools`。
- 不移动 benchmark/verification。
- 不改 CLI 行为。

验证：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
docker compose -f .docker\docker-compose.yml up --build -d
```

完成标准：

- `range-store-core` 和 `service` 已在根目录平级。
- Docker 镜像能重新构建。
- API `/ready` 正常。

## Phase 2：新增空 storage-tools crate ✅

目的：建立第三个平级 crate，但暂不迁移业务。

新增：

```text
storage-tools/
  Cargo.toml
  src/
    main.rs
```

根 `Cargo.toml`：

```toml
[workspace]
members = [
    "range-store-core",
    "service",
    "storage-tools",
]
resolver = "2"
```

`storage-tools/Cargo.toml`：

```toml
[package]
name = "poker-hands-storage-tools"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
range-store-core = { path = "../range-store-core" }
```

初始 CLI：

```text
poker-hands-storage-tools help
```

不做：

- 不依赖 `poker-hands-storage-service`。
- 不迁移 benchmark/verification。

验证：

```text
cargo tree -p poker-hands-storage-tools
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

完成标准：

- `cargo tree -p poker-hands-storage-tools` 中没有 `poker-hands-storage-service`。
- workspace 验证通过。

## Phase 3：下沉共享存储能力到 range-store-core ✅

目的：为 `service` 和 `storage-tools` 解耦做准备。

优先下沉稳定共享模块：

```text
service/src/domain/dimension.rs      -> range-store-core/src/dimension.rs
service/src/domain/hole_cards.rs     -> range-store-core/src/hole_cards.rs
service/src/domain/action_schema.rs  -> range-store-core/src/action_schema.rs
service/src/storage/manifest/*       -> range-store-core/src/manifest/
```

可能下沉，但需要单独确认：

```text
service/src/storage/sqlite/*
service/src/storage/metadata/*
```

判断标准：

- 如果 API 和 tools 都需要同一套 SQLite 动态加载能力，则放入 `range-store-core`。
- 如果只是某个工具的 source DB 查询逻辑，则留在 `storage-tools`。
- 如果只是 API runtime 的 metadata 查询逻辑，则留在 `service`。

更新：

- `service` 从 `range_store_core::dimension`、`range_store_core::hole_cards`、`range_store_core::action_schema`、`range_store_core::manifest` 引用。
- 删除或收缩 `service/src/domain` 中已下沉的模块。

不做：

- 不迁移 benchmark。
- 不迁移 verification。
- 不迁移 builder。

验证：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

完成标准：

- `service` 不再拥有已下沉的共享存储模型。
- `range-store-core` 仍不依赖 `service`。

## Phase 4：让 service 测试脱离 range_store_builder ✅

目的：为后续把 builder 移到 `storage-tools` 做准备。

当前风险：

- `service` 的 HTTP/router 测试可能使用 `range_store_builder` 构建临时测试 store。
- 如果直接把 builder 移到 `storage-tools`，`service` 测试会出现反向依赖压力。

处理方式：

- 在 `service/tests/support/` 中准备专用 fixture 生成逻辑。
- fixture 只使用 `range-store-core` 的底层写入/格式能力，或写入固定最小 `.idx/.bin/meta.db/manifest.json` 测试数据。
- `service` 测试不依赖 `storage-tools`。
- `service` 测试不依赖 `range_store_builder`。

不做：

- 不迁移 builder。
- 不迁移 benchmark/verification。

验证：

```text
cargo test -p poker-hands-storage-service --target x86_64-pc-windows-msvc
cargo test --workspace --target x86_64-pc-windows-msvc
```

完成标准：

- `rg "range_store_builder" service/tests service/src` 不再命中 API 测试依赖。
- `service` 仍然可以独立跑 API 测试。

## Phase 5：迁移 range_store_builder 到 storage-tools ✅

目的：把离线构建工具从 API 服务中移走。

移动：

```text
service/src/range_store_builder -> storage-tools/src/range_store_builder
service/src/scripts/build*.rs   -> storage-tools/src/cli/build.rs
```

CLI 变化：

```text
# 迁移前
poker-hands-storage-service build ...

# 迁移后
poker-hands-storage-tools build ...
```

`service` 删除：

- `build` command。
- `range_store_builder` module。

不做：

- 不迁移 benchmark。
- 不迁移 verification。

验证：

```text
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- build --help
cargo test -p poker-hands-storage-tools --target x86_64-pc-windows-msvc
cargo test --workspace --target x86_64-pc-windows-msvc
```

完成标准：

- `service` 不再包含 build store 能力。
- `storage-tools` build 命令行为与旧命令一致。
- `storage-tools` 不依赖 `service`。

## Phase 6：迁移 verification 到 storage-tools ✅

目的：把 standalone/cross verifier 和报告生成移出 API 服务。

移动：

```text
service/src/verification -> storage-tools/src/verification
service/src/scripts/verify_store.rs -> storage-tools/src/cli/verify.rs
service/tests/verification -> storage-tools/tests/verification
service/tests/scripts/verify_store.test.rs -> storage-tools/tests/cli/verify.test.rs
```

CLI 变化：

```text
# 迁移前
poker-hands-storage-service verify ...

# 迁移后
poker-hands-storage-tools verify ...
```

验证：

```text
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify --mode standalone --dir data/range-strata --verify-checksum
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify --mode cross --dir data/range-strata --source data/sqlite/range.db --sample-size 10000 --verify-checksum
cargo test --workspace --target x86_64-pc-windows-msvc
```

完成标准：

- `service` 没有 verifier 模块和 verify 命令。
- verify 报告 JSON/Markdown shape 不变。
- `storage-tools` 不依赖 `service`。

## Phase 7：迁移 benchmark 和对比分析到 storage-tools ✅

目的：把 hot benchmark、SQLite benchmark、compare、cold-start compare 全部移出 API 服务。

移动：

```text
service/src/benchmark -> storage-tools/src/benchmark
service/src/scripts/benchmark*.rs -> storage-tools/src/cli/
service/tests/benchmark -> storage-tools/tests/benchmark
service/tests/scripts/benchmark*.test.rs -> storage-tools/tests/cli/
```

内部命名中等粒度即可：

```text
storage-tools/src/benchmark/
  hot/
  cold/
  sqlite/
  compare/
  workload.rs
  metrics.rs
  report.rs
```

`cold/runner.rs` 可以顺手改成 `cold/binary_runner.rs`，因为这里已经同时存在 SQLite cold runner。

CLI 变化：

```text
poker-hands-storage-tools benchmark ...
poker-hands-storage-tools benchmark-sqlite ...
poker-hands-storage-tools benchmark-compare ...
poker-hands-storage-tools benchmark-cold ...
poker-hands-storage-tools benchmark-sqlite-cold ...
poker-hands-storage-tools benchmark-cold-compare ...
```

验证：

```text
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark --help
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-sqlite --help
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold --help
cargo test --workspace --target x86_64-pc-windows-msvc
```

正式 benchmark 可在该阶段最后单独跑，不和代码迁移混在一起。

完成标准：

- `service` 没有 benchmark 模块和 benchmark 命令。
- 已有 benchmark report 格式保持兼容。
- `storage-tools` 不依赖 `service`。

## Phase 8：收敛 service 为纯 API 服务

目的：最终清理 `service`。

`service/main.rs` 只保留：

```text
serve
healthcheck
help
```

`service/src/` 只保留 API runtime 所需模块：

```text
service/src/
  config/
  errors/
  http/
  query/
  routes/
  storage/
  main.rs
  lib.rs
```

删除：

- `service/src/scripts`，如果已经没有剩余命令解析职责。
- build/verify/benchmark/cold worker 子命令。

验证：

```text
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- help
cargo test -p poker-hands-storage-service --target x86_64-pc-windows-msvc
cargo test --workspace --target x86_64-pc-windows-msvc
docker compose -f .docker\docker-compose.yml up --build -d
```

完成标准：

- `poker-hands-storage-service help` 只展示 API 服务相关命令。
- Docker 只构建和运行 `service`。
- `/ready` 正常。

## Phase 9：文档和命令入口收尾

目的：让 README、progress、Docker 文档和 benchmark 文档全部匹配新结构。

更新：

- README 项目结构。
- `docs/progress.md` 已实现模块列表。
- Docker 构建说明。
- benchmark/verification 使用说明。
- 报告生成命令。

旧命令迁移表：

| 旧命令 | 新命令 |
|---|---|
| `poker-hands-storage-service serve` | 不变 |
| `poker-hands-storage-service healthcheck` | 不变 |
| `poker-hands-storage-service build` | `poker-hands-storage-tools build` |
| `poker-hands-storage-service verify` | `poker-hands-storage-tools verify` |
| `poker-hands-storage-service benchmark` | `poker-hands-storage-tools benchmark` |
| `poker-hands-storage-service benchmark-sqlite` | `poker-hands-storage-tools benchmark-sqlite` |
| `poker-hands-storage-service benchmark-compare` | `poker-hands-storage-tools benchmark-compare` |
| `poker-hands-storage-service benchmark-cold` | `poker-hands-storage-tools benchmark-cold` |
| `poker-hands-storage-service benchmark-sqlite-cold` | `poker-hands-storage-tools benchmark-sqlite-cold` |
| `poker-hands-storage-service benchmark-cold-compare` | `poker-hands-storage-tools benchmark-cold-compare` |

验证：

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
docker compose -f .docker\docker-compose.yml up --build -d
```

完成标准：

- 文档里不再把 benchmark/verification 归到 `service`。
- 文档里不再引用旧的 `crates/service` 和 `crates/range-store-core` 路径。
- 所有新命令示例都能执行。

## 每轮实施建议

为了避免上下文过长，每次只执行一个 phase。

每轮输出固定包含：

```text
完成的 phase
改动文件
行为变化
验证命令和结果
下一 phase 建议
```

如果某个 phase 中间发现共享依赖比预期复杂，停止继续大搬，先把该 phase 拆成更小的准备步骤。

## 风险清单

| 风险 | 处理方式 |
|---|---|
| `storage-tools` 临时依赖 `service` | 禁止；用 `cargo tree` 和 `rg "poker_hands_storage_service" storage-tools` 验证 |
| `service` 测试依赖离线 builder | Phase 4 先移除该依赖 |
| 目录移动导致 Docker build 失败 | Phase 1 单独验证 Docker |
| report shape 被迁移改坏 | verification/benchmark 迁移阶段保留现有 snapshot/shape 测试 |
| `range-store-core` 变成杂物 crate | 只下沉两边共享且与存储协议/读写相关的能力 |
| 一次移动太多文件 | 每次只做一个 phase，完成后再继续 |

## 最终验收标准

- 根目录下存在 `range-store-core`、`service`、`storage-tools` 三个平级 crate。
- 根目录下不再有 `crates/`。
- `service` 和 `storage-tools` 没有互相依赖。
- `service` 只负责 API 服务。
- `storage-tools` 负责 benchmark、verification、cold-start compare、SQLite vs binary compare、存储方案分析。
- `range-store-core` 提供两边共享的存储核心能力。
- workspace validate 全部通过。
- Docker image 可重建，容器可启动，`/ready` 正常。
