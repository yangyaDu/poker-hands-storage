---
name: poker-hands-storage
description: >
  poker-hands-storage Rust workspace 的全局项目指令。
  三个 crate：range-store-core（存储核心）、service（HTTP API）、storage-tools（离线工具）。
  涵盖：编译规则、架构边界、代码质量、文档同步、数据构建/验证/基准测试、服务部署。
---

# poker-hands-storage 项目指令

## 编译规则

- 唯一 target：`x86_64-pc-windows-msvc`，禁止 GNU target
- 所有 `cargo` 命令必须带 `--target x86_64-pc-windows-msvc`
- SQLite 通过 `libloading` 动态加载，不需要静态链接
- Windows 下通过 `PHS_SQLITE3_LIB` 指定 64 位 `sqlite3.dll`
- 仅离线工具需要 SQLite；HTTP 服务运行不需要

## 架构边界

三个 crate 职责严格分离：

- `range-store-core`：二进制格式、reader、codec、查询原语（不依赖 HTTP）
- `service`：HTTP 路由、请求校验、响应封装、服务启动（不依赖离线工具）
- `storage-tools`：离线构建、验证、基准测试（不依赖 HTTP）

**规则**：`service` 和 `storage-tools` 不得互相依赖，均只依赖 `range-store-core`。

## 代码质量

提交前运行（pre-commit hook 会自动执行）：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

## 测试约定

- 集成测试文件名格式：`<module>.test.rs`
- 测试位于各 crate 的 `tests/` 目录，使用显式 `[[test]]` Cargo target

## 文档同步

行为变更时必须同步更新对应文档：

| 变更类型 | 文档 |
|---|---|
| API 路由 / 请求 / 响应 | `docs/api-business-contract.md` |
| 二进制格式 / 存储布局 | `docs/range-db-binary-storage-design.md` |
| 验证逻辑 | `docs/data-verification-and-format-validation.md` |
| Docker / 运行时 | `docs/docker-deployment-guide.md` |
| 架构决策 | `docs/storage-architecture-research.md` |

## Git Hooks

```bash
git config core.hooksPath .githooks
```

## 操作流程

详细命令参数见 `references/commands.md`。

### 构建二进制数据

1. 确认源 SQLite 路径和输出目录
2. 运行 `storage-tools build`
3. 构建完成后运行 standalone 验证
4. 如有源 SQLite，追加 cross 验证

### 验证数据

- **standalone**：检查 manifest、meta.db、.idx、.bin 结构和 CRC32C，不需要源 SQLite
- **cross**：比较源 SQLite 与二进制 pack 的 float32 bit-exact 一致性
- 发布级验证使用 `--sample-size 0` 全量扫描

### 基准测试

1. binary benchmark → SQLite baseline → benchmark-compare
2. 冷启动同理：benchmark-cold → benchmark-sqlite-cold → benchmark-cold-compare
3. 不同 workload / dimension / sample set 的报告不可直接对比

### HTTP 服务

1. 设置 `PHS_DATA_DIR`、`PHS_META_DB`、`PHS_PREWARM`
2. 运行 `service serve`
3. 检查 `/health` 和 `/ready`

### Docker 部署

1. `docker compose -f .docker/docker-compose.yml up --build`
2. 检查容器状态和 readiness
3. 数据目录只读挂载，源 SQLite 仅用于离线构建

## API 行为要点

- 成功：`{ code: 0, data, message: null }`
- 错误：`{ code, data: null, message }`
- 验证错误：HTTP 400 / code 1000
- Not Found：HTTP 404 / code 404
- 内部错误：HTTP 500 / code 500
- `/range/hands-by-actions`：省略 frequency = `> 0`，提供 `frequency = x` = `>= x`
- 多个 `action_name` 为交集语义

详细 API 契约见 `docs/api-business-contract.md`。
