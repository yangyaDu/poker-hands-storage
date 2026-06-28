# poker-hands-storage

独立的 Rust 存储与查询服务，用于 Preflop Storage range 数据的高性能读取。

V1 遵循当前 `preflop-storage` Range Strata Binary 契约：

- `manifest.json`（`format = "PFSP"`，`version = 1`）
- `meta.db`
- `ranges_{strategy}_{player_count}max_{depth_bb}BB.idx`
- `ranges_{strategy}_{player_count}max_{depth_bb}BB.bin`

## 环境准备

| 依赖 | 要求 | 说明 |
| --- | --- | --- |
| Rust toolchain | stable（edition 2021） | 安装方式见 [rustup.rs](https://rustup.rs) |
| 编译目标 | `x86_64-pc-windows-msvc` | `rustup target add x86_64-pc-windows-msvc`；禁止使用 GNU target |
| SQLite 动态库 | `sqlite3.dll`（64 位） | 仅离线工具（build / verify / benchmark）需要；服务运行时不需要 |
| Docker（可选） | Docker Desktop 或兼容引擎 | 仅容器化部署时需要 |

> **注意**：项目禁止使用 GNU target（会误用 32 位 dlltool）。所有 `cargo` 命令
> 均需指定 `--target x86_64-pc-windows-msvc`。

### SQLite 配置

SQLite 通过 `libloading` 动态加载，**不需要** 系统安装或静态链接。
运行离线工具时，程序会依次查找 `sqlite3.dll` / `libsqlite3.so.0` /
`libsqlite3.so` / `libsqlite3.dylib`。

如果自动查找失败，通过环境变量显式指定：

```powershell
$env:PHS_SQLITE3_LIB = "C:\path\to\sqlite3.dll"
```

> Windows 下建议固定指向已知的 64 位 `sqlite3.dll`，避免系统 `PATH`
> 中被加载到不兼容的 32 位版本。

## 首次配置

```powershell
# 1. 克隆项目
git clone <repo-url>
cd poker-hands-storage

# 2. 启用 Git Hooks（pre-commit 自动运行 fmt + clippy + test）
git config core.hooksPath .githooks

# 3. 验证编译
cargo build --workspace --target x86_64-pc-windows-msvc
```

## 项目结构

```text
poker-hands-storage/
├── Cargo.toml                  # Workspace 根配置（三个 member crate）
├── Cargo.lock
├── AGENTS.md                   # AI 编码助手项目指令
├── rustfmt.toml                # 代码格式化配置
│
├── range-store-core/           # 共享存储核心库（无 HTTP 依赖）
│   ├── src/
│   │   ├── lib.rs
│   │   ├── bin_reader.rs       # .bin 文件 mmap reader
│   │   ├── idx_reader.rs       # .idx 索引文件 reader
│   │   ├── pack_codec.rs       # Pack 二进制编解码
│   │   ├── crc32c.rs           # CRC32C 校验
│   │   ├── dimension.rs        # 维度命名与发现
│   │   ├── dimension_reader.rs # 单维度 reader（组合 idx + bin）
│   │   ├── hole_cards.rs       # 手牌解析与归一化
│   │   ├── action_schema.rs    # Action 类型定义与解码
│   │   ├── types.rs            # 共享类型定义
│   │   ├── manifest/           # manifest.json 解析
│   │   ├── query/              # StoreQueryService 与 LRU handle pool
│   │   └── sqlite/             # SQLite 动态加载（libloading）
│   └── tests/                  # 集成测试
│       ├── domain/             # action_schema / dimension / hole_cards 测试
│       ├── storage/            # manifest reader / sqlite connection 测试
│       └── traversal_and_decode.rs  # idx + bin 联合遍历测试
│
├── service/                    # HTTP API 服务（axum 0.8）
│   ├── src/
│   │   ├── main.rs             # 入口：serve / healthcheck 子命令
│   │   ├── lib.rs
│   │   ├── config/             # 环境变量配置加载
│   │   ├── errors/             # 统一 AppError 错误类型
│   │   ├── http/               # Axum server 启动、OpenAPI、校验
│   │   ├── query/              # 查询服务层、维度 handle pool
│   │   ├── routes/             # HTTP 路由 handler
│   │   └── storage/            # Manifest reader、metadata DB
│   └── tests/                  # 集成测试
│       ├── config/             # 配置解析测试
│       ├── http/               # 路由测试
│       └── storage/            # 存储层测试
│
├── storage-tools/              # 离线工具集（不依赖 HTTP）
│   ├── src/
│   │   ├── main.rs             # CLI 入口（build / verify / benchmark 等）
│   │   ├── lib.rs
│   │   ├── errors.rs           # 工具错误类型
│   │   ├── metadata.rs         # meta.db 元数据写入
│   │   ├── range_store_builder/  # SQLite → PFSP/PFXI 二进制构建流程
│   │   ├── verification/       # 数据验证
│   │   │   ├── standalone/     # 独立验证（不需要源 SQLite）
│   │   │   ├── cross/          # 源数据交叉验证
│   │   │   ├── report/         # 验证报告生成（JSON / Markdown）
│   │   │   ├── float32_precision.rs  # float32 精度比较语义
│   │   │   └── catalog_checks.rs     # manifest / meta.db 目录检查
│   │   └── benchmark/          # 性能基准测试
│   │       ├── hot/            # 热路径（mmap 缓存命中）基准
│   │       ├── cold/           # 冷启动基准
│   │       ├── sqlite/         # SQLite 基线基准
│   │       ├── compare/        # 二进制 vs SQLite 对比报告
│   │       ├── workload.rs     # 工作负载生成
│   │       ├── metrics.rs      # QPS / 延迟 / 百分位统计
│   │       └── report.rs       # 基准报告生成
│   └── tests/                  # 集成测试
│       ├── verification/       # 验证逻辑测试
│       └── benchmark/          # 基准配置解析测试
│
├── .docker/                    # 容器化部署
│   ├── Dockerfile              # 多阶段构建（builder + runtime）
│   ├── Cargo.service.toml      # Docker 构建专用最小 Cargo.toml
│   ├── docker-compose.yml      # Compose 编排
│   └── k8s.yaml                # Kubernetes 部署清单
│
├── .githooks/                  # Git Hooks
│   ├── pre-commit              # 提交前自动运行 fmt + clippy + test
│   ├── pre-push                # 推送前检查
│   ├── post-commit
│   ├── post-checkout
│   └── post-merge
│
├── data/                       # 数据目录（gitignore 排除大文件）
│   ├── range-strata/           # 完整构建输出（manifest + idx + bin）
│   ├── smoke/                  # 测试用小数据集
│   └── sqlite/                 # 源 SQLite 数据库
│
├── docs/                       # 项目文档
├── reports/                    # 验证和基准报告输出
└── target/                     # Cargo 编译输出
```

集成测试位于各 crate 的 `tests/` 目录下，使用显式 Cargo target，
文件名格式为 `<source-file>.test.rs`。

## 快速上手

### 构建二进制数据

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- build `
  --source-db C:\path\to\range.db `
  --out-dir data\range-strata `
  --dimension default:6:100 `
  --overwrite
```

### 启动 HTTP 服务

```powershell
$env:PHS_DATA_DIR = "data\range-strata"
$env:PHS_META_DB = "data\range-strata\meta.db"
$env:PHS_PREWARM = "default:6:100"
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- serve
```

### Docker 部署

```powershell
docker compose -f .docker/docker-compose.yml up --build
```

## 环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `PHS_BIND` | `0.0.0.0:8080` | 监听地址 |
| `PHS_DATA_DIR` | `/data` | 数据目录 |
| `PHS_META_DB` | `${PHS_DATA_DIR}/meta.db` | 元数据数据库路径 |
| `PHS_MAX_OPEN_HANDLES` | `3` | 最大打开句柄数 |
| `PHS_VERIFY_CHECKSUMS` | `false` | 启用 CRC32C 校验 |
| `PHS_PREWARM` | 空 | 启动预热维度 |
| `PHS_SQLITE3_LIB` | 自动检测 | SQLite 动态库路径 |
| `RUST_LOG` | `info` | 日志级别 |

## 详细文档

| 文档 | 说明 |
| --- | --- |
| [API 业务逻辑和接口契约](docs/api-business-contract.md) | HTTP 路由、请求/响应格式、验证规则、错误码 |
| [二进制存储方案设计](docs/range-db-binary-storage-design.md) | `.idx/.bin` 文件格式、pack 编码、CRC32C 校验 |
| [存储架构调研报告](docs/storage-architecture-research.md) | 技术选型、mmap、SQLite 动态加载、性能基准 |
| [数据一致性验证](docs/data-verification-and-format-validation.md) | standalone / cross 验证流程、报告格式 |
| [Docker 部署指南](docs/docker-deployment-guide.md) | Dockerfile、compose、容器运行、健康检查 |

## Agent Skills

本项目内置了 [Agent Skills](https://agentskills.io) 支持，AI 编码助手（Claude Code、
Gemini 等）可自动加载项目指令，获得编译规则、架构边界、操作流程等上下文。

```text
.agents/
├── SKILL.md                  # 全局项目指令（编译规则、架构边界、操作流程）
└── references/               # 按需加载的详细参考
    ├── build.md              # 构建二进制数据
    ├── verify.md             # 数据验证（standalone / cross）
    ├── benchmark.md          # 性能基准（hot / cold / compare）
    └── service.md            # HTTP 服务与 Docker 部署
```

支持 Agent Skills 的客户端会自动发现并使用这些指令，无需额外配置。

## 校验

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```
