# Docker 部署流程

更新日期：2026-06-28

## 部署目标

Docker 部署只运行 HTTP API 服务，不包含离线构建、验证和 benchmark 工具。

运行时需要两个部分：

1. 镜像：`poker-hands-storage:local` 或发布到镜像仓库的正式 tag。
2. 数据目录：包含 `manifest.json`、`meta.db`、`*.idx`、`*.bin` 的 Range Strata 输出目录。

源 SQLite `range.db` 不是线上容器输入，只用于离线构建和验证。

## 镜像构建方案

Dockerfile 使用多阶段构建：

1. `builder` 阶段使用 `rust:1-slim-bookworm`。
2. 只 COPY `.docker/Cargo.service.toml`、`Cargo.lock`、`range-store-core`、`service`。
3. 执行 `cargo build --release --locked -p poker-hands-storage-service`。
4. `deps-extractor` 阶段安装运行时所需的 `libsqlite3-0` 和 CA 证书。
5. runtime 阶段使用 `gcr.io/distroless/base-debian12`。
6. 拷贝 service 二进制和动态库。
7. 使用非 root 用户启动。

`.docker/Cargo.service.toml` 只包含：

```text
range-store-core
service
```

因此 `storage-tools` 的 benchmark、verification、build 工具变更不会影响 service 镜像构建缓存。

## 本地 Compose 启动

默认命令：

```powershell
docker compose -f .docker\docker-compose.yml up --build -d
```

默认挂载：

```text
${PHS_HOST_DATA_DIR:-../data/range-strata}:/data:ro
```

如果要验证其他数据目录：

```powershell
$env:PHS_HOST_DATA_DIR = "C:\path\to\range-strata-v2"
docker compose -f .docker\docker-compose.yml up --build -d
```

查看容器：

```powershell
docker compose -f .docker\docker-compose.yml ps
```

停止：

```powershell
docker compose -f .docker\docker-compose.yml down
```

## 环境变量

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `PHS_BIND` | `0.0.0.0:8080` | 服务监听地址 |
| `PHS_DATA_DIR` | `/data` | 容器内数据目录 |
| `PHS_META_DB` | `/data/meta.db` | 元数据库路径 |
| `PHS_MAX_OPEN_HANDLES` | `3` | 维度 reader LRU 池大小 |
| `PHS_VERIFY_CHECKSUMS` | `false` | 查询时是否校验 pack CRC32C |
| `PHS_PREWARM` | 空 | 启动时预热的维度列表，格式 `strategy:player_count:depth_bb` |
| `RUST_LOG` | `info` | 日志级别 |
| `PHS_HOST_DATA_DIR` | `../data/range-strata` | Compose 使用的宿主机数据目录 |

Compose 当前显式配置：

```yaml
PHS_MAX_OPEN_HANDLES: "3"
PHS_VERIFY_CHECKSUMS: "true"
PHS_PREWARM: default:6:100
```

## 健康检查

接口：

```text
GET /health
GET /ready
```

`/health` 表示进程存活。  
`/ready` 表示服务已经成功打开数据目录并加载到可查询维度。

本地检查：

```powershell
Invoke-RestMethod http://127.0.0.1:8080/health
Invoke-RestMethod http://127.0.0.1:8080/ready
```

Dockerfile 内置 HEALTHCHECK 默认检查 `/health`。Compose 覆盖为检查 `/ready`，更接近线上流量接入条件。

## 查询 smoke

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

Swagger：

```text
http://127.0.0.1:8080/swagger
```

OpenAPI JSON：

```text
http://127.0.0.1:8080/api-docs/openapi.json
```

## 发布前数据准备

构建数据：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- build `
  --source-db data\sqlite\range.db `
  --out-dir data\range-strata `
  --overwrite
```

验证数据：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode standalone `
  --dir data\range-strata `
  --verify-checksum

cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode cross `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --sample-size 10000 `
  --verify-checksum
```

严格发布建议使用 `--sample-size 0` 做全量 cross verify。

## 推荐部署流程

1. 从源 SQLite 构建新的 Range Strata 数据目录。
2. 对新目录执行 standalone verify。
3. 如果源 SQLite 可用，对新目录执行 cross verify。
4. 构建 Docker 镜像。
5. 用新数据目录启动容器。
6. 检查 `/health`、`/ready`。
7. 执行至少一个 `hand-strategy` smoke 查询。
8. 切正式流量。

命令示例：

```powershell
docker compose -f .docker\docker-compose.yml up --build -d
docker compose -f .docker\docker-compose.yml ps
Invoke-RestMethod http://127.0.0.1:8080/ready
```

## Kubernetes 部署

`.docker/k8s.yaml` 提供基础模板：

- `ConfigMap`：服务环境变量。
- `PersistentVolumeClaim`：只读数据卷。
- `Deployment`：容器、资源限制、探针和只读挂载。
- `Service`：ClusterIP。

探针：

```text
readinessProbe -> GET /ready
livenessProbe  -> GET /health
```

默认资源：

```yaml
requests:
  cpu: 100m
  memory: 256Mi
limits:
  cpu: "1"
  memory: 1Gi
```

生产环境应按实际数据目录大小、prewarm 策略和访问量调整资源。

## 安全和文件系统策略

Compose 当前配置：

- 数据目录 `/data` 只读挂载。
- root filesystem `read_only: true`。
- `cap_drop: [ALL]`。
- `no-new-privileges:true`。
- distroless runtime。
- 非 root 用户运行。

这些配置要求服务运行时不能写本地文件。日志应走 stdout/stderr。

## Prewarm 与内存

服务启动时会读取 manifest、打开 `meta.db`、加载 action schemas，并校验维度文件和 action schema 引用。

`PHS_PREWARM` 只会把配置的维度打开进 handle pool。mmap 打开 `.idx/.bin` 不等于立即把整个 `.bin` 读入物理内存，但被访问过的页会进入 OS page cache，并可能体现在 RSS 或容器内存统计中。

建议：

- 生产只 prewarm 高频维度。
- 不建议为了 ready 全量 prewarm 所有维度。
- 如果希望把某些首次访问成本放到 ready 前，可把这些维度加入 `PHS_PREWARM`。
- 如果容器内存紧张，应降低 `PHS_MAX_OPEN_HANDLES` 或减少 prewarm 维度。

## 回滚流程

推荐使用版本化数据目录：

```text
range-strata-v1/
range-strata-v2/
current -> range-strata-v2
```

回滚时切回旧目录并重启容器：

```powershell
$env:PHS_HOST_DATA_DIR = "C:\path\to\range-strata-v1"
docker compose -f .docker\docker-compose.yml up -d
```

不要在已有容器正在 mmap 的目录中原地覆盖 `.idx/.bin` 文件。应发布新目录并重启或滚动替换容器。

## 常见问题

### `/ready` 返回 503

可能原因：

- `/data/manifest.json` 不存在或格式错误。
- manifest 中没有成功维度。
- 数据目录挂载错误。
- `meta.db` 或 `.idx/.bin` 缺失导致服务启动失败。

先检查容器日志和挂载路径。

### 容器启动失败，提示 SQLite 动态库问题

runtime 镜像已经包含 `libsqlite3.so.0`。如果自定义基础镜像，需要确保动态 SQLite 库存在。

Windows 本机运行时可通过 `PHS_SQLITE3_LIB` 指定 `sqlite3.dll`，Docker Linux runtime 通常不需要设置。

### 查询返回 404

可能原因：

- 维度不存在。
- `concrete_line_id` 不存在。
- 手牌不在该 concrete line 的 pack 中。
- `/range/hands-by-actions` 筛选后没有满足条件的手牌。

### 内存高于预期

可能原因：

- `PHS_PREWARM` 配置了多个大维度。
- `PHS_MAX_OPEN_HANDLES` 较大。
- 查询访问触发了大量 `.bin` 文件页进入 page cache。
- `PHS_VERIFY_CHECKSUMS=true` 增加了 pack 读取和计算。

先用较小 prewarm 集合验证，再逐步增加。

