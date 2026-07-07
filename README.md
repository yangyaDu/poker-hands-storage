# poker-hands-storage

独立的 Rust 存储与查询服务，用于读取 `preflop-storage` 产出的 Range Strata Binary 数据。

当前 V1 运行目录由以下文件组成：

- `manifest.json`（`format = "PFSP"`，`version = 1`）
- `meta.db`
- `ranges_{strategy}_{player_count}max_{depth_bb}BB.idx`
- `ranges_{strategy}_{player_count}max_{depth_bb}BB.bin`

当前主链路：

```text
1.45GB slim SQLite -> 345.5MB Range Strata Binary -> HTTP service / Bun native SDK
```

## 模块职责

| 路径 | 对外能力 | 说明 |
| --- | --- | --- |
| `range-store-core` | Rust 只读查询核心 | 负责 manifest、metadata、`.idx/.bin` reader、pack decode、CRC32C、LRU handle pool 和 `RangeStoreFacade` |
| `service` | HTTP API 服务 | 提供 OpenAPI、请求校验、错误码映射、health/readiness、Docker 运行入口 |
| `range-store-native` | Bun/Node 进程内 SDK | 通过 napi-rs 加载同一套 core 查询能力，成功返回直接 payload，失败抛出 `RangeStoreError` |
| `storage-tools` | 离线工具 | 提供构建、standalone/cross verify、SQLite/Binary/native benchmark 和报告生成 |
| `.docker` | HTTP service 容器化 | Dockerfile 只构建 `range-store-core` + `service`，不包含 benchmark 或 native SDK |
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
|   |-- SKILL.md                       # AI 编码助手的项目级规则
|   `-- references/                    # 按需加载的构建、验证、benchmark、service 参考
|
|-- .docker/
|   |-- Dockerfile                     # HTTP service 多阶段构建镜像
|   |-- Cargo.service.toml             # Docker 构建专用最小 workspace，只含 core + service
|   |-- Cargo.service.lock             # Docker 构建专用依赖锁定文件
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
|   |   |-- lib.rs                     # core 对外模块导出
|   |   |-- types.rs                   # 存储和查询共享类型
|   |   |-- manifest/mod.rs            # manifest.json 解析和校验
|   |   |-- metadata.rs                # meta.db 读取、维度发现和 metadata lookup
|   |   |-- dimension.rs               # 维度命名、解析和文件名规则
|   |   |-- idx_reader.rs              # PFXI .idx 索引文件 reader
|   |   |-- bin_reader.rs              # PFSP .bin 数据文件 mmap reader
|   |   |-- dimension_reader.rs        # 单维度 idx+bin 组合 reader
|   |   |-- pack_codec.rs              # range pack 编解码
|   |   |-- action_schema.rs           # action schema 解码和动作语义
|   |   |-- hole_cards.rs              # 手牌字符串解析、归一化和 hand_id 映射
|   |   |-- crc32c.rs                  # pack CRC32C 校验
|   |   |-- sqlite/mod.rs              # SQLite 动态库加载和连接封装
|   |   `-- query/
|   |       |-- mod.rs                  # query 子模块导出
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
|   |   |-- lib.rs                     # service 库入口和模块导出
|   |   |-- config/                    # PHS_* 环境变量解析
|   |   |-- errors/                    # AppError 和业务错误映射
|   |   |-- http/                      # axum router、OpenAPI、response、healthcheck
|   |   |-- routes/                    # /range/*、/health、/ready 路由 handler
|   |   |-- query/                     # HTTP 层查询服务和维度 handle pool wrapper
|   |   `-- storage/                   # service 侧 metadata 存储入口
|   `-- tests/                         # HTTP 路由、配置和 service 集成测试
|
|-- range-store-native/
|   |-- Cargo.toml                     # napi-rs native crate 配置
|   |-- build.rs                       # napi-rs 构建初始化
|   |-- package.json                   # Bun 构建和 SDK 测试脚本
|   |-- index.js                       # JS SDK 包装层，加载 index.node 并转换 RangeStoreError
|   |-- index.d.ts                     # TypeScript API 类型声明
|   |-- src/lib.rs                     # N-API 绑定，复用 RangeStoreFacade
|   `-- tests/
|       |-- sdk-contract.test.js       # SDK contract 测试
|       `-- http-consistency.test.js   # native SDK 与 HTTP service 抽样一致性测试
|
|-- storage-tools/
|   |-- Cargo.toml                     # 离线工具 crate 配置
|   |-- src/
|   |   |-- main.rs                    # CLI 入口，分发 build/verify/benchmark 命令
|   |   |-- lib.rs                     # storage-tools 库入口
|   |   |-- errors.rs                  # ToolError 错误类型
|   |   |-- metadata.rs                # 构建阶段写入 meta.db
|   |   |-- range_store_builder/       # SQLite -> manifest/meta/idx/bin 构建流程
|   |   |-- verification/              # standalone/cross verify 和验证报告
|   |   `-- benchmark/                 # hot/cold/sqlite/compare/native benchmark
|   `-- tests/                         # 构建、验证、benchmark CLI 和报告测试
|
|-- docs/
|   |-- README.md                      # 文档地图和阅读路径
|   |-- roadmap.md                     # 剩余工作、验收条件和暂不做事项
|   |-- sdk-and-query-chain-explanation.md # Bun/Node native SDK API、接入边界和查询链路
|   |-- api-business-contract.md       # HTTP API 契约、错误码和业务语义
|   |-- range-db-binary-storage-design.md # 二进制格式、pack 编码和查询流程
|   |-- data-flow-overview.md          # 从构建到查询的代码级数据流
|   |-- data-verification-and-format-validation.md # 验证覆盖面和发布前校验
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

已完成：

- 二进制运行格式、构建工具和 `build --resume`。
- HTTP API、OpenAPI、Docker/Compose/Kubernetes 模板。
- Bun/Node native SDK 的 Windows 本地构建、SDK contract 和 HTTP consistency 测试入口。
- full cross verify 覆盖 9 个维度、23,806,716 条源记录，失败数为 0。
- benchmark 覆盖 SQLite vs Binary hot/cold、drill metadata、Rust core、Bun native SDK、HTTP service 和 `concrete_line -> handsByActions` 单链路。

剩余工作只在 [docs/roadmap.md](docs/roadmap.md) 维护，当前主要是完整 `line-transition` benchmark、Linux native SDK 生产接入验证和最终验收边界清单。

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

构建 Range Strata Binary：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- build `
  --source-db data\sqlite\range.db `
  --out-dir data\range-strata `
  --dimension default:6:100 `
  --overwrite
```

数据验证：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --dir data\range-strata `
  --mode standalone `
  --verify-checksum
```

启动 HTTP service：

```powershell
$env:PHS_DATA_DIR = "data\range-strata"
$env:PHS_META_DB = "data\range-strata\meta.db"
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
| `PHS_DATA_DIR` | `/data` | Range Strata 运行目录 |
| `PHS_META_DB` | `${PHS_DATA_DIR}/meta.db` | metadata SQLite 路径 |
| `PHS_MAX_OPEN_HANDLES` | `2` | 最大打开维度 reader 数 |
| `PHS_VERIFY_CHECKSUMS` | `false` | 查询时是否校验 pack CRC32C |
| `PHS_PREWARM` | 空 | 启动预热维度，格式 `strategy:player_count:depth_bb` |
| `PHS_SQLITE3_LIB` | 自动检测 | 离线工具使用的 SQLite 动态库路径 |
| `RUST_LOG` | `info` | 日志级别 |

## 文档入口

| 文档 | 职责 |
| --- | --- |
| [docs/README.md](docs/README.md) | 文档地图和阅读路径 |
| [docs/roadmap.md](docs/roadmap.md) | 当前剩余工作、验收条件和优先级 |
| [docs/range-db-binary-storage-design.md](docs/range-db-binary-storage-design.md) | 文件格式、pack 编码、查询流程和运行时约束 |
| [docs/api-business-contract.md](docs/api-business-contract.md) | HTTP API 请求/响应、错误码和业务语义 |
| [docs/sdk-and-query-chain-explanation.md](docs/sdk-and-query-chain-explanation.md) | Bun/Node native SDK API、构建测试、生产接入边界和查询链路 |
| [docs/data-verification-and-format-validation.md](docs/data-verification-and-format-validation.md) | standalone/cross verify、Float32 策略和发布前验证 |
| [docs/binary-vs-sqlite-benchmark-and-verification-report.md](docs/binary-vs-sqlite-benchmark-and-verification-report.md) | 性能、体积、内存和 benchmark 结论 |
| [docs/docker-deployment-guide.md](docs/docker-deployment-guide.md) | Docker/Compose/Kubernetes、发布和回滚 |
| [docs/data-flow-overview.md](docs/data-flow-overview.md) | 构建到查询的代码级数据流速查 |

## 校验

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```
