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

## Workspace 结构

项目为 Cargo workspace，包含三个 crate：

```text
range-store-core/     共享存储核心：.idx/.bin reader、pack 编解码、
                      CRC32C、维度命名、手牌解析、action schema、
                      StoreQueryService。

service/              HTTP API 服务（axum）：serve、healthcheck。

storage-tools/        离线工具集（不依赖 HTTP）：
                      build、verify、benchmark。
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

## 校验

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```
