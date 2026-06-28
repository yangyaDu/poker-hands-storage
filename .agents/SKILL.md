---
name: poker-hands-storage-ops
description: >
  操作 poker-hands-storage 项目的离线工具和服务部署。
  包括：从 SQLite 构建二进制数据、standalone/cross 数据验证、
  hot/cold 性能基准测试、HTTP 服务启动、Docker 容器部署。
  当用户请求构建数据、验证数据、运行基准测试、启动服务或部署容器时触发。
---

# poker-hands-storage 操作指南

操作前先阅读 `CLAUDE.md` 了解平台规则和架构边界。
详细命令参数见 `references/commands.md`。

## 操作流程

### 构建二进制数据

1. 确认源 SQLite 路径和输出目录
2. 运行 `storage-tools build` 命令（参数见 references/commands.md）
3. 构建完成后，自动运行 standalone 验证
4. 如果有源 SQLite，追加 cross 验证

### 验证数据

- **standalone**：检查 manifest、meta.db、.idx、.bin 结构和 CRC32C 校验，不需要源 SQLite
- **cross**：在 standalone 基础上，比较源 SQLite 行与二进制 pack 的 float32 bit-exact 一致性
- 发布级验证使用 `--sample-size 0` 做全量扫描

### 性能基准测试

1. 先运行 binary benchmark（hot 路径）
2. 再运行 SQLite baseline（使用相同 workload）
3. 最后运行 benchmark-compare 对比两份报告
4. 冷启动测试同理（benchmark-cold → benchmark-sqlite-cold → benchmark-cold-compare）

**注意**：不同 workload mode、dimension、sample set 的报告不可直接对比。

### 启动 HTTP 服务

1. 设置 `PHS_DATA_DIR`、`PHS_META_DB`、`PHS_PREWARM` 环境变量
2. 运行 `service serve`
3. 检查 `/health` 和 `/ready` 端点

### Docker 部署

1. 运行 `docker compose up --build`
2. 检查容器状态和 readiness 端点
3. Docker 运行时使用只读数据挂载，源 SQLite 仅用于离线构建

## API 行为要点

- 成功响应：`{ code: 0, data, message: null }`
- 错误响应：`{ code, data: null, message }`
- 验证错误：HTTP 400 / code 1000
- Not Found：HTTP 404 / code 404
- 内部错误：HTTP 500 / code 500
- `/range/hands-by-actions` 省略 frequency 表示 `> 0`，提供 `frequency = x` 表示 `>= x`
- 多个 `action_name` 过滤条件为交集语义

详细 API 契约见 `docs/api-business-contract.md`。
