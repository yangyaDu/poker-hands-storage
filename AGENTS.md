# AGENTS.md

## 构建与编译

- 默认且唯一 target：`x86_64-pc-windows-msvc`
- **禁止** GNU target（会误用 32 位 dlltool，导致链接失败）
- 所有 `cargo` 命令必须带 `--target x86_64-pc-windows-msvc`

## SQLite

- SQLite 通过 `libloading` 运行时动态加载，不需要静态链接
- Windows 下通过 `PHS_SQLITE3_LIB` 环境变量指定 64 位 `sqlite3.dll` 路径
- 仅离线工具（build / verify / benchmark）需要 SQLite；HTTP 服务运行不需要

## 代码质量检查

提交前运行（pre-commit hook 会自动执行）：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

## Git Hooks

```bash
git config core.hooksPath .githooks
```

## Workspace 架构边界

三个 crate 职责严格分离：

- `range-store-core`：二进制格式、reader、codec、查询原语（不依赖 HTTP）
- `service`：HTTP 路由、请求校验、响应封装、服务启动（不依赖离线工具）
- `storage-tools`：离线构建、验证、基准测试（不依赖 HTTP）

**规则**：`service` 和 `storage-tools` 不得互相依赖，均只依赖 `range-store-core`。

## 文档同步规则

行为变更时必须同步更新对应文档：

| 变更类型 | 文档 |
|---|---|
| API 路由 / 请求 / 响应 | `docs/api-business-contract.md` |
| 二进制格式 / 存储布局 | `docs/range-db-binary-storage-design.md` |
| 验证逻辑 | `docs/data-verification-and-format-validation.md` |
| Docker / 运行时 | `docs/docker-deployment-guide.md` |
| 架构决策 | `docs/storage-architecture-research.md` |

## 测试约定

- 集成测试文件名格式：`<module>.test.rs`
- 测试位于各 crate 的 `tests/` 目录，使用显式 `[[test]]` Cargo target
