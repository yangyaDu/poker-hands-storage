# Proto V3 Docker 部署流程

更新日期：2026-07-16

HTTP service 默认且只读取 Proto V3。生产容器只需要镜像和一个只读 V3 根目录；不需要源
`range.db`、`meta.db`、`lines.db`、PFSP Binary 或 Proto V2 文件。

## 数据目录

根目录下每个子目录代表一个维度：

```text
proto-v3/
  default_6max_100BB/
    manifest.json
    drill-scenarios.pb
    drill-scenarios.idx
    abstract-action-paths.pb
    abstract-action-paths.idx
    hand-strategies.pb
    hand-strategies.idx
```

离线生成并验收全部维度：

```powershell
cargo run -p poker-hands-storage-tools -- v3-export-all `
  --source data\sqlite\range.db `
  --out-root data\proto-v3-releases\2026-07-16T000000Z

cargo run -p poker-hands-storage-tools -- v3-benchmark `
  --source data\sqlite\range.db `
  --archive-root data\proto-v3-releases\2026-07-16T000000Z `
  --dimension default:6:100
```

`v3-export-all` 对每个维度执行 read-back standalone verify 和 SQLite cross verify。正式发布仍应
保存报告，并确认所有计划维度均被发现。

## Compose

```powershell
$env:PHS_HOST_DATA_DIR = "C:\path\to\proto-v3-release"
docker compose -f .docker\docker-compose.yml up --build -d
docker compose -f .docker\docker-compose.yml ps
Invoke-RestMethod http://127.0.0.1:8080/ready
```

Compose 把宿主机 V3 root 只读挂载到 `/data`。不要把单个维度目录挂到 `/data`；service 需要在
根目录发现维度子目录。

## 环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `PHS_BIND` | `0.0.0.0:8080` | HTTP 监听地址 |
| `PHS_DATA_DIR` | `/data` | Proto V3 根目录 |
| `PHS_MAX_OPEN_HANDLES` | `2` | 维度 handle LRU 容量 |
| `PHS_METADATA_CACHE_BYTES` | `8388608` | 每 handle metadata cache 字节预算 |
| `PHS_STRATEGY_CACHE_BYTES` | `67108864` | 每 handle strategy cache 字节预算 |
| `PHS_VERIFY_CHECKSUMS` | `false` | 打开维度时验证完整文件 CRC32C |
| `PHS_PREWARM` | 空 | `strategy:player_count:depth_bb` 列表 |
| `RUST_LOG` | `info` | 日志级别 |

发布环境建议启用 `PHS_VERIFY_CHECKSUMS=true`。查询时仍会检查目标 page/payload CRC。

## 健康检查与 smoke

```text
GET /health
GET /ready
GET /swagger
```

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
  -Method Post -ContentType "application/json" -Body $body
```

## 发布和问题处理

1. 每次从源 SQLite 导出到新的版本化目录，不覆盖正在 mmap 的目录。
2. 确认全部维度的 standalone/cross verify 零差异。
3. 运行 SQLite/V3 benchmark，确认 correctness gate、P50/P95、cache bytes 和 RSS。
4. 将挂载切到新目录，滚动重启，检查 `/ready` 和 smoke 查询。
5. 若发现问题，停止发布，修复 writer/reader 后从 SQLite 重新导出新的 V3 目录。

V3 没有 V2 双读或 V2 回退路径。所谓“回滚”只能切回一个已经通过校验的旧 V3 release。
