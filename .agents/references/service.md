# HTTP 服务与 Docker 部署

## 本地启动

```powershell
$env:PHS_DATA_DIR = "data\range-strata"
$env:PHS_META_DB = "data\range-strata\meta.db"
$env:PHS_PREWARM = "default:6:100"
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- serve
```

## 环境变量

| 变量 | 默认值 | 说明 |
|---|---|---|
| `PHS_BIND` | `0.0.0.0:8080` | HTTP 监听地址 |
| `PHS_DATA_DIR` | `/data` | Range Strata 运行目录 |
| `PHS_META_DB` | `${PHS_DATA_DIR}/meta.db` | metadata SQLite 路径 |
| `PHS_MAX_OPEN_HANDLES` | `2` | 最大打开维度 reader 数 |
| `PHS_VERIFY_CHECKSUMS` | `false` | 查询时是否校验 pack CRC32C |
| `PHS_PREWARM` | 空 | 启动预热维度，格式 `strategy:player_count:depth_bb` |
| `PHS_SQLITE3_LIB` | 自动检测 | 离线工具使用的 SQLite 动态库路径 |
| `RUST_LOG` | `info` | 日志级别 |

## 端点

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/swagger` | Scalar API Reference |
| GET | `/api-docs/openapi.json` | OpenAPI 文档 |
| GET | `/health` | 健康检查 |
| GET | `/ready` | 就绪检查 |
| POST | `/range/hand-strategy` | 单手策略查询 |
| POST | `/range/hand-strategy-batch` | 批量策略查询 |
| POST | `/range/hands-by-actions` | 按 action 过滤手牌 |
| POST | `/range/prewarm` | 预热维度 |
| POST | `/range/concrete-lines` | 具体线路查询 |
| POST | `/range/drill-scenarios` | Drill 场景查询 |

## Docker 部署

```powershell
docker compose -f .docker\docker-compose.yml up --build -d
docker compose -f .docker\docker-compose.yml ps
Invoke-RestMethod -Uri http://127.0.0.1:8080/ready
```

### 查询 smoke test

```powershell
$body = @{
  strategy = "default"
  player_count = 6
  depth_bb = 100
  concrete_line_id = 1
  hole_cards = "AA"
} | ConvertTo-Json

Invoke-RestMethod `
  -Uri http://127.0.0.1:8080/range/hand-strategy `
  -Method Post `
  -ContentType "application/json" `
  -Body $body
```

### 注意事项

- Docker 运行时数据目录只读挂载
- 源 SQLite 仅用于离线构建，不进入容器
- 运行时镜像包含 `libsqlite3.so.0`
