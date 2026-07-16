# poker-hands-storage

独立的 Rust 存储与查询服务。默认 HTTP service 与 Bun/Node native SDK 读取 Proto V3 业务存储。

一个 V3 根目录包含若干维度子目录；每个维度固定由七个文件组成：

- `manifest.json`
- `drill-scenarios.pb` / `drill-scenarios.idx`
- `abstract-action-paths.pb` / `abstract-action-paths.idx`
- `hand-strategies.pb` / `hand-strategies.idx`

当前主链路：

```text
源 SQLite -> 离线导出与 SQLite cross verify -> Proto V3 root -> HTTP service / Bun native SDK
```

线上运行时不需要源 SQLite、`meta.db`、`lines.db` 或 Proto V2 产物。V2 代码和文档仅保留为实现参考，
V3 reader 不读取 V2，也没有 V2/V3 双读或回退路径。

## 模块职责

| 路径 | 对外能力 | 说明 |
| --- | --- | --- |
| `range-store-core` | 共享领域类型 | 提供维度、手牌、查询契约、SQLite 离线访问和历史 Binary 参考实现 |
| `service` | HTTP API 服务 | 默认使用 V3 facade，提供 OpenAPI、错误映射、health/readiness 和 Docker 入口 |
| `range-store-native` | Bun/Node 进程内 SDK | 默认使用同一 V3 facade，失败抛出 `RangeStoreError` |
| `storage-tools` | V3 存储与离线工具 | 提供 V3 export、reader、facade、standalone/cross verify 和 SQLite/V3 benchmark |
| `.docker` | HTTP service 容器化 | Dockerfile 构建 V3 runtime 所需的 `storage-tools` library + `service`，不包含 CLI 或 native SDK 二进制 |
| `docs` | 项目文档 | 入口见 [docs/README.md](docs/README.md) |

## 项目目录树

下表列出主要维护入口和核心源码文件；`target/`、完整测试文件清单、生成报告和大数据文件不展开。

```text
poker-hands-storage/
|-- Cargo.toml                         # Rust workspace 配置，声明 4 个 member crate
|-- Cargo.lock                         # Rust 依赖锁定文件
|-- rustfmt.toml                       # Rust 格式化规则
|-- README.md                          # 项目入口、模块职责、常用命令和文档入口
|-- CHANGELOG.md                       # 项目阶段性变更记录
|-- .gitignore                         # Git 忽略规则，排除 target/data/reports 等生成内容
|-- .gitattributes                     # Git 文件属性和换行策略
|
|-- .agents/
|   `-- poker-hands-storage/           # 与 skill name 一致的项目级 Codex skill
|       |-- SKILL.md                    # AI 编码助手的项目级规则
|       `-- references/                 # 按需加载的构建、验证、benchmark、service 参考
|
|-- .docker/
|   |-- Dockerfile                     # HTTP service 多阶段构建镜像
|   |-- Cargo.service.toml             # Docker 构建专用 workspace，只含 core + V3 library + service
|   |-- docker-compose.yml             # 本地 Compose 启动、只读数据挂载和 healthcheck
|   `-- k8s.yaml                       # Kubernetes 部署模板
|
|-- .githooks/
|   |-- pre-commit                     # 提交前运行格式化、clippy、测试检查
|   |-- pre-push                       # 推送前检查入口
|   |-- post-commit                    # 提交后的辅助钩子
|   |-- post-checkout                  # 切换分支后的辅助钩子
|   `-- post-merge                     # merge 后的辅助钩子
|
|-- range-store-core/
|   |-- Cargo.toml                     # core crate 配置
|   |-- src/
|   |   |-- lib.rs                     # core crate root，声明顶层模块和公共 re-export
|   |   |-- types.rs                   # 存储和查询共享类型
|   |   |-- manifest.rs                # manifest.json 解析和校验
|   |   |-- metadata.rs                # meta.db 读取、维度发现和 metadata lookup
|   |   |-- dimension.rs               # 维度命名、解析和文件名规则
|   |   |-- idx_reader.rs              # PFXI .idx 索引文件 reader
|   |   |-- bin_reader.rs              # PFSP .bin 数据文件 mmap reader
|   |   |-- dimension_reader.rs        # 单维度 idx+bin 组合 reader
|   |   |-- pack_codec.rs              # range pack 编解码
|   |   |-- action_schema.rs           # action schema 解码和动作语义
|   |   |-- hole_cards.rs              # 手牌字符串解析、归一化和 hand_id 映射
|   |   |-- crc32c.rs                  # pack CRC32C 校验
|   |   |-- sqlite.rs                  # SQLite 动态库加载和连接封装
|   |   `-- query/
|   |       |-- mod.rs                  # query 子模块入口，组织内部查询实现
|   |       |-- range_store_facade.rs   # HTTP/native 共用业务 facade
|   |       |-- store_query_service.rs  # 单手牌和批量策略查询服务
|   |       |-- hands_by_actions.rs     # actions/frequency 过滤手牌能力
|   |       `-- handle_pool.rs         # 维度 reader LRU handle pool
|   `-- tests/                         # core 领域、存储和遍历解码测试
|
|-- service/
|   |-- Cargo.toml                     # HTTP service crate 配置
|   |-- src/
|   |   |-- main.rs                    # service 二进制入口，支持 serve/healthcheck
|   |   |-- lib.rs                     # service crate root，声明 HTTP 服务公共模块
|   |   |-- config.rs                  # PHS_* 环境变量解析
|   |   |-- errors.rs                  # AppError 和业务错误映射
|   |   |-- http/                      # axum router、OpenAPI、response、healthcheck
|   |   |-- routes/                    # /range/*、/health、/ready 路由 handler
|   |   `-- query/                     # HTTP 层查询服务和维度 handle pool wrapper
|   `-- tests/                         # HTTP 路由、配置和 service 集成测试
|
|-- range-store-native/
|   |-- Cargo.toml                     # napi-rs native crate 配置
|   |-- build.rs                       # napi-rs 构建初始化
|   |-- package.json                   # Bun 构建和 SDK 测试脚本
|   |-- index.js                       # JS SDK 包装层，加载 index.node 并转换 RangeStoreError
|   |-- index.d.ts                     # TypeScript API 类型声明
|   |-- src/lib.rs                     # N-API 绑定，复用 V3Facade
|   `-- tests/
|       |-- sdk-contract.test.js       # SDK contract 测试
|       `-- http-consistency.test.js   # native SDK 与 HTTP service 抽样一致性测试
|
|-- storage-tools/
|   |-- Cargo.toml                     # 离线工具 crate 配置
|   |-- build.rs                       # 从 matrix.proto 生成 Prost Rust 类型
|   |-- proto/                         # 单行动线 LineMatrix Protobuf schema
|   |-- src/
|   |   |-- main.rs                    # CLI 入口，分发 build/export/verify/benchmark 命令
|   |   |-- lib.rs                     # storage-tools crate root，声明离线工具公共模块
|   |   |-- errors.rs                  # ToolError 错误类型
|   |   |-- metadata.rs                # 构建阶段写入 meta.db
|   |   |-- range_store_builder.rs     # SQLite -> manifest/meta/idx/bin 构建流程
|   |   |-- proto_range_storage/       # Proto LineMatrix payload and archive storage
|   |   |   |-- proto.rs               # Protobuf type definitions
|   |   |   |-- line_matrix_codec.rs   # payload conversion and validation
|   |   |   |-- sqlite_source.rs       # SQLite source query
|   |   |   |-- line_matrix_store.rs   # archive writer and reader
|   |   |   |-- query_service.rs       # single-dimension core-compatible query
|   |   |   |-- query_facade.rs        # multi-dimension query and LRU handle pool
|   |   |   |-- three_way_benchmark.rs # shared Core/Proto/SQLite hot benchmark
|   |   |   |-- three_way_cold_benchmark.rs # fresh-process Core/Proto/SQLite cold benchmark
|   |   |   |-- cli.rs                 # archive CLI argument parsing
|   |   |   |-- format.rs              # archive binary layout
|   |   |   `-- benchmark.rs           # Proto archive versus core benchmark
|   |   |-- verification/              # standalone/cross verify 和验证报告
|   |   |   |-- mod.rs                 # verification 子模块入口，组织验证实现
|   |   |   |-- cli.rs                 # verify --mode standalone|cross 参数解析
|   |   |   |-- standalone.rs          # manifest/header/idx/bin/catalog 自洽检查
|   |   |   |-- cross.rs               # 源 SQLite 与二进制 pack 逐项比对
|   |   |   |-- catalog_checks.rs      # meta.db 表结构、action_schemas、concrete_lines 表检查
|   |   |   |-- float32_precision.rs   # IEEE754 Float32 bit-exact 精度校验
|   |   |   |-- report.rs              # 验证报告 JSON/Markdown 生成
|   |   |-- benchmark/                 # hot/cold/native benchmark
|   |   |   |-- mod.rs                 # benchmark 子模块入口，组织测量实现和运行入口
|   |   |   |-- cli.rs                 # benchmark 参数解析
|   |   |   |-- types.rs               # BenchmarkWorkload 和查询项类型
|   |   |   |-- workload.rs            # workload 生成与 JSON 序列化（跨 benchmark 复用）
|   |   |   |-- metrics.rs             # QPS/latency/percentile 计算
|   |   |   |-- memory_snapshot.rs     # RSS 内存快照
|   |   |   |-- report.rs              # hot/native/cold 等 benchmark 报告 JSON/Markdown 生成
|   |   |   |-- report_support.rs      # benchmark 报告写文件、时间、单位和表格等公共 helper
|   |   |   |-- hot/                   # 热路径 benchmark，包含 Binary、SQLite baseline 和 hot compare
|   |   |   |   |-- runner.rs          # Binary concrete-lines-exact/hand-strategy/batch/hands-by-actions/drill 测量
|   |   |   |   |-- sqlite_runner.rs   # 同 workload 直接查源 range_data/concrete_lines/drill 表
|   |   |   |   |-- compare.rs         # Binary hot vs SQLite hot 对比
|   |   |   |   |-- result_verifier.rs # --verify-results 结果一致性校验
|   |   |   |   |-- types.rs           # Hot/SQLite/Compare 命令和报告类型
|   |   |   |-- cold/                  # 冷启动 benchmark
|   |   |   |   |-- runner.rs          # 多 run 冷启动测量，cache eviction，worker 编排
|   |   |   |   |-- worker.rs          # core worker 进程：打开 store + 查询 + 计时
|   |   |   |   |-- sqlite_worker.rs   # SQLite worker 进程
|   |   |   |   |-- cache_eviction.rs  # OS page cache 驱逐策略
|   |   |   |   |-- compare.rs         # cold Binary vs SQLite 对比
|   |   |   |   |-- types.rs           # ColdWorkerParams/ColdStartBenchmarkReport
|   |   |   |-- native/                # core / SDK / HTTP 三路公平对比
|   |   |   |   |-- runner.rs          # 同 workload 在三个子进程中并行测试
|   |   |   |   |-- types.rs           # BenchmarkNativeCommand 参数
|   |   |   |-- metadata.rs            # drill metadata microbenchmark
|   |   |-- tests/                     # 构建、验证、benchmark CLI 和报告测试
|
|-- docs/
|   |-- README.md                      # 文档地图和阅读路径
|   |-- roadmap.md                     # 剩余工作、验收条件和暂不做事项
|   |-- sdk-and-query-chain-explanation.md # Bun/Node native SDK API、接入边界和查询链路
|   |-- api-business-contract.md       # HTTP API 契约、错误码和业务语义
|   |-- range-db-binary-storage-design.md # 二进制格式、pack 编码和查询流程
|   |-- protobuf-line-matrix-export.md # 单行动线 Protobuf schema、字段语义、导出与校验
|   |-- data-flow-overview.md          # 从构建到查询的代码级数据流
|   |-- verification_and_benchmark.md # 验证覆盖面、Float32 策略、benchmark 脚本介绍和发布前校验
|   |-- binary-vs-sqlite-benchmark-and-verification-report.md # 性能、体积、内存和 benchmark 结论
|   `-- docker-deployment-guide.md     # Docker/Compose/Kubernetes、发布和回滚
|
|-- data/                              # 本地数据目录，通常不提交
|   |-- sqlite/                        # 源 SQLite range.db
|   |-- range-strata/                  # manifest/meta/idx/bin 运行目录
|   `-- smoke/                         # 小规模 smoke 数据
|
|-- reports/                           # benchmark/verify 生成报告，通常不提交
`-- target/                            # Cargo 编译输出，通常不提交
```

## 当前状态

V3 实现状态：

- V3 schema、六个 data/index 文件、manifest、元数据和 HandStrategy reader/writer 已完成。
- standalone verify 与 SQLite 全量 cross verify 已完成，包含 NULL-EV 精确语义。
- V3 facade、按字节预算缓存、HTTP service、native SDK 和 SQLite/V3 benchmark 已接入。
- fixture 端到端和 workspace 回归已通过；真实九维源库仍须在发布环境执行 export + cross gate。

历史 Range Strata Binary 与 Proto V2 仍留在仓库中用于参考和回归，不属于 V3 发布产物。

## 环境准备

| 依赖 | 要求 | 说明 |
| --- | --- | --- |
| Rust toolchain | stable，edition 2021 | Windows 本地命令统一显式指定 MSVC target |
| 编译目标 | `x86_64-pc-windows-msvc` | 禁止使用 GNU target |
| SQLite 动态库 | 64 位 `sqlite3.dll` 或系统动态库 | 仅 `storage-tools` 的 build/verify/benchmark 需要 |
| Bun | 用于 native SDK 构建和测试 | 仅 `range-store-native` 需要 |
| Docker | Docker Desktop 或兼容引擎 | 仅容器化部署需要 |

Windows 本地初始化：

```powershell
rustup target add x86_64-pc-windows-msvc
git config core.hooksPath .githooks
cargo build --workspace --target x86_64-pc-windows-msvc
```

如果 SQLite 动态库无法自动发现：

```powershell
$env:PHS_SQLITE3_LIB = "C:\path\to\sqlite3.dll"
```

## 常用命令

导出并交叉验证一个 V3 维度：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- v3-export `
  --source data\sqlite\range.db `
  --out data\proto-v3\default_6max_100BB `
  --dimension default:6:100
```

导出并交叉验证全部可发现维度：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- v3-export-all `
  --source data\sqlite\range.db `
  --out-root data\proto-v3
```

数据验证：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- v3-verify `
  --archive data\proto-v3\default_6max_100BB

cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- v3-cross-verify `
  --source data\sqlite\range.db `
  --archive data\proto-v3\default_6max_100BB
```

启动 HTTP service：

```powershell
$env:PHS_DATA_DIR = "data\proto-v3"
$env:PHS_PREWARM = "default:6:100"
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- serve
```

Docker 启动 HTTP service：

```powershell
docker compose -f .docker\docker-compose.yml up --build
```

构建和测试 Bun native SDK：

```powershell
Set-Location range-store-native
bun install
bun run build:native
bun run test:sdk
```

## 运行时环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `PHS_BIND` | `0.0.0.0:8080` | HTTP 监听地址 |
| `PHS_DATA_DIR` | `/data` | Proto V3 根目录；直接包含维度子目录 |
| `PHS_MAX_OPEN_HANDLES` | `2` | 最大打开维度 reader 数 |
| `PHS_METADATA_CACHE_BYTES` | `8388608` | 每个维度 metadata page cache 字节预算 |
| `PHS_STRATEGY_CACHE_BYTES` | `67108864` | 每个维度 decoded strategy cache 字节预算 |
| `PHS_VERIFY_CHECKSUMS` | `false` | 打开维度时是否校验六个完整文件 CRC32C |
| `PHS_PREWARM` | 空 | 启动预热维度，格式 `strategy:player_count:depth_bb` |
| `PHS_SQLITE3_LIB` | 自动检测 | 离线工具使用的 SQLite 动态库路径 |
| `RUST_LOG` | `info` | 日志级别 |

## 文档入口

| 文档 | 职责 |
| --- | --- |
| [docs/README.md](docs/README.md) | 文档地图和阅读路径 |
| [docs/roadmap.md](docs/roadmap.md) | 当前剩余工作、验收条件和优先级 |
| [docs/range-db-binary-storage-design.md](docs/range-db-binary-storage-design.md) | 文件格式、pack 编码、查询流程和运行时约束 |
| [docs/protobuf-line-matrix-export.md](docs/protobuf-line-matrix-export.md) | 单行动线 Protobuf schema、字段语义、bitmap、导出和校验 |
| [docs/proto/v3-runtime-and-operations.md](docs/proto/v3-runtime-and-operations.md) | V3 CLI、服务配置、benchmark 与发布门禁 |
| [docs/api-business-contract.md](docs/api-business-contract.md) | HTTP API 请求/响应、错误码和业务语义 |
| [docs/sdk-and-query-chain-explanation.md](docs/sdk-and-query-chain-explanation.md) | Bun/Node native SDK API、构建测试、生产接入边界和查询链路 |
| [docs/verification_and_benchmark.md](docs/verification_and_benchmark.md) | standalone/cross verify、Float32 策略、benchmark 脚本和发布前验证 |
| [docs/binary-vs-sqlite-benchmark-and-verification-report.md](docs/binary-vs-sqlite-benchmark-and-verification-report.md) | 性能、体积、内存和 benchmark 结论 |
| [docs/docker-deployment-guide.md](docs/docker-deployment-guide.md) | Docker/Compose/Kubernetes、发布和回滚 |
| [docs/data-flow-overview.md](docs/data-flow-overview.md) | 构建到查询的代码级数据流速查 |

## 校验

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```
