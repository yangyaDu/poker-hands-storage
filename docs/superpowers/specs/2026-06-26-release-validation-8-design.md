# Rust Release Validation 8 Design

## 背景

`poker-hands-storage` 是 `preflop-storage` 的独立 Rust 服务重构项目。到
Phase 7 为止，核心运行时能力已经基本迁移完成：

- `build` 可以从旧 SQLite `range.db` 生成 `manifest.json + meta.db + .idx + .bin`。
- `verify --mode standalone|cross` 已覆盖 Rust 版数据包自检和源库交叉校验。
- HTTP 服务、OpenAPI、容器化和 smoke 验收已经具备。
- hot benchmark、cold benchmark、SQLite baseline、compare report 已经接入 Rust CLI。

同级 `preflop-storage` 的主线已经进入发布前检查和报告刷新阶段。当前 Rust 项目的下一步不是继续新增查询功能，而是建立一条可重复的发布验收闭环，证明全量数据、Rust 服务、容器和 benchmark 报告在同一份产物上同时成立。

## 目标

1. 定义 Rust 服务发布前必须通过的验证链路。
2. 使用全量 `data/range-strata`，不再只依赖 `data/smoke`。
3. 刷新 Rust standalone/cross verifier 报告。
4. 使用同一 workload 刷新 binary hot、SQLite baseline、compare 报告。
5. 刷新 9 维度 cold-start 报告，明确 process/store/query 三层口径。
6. 用全量数据目录执行容器验收。
7. 更新 `docs/progress.md`，让文档状态反映 7d/7e 和 Phase 8 的真实结果。

## 非目标

1. 不新增 `.pfs` 合并格式。
2. 不修改 PFSP/PFXI 文件格式。
3. 不引入硬性能阈值作为发布阻塞条件。
4. 不把 Docker/K8s 操作塞进 `poker-hands-storage-service` 二进制。
5. 不要求本阶段实现 CI 或多平台镜像发布。
6. 不处理当前工作区里与 release validation 无关的 Docker 文件移动或重命名决策。

## 发布验收输入

默认输入：

```text
data/sqlite/range.db
data/range-strata/
  manifest.json
  meta.db
  ranges_default_6max_100BB.idx
  ranges_default_6max_100BB.bin
  ...
```

要求：

- `data/sqlite/range.db` 是构建源库。
- `data/range-strata` 是由当前 Rust `build` 或已确认兼容的上游构建器生成的运行时目录。
- `manifest.json` 中成功维度应覆盖 9 个默认维度：`6max/8max/9max x 100BB/200BB/300BB`。
- 所有 release report 必须基于同一份 `data/range-strata`。

## 发布验收输出

Phase 8 默认刷新下面的报告：

```text
reports/range-strata-verify-standalone.json
reports/range-strata-verify-standalone.md
reports/range-strata-verify-cross.json
reports/range-strata-verify-cross.md

reports/benchmark-range-strata-binary.json
reports/benchmark-range-strata-binary.md
reports/benchmark-sqlite.json
reports/benchmark-sqlite.md
reports/benchmark-compare.json
reports/benchmark-compare.md
reports/benchmark-cold-start.json
reports/benchmark-cold-start.md
```

可选 smoke 报告可以保留 `*-smoke` 后缀，但不能替代 release 报告。

## 验收命令

### 1. 基础质量门禁

必须使用 MSVC target，禁止 GNU target。

```powershell
$env:PHS_SQLITE3_LIB = "C:\path\to\known-64-bit\sqlite3.dll"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc
```

`PHS_SQLITE3_LIB` 是 Windows 发布验证的前置条件：Phase 8 smoke 中，默认
DLL 解析曾让 `benchmark-sqlite` 返回 SQLite `disk I/O error`；固定到已知
64-bit `sqlite3.dll` 后同一 workload 通过。

### 2. 全量 standalone verifier

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify `
  --mode standalone `
  --dir data/range-strata `
  --verify-checksum `
  --out reports/range-strata-verify-standalone.json `
  --md reports/range-strata-verify-standalone.md
```

通过标准：

- `total failures = 0`
- manifest OK
- catalog OK
- index files OK = 成功维度数
- pack files OK = 成功维度数
- index-pack cross failures = 0

### 3. 全量或采样 cross verifier

默认 release gate 使用采样交叉校验：

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify `
  --mode cross `
  --dir data/range-strata `
  --source data/sqlite/range.db `
  --sample-size 10000 `
  --verify-checksum `
  --out reports/range-strata-verify-cross.json `
  --md reports/range-strata-verify-cross.md
```

发布候选版本可追加一次 full scan：

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- verify `
  --mode cross `
  --dir data/range-strata `
  --source data/sqlite/range.db `
  --sample-size 0 `
  --verify-checksum `
  --out reports/range-strata-verify-cross-full.json `
  --md reports/range-strata-verify-cross-full.md
```

通过标准：

- source records failed = 0
- extra binary records = 0
- Float32 bit-exact mismatch = 0
- `hand_ev` null mismatch = 0

### 4. 生成共享 benchmark workload

binary 和 SQLite 必须使用同一份 workload，否则 compare 默认应拒绝。

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark `
  --dir data/range-strata `
  --source data/sqlite/range.db `
  --write-workload reports/release-workload.json `
  --workload-mode abstract-local `
  --iterations 1000 `
  --batch-size 20 `
  --batch-sizes 1,5,10,20,50,100 `
  --verify-results `
  --verify-checksum `
  --out reports/benchmark-range-strata-binary.json `
  --md reports/benchmark-range-strata-binary.md
```

Phase 8 实施已补 `--write-workload`，release benchmark 链路应先写出 workload，再让 SQLite baseline 复用该文件。

建议新增 CLI 选项：

```text
benchmark --write-workload <path>
benchmark-sqlite --workload <path>
```

这样 release compare 可以严格保证 binary 和 SQLite 使用同一批 query。

### 5. SQLite baseline benchmark

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark-sqlite `
  --source data/sqlite/range.db `
  --workload reports/release-workload.json `
  --warmup-iterations 20 `
  --out reports/benchmark-sqlite.json `
  --md reports/benchmark-sqlite.md
```

通过标准：

- `totals.errorCount = 0`
- report engine = `sqlite`
- case names 与 binary benchmark 对齐

### 6. Benchmark compare

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark-compare `
  --binary reports/benchmark-range-strata-binary.json `
  --sqlite reports/benchmark-sqlite.json `
  --out reports/benchmark-compare.json `
  --md reports/benchmark-compare.md
```

通过标准：

- compare 不使用 `--allow-mismatch`
- `compatibleWorkload = true`
- case 数量与 binary/sqlite 报告一致
- result count match
- 有错误的 case 不能作为性能结论

性能结论只写“当前观测值”，不写固定门槛。后续如果要加阈值，应先积累多个 release 的本机/CI 基线。

### 7. Cold-start benchmark

```powershell
cargo run -p poker-hands-storage-service --target x86_64-pc-windows-msvc -- benchmark-cold `
  --dir data/range-strata `
  --source data/sqlite/range.db `
  --mode process-cold `
  --runs 10 `
  --query-policy fixed `
  --concrete-line-id 1 `
  --hand AA `
  --verify-checksum `
  --out reports/benchmark-cold-start.json `
  --md reports/benchmark-cold-start.md
```

通过标准：

- 9 个成功维度均覆盖
- `aggregate.error_count = 0`
- failure 列表为空
- 报告明确区分 `storeOpenAndFirstQueryMs`、`workerTotalMs`、`processElapsedMs`
- latency 聚合只使用成功 run

### 8. 全量容器验收

当前 compose 文件位于 `.docker/docker-compose.yml`，默认挂载 `../data/range-strata:/data:ro`。
这是 Linux 发布链路的权威验收：镜像在 Linux builder 阶段编译 Rust binary，
runtime 镜像提供 `libsqlite3.so.0` 并挂载只读 `/data`。WSL `/home/ubuntu2204`
可用于提前跑 Linux `cargo test/build` 或 benchmark 调试，但不作为发布必需步骤；
最终以 Docker build/run、health/readiness 和 API smoke 为准。

```powershell
docker compose -f .docker/docker-compose.yml up --build -d
```

验收请求：

```powershell
curl.exe -fsS http://127.0.0.1:8080/health
curl.exe -fsS http://127.0.0.1:8080/ready
```

查询 smoke：

```powershell
curl.exe -fsS -X POST http://127.0.0.1:8080/query `
  -H "content-type: application/json" `
  -d "{\"strategy\":\"default\",\"player_count\":6,\"depth_bb\":100,\"concrete_line_id\":1,\"hole_cards\":\"AA\"}"
```

batch smoke：

```powershell
curl.exe -fsS -X POST http://127.0.0.1:8080/batch `
  -H "content-type: application/json" `
  -d "{\"strategy\":\"default\",\"player_count\":6,\"depth_bb\":100,\"requests\":[{\"concrete_line_id\":1,\"hole_cards\":\"AA\"},{\"concrete_line_id\":1,\"hole_cards\":\"AKs\"}]}"
```

通过标准：

- container health = healthy
- `/ready` 返回 schema count、known dimensions、open handle count
- `/query` 返回 `exists=true` 且 action 数量与 verifier/source 口径一致
- `/batch` 保持输入顺序，单项错误不污染其他项
- 容器使用只读 `/data` volume

## 文档更新

Phase 8 实施完成后更新：

- `docs/progress.md`
- `README.md`
- 如 Docker 路径最终稳定在 `.docker/`，同步 README 中的 compose 命令

`docs/progress.md` 必须记录：

- 当前测试数量
- verifier standalone/cross 最新结果
- hot/sqlite/compare benchmark 报告路径
- cold benchmark 运行参数和错误数
- 容器是否使用全量 `data/range-strata`
- 已知 caveat，例如 OS page cache 对 cold benchmark 的影响

## 需要补的实现点

### 1. Workload 写出

`benchmark` 可以生成 workload，release compare 需要稳定复用同一 workload。Phase 8 已补最小实现：

```text
benchmark --write-workload <path>
```

行为：

- 如果 `--workload` 存在：读取 workload，并拒绝同时使用 `--write-workload`。
- 如果 `--write-workload` 存在且本次是生成 workload：写出完整 workload JSON。
- 写出的 workload 应包含 seed、mode、dimensions、hand queries、batch queries、batch queries by size。

测试：

- 生成 workload 后文件存在。
- SQLite benchmark 读取同一 workload 后 case iterations 与 binary 对齐。
- compare 不需要 `--allow-mismatch`。

### 2. Release checklist 文档或脚本

V1 可以只做文档化 checklist。若需要自动化，建议新增独立脚本而不是扩展服务二进制：

```text
scripts/release-validate.ps1
```

脚本职责：

- 顺序执行质量门禁、verifier、benchmark、container smoke。
- 遇到失败立即退出非 0。
- 输出报告路径摘要。

脚本不应隐藏命令细节；文档仍应保留完整命令。

## 风险与决策

| 风险 | 处理 |
|---|---|
| benchmark 结果受机器负载影响 | 不设硬阈值，只记录趋势和 ratio |
| binary/sqlite workload 不一致导致 compare 误导 | compare 默认拒绝 mismatch；release benchmark 使用 `--write-workload` 固定 workload |
| cold benchmark 受 OS page cache 影响 | 报告中明确 `process-cold` 只保证 fresh process |
| full cross verify 时间较长 | release gate 默认 sampled，发布候选可人工跑 full |
| 容器验收修改本地 Docker 状态 | Docker 相关改动应单独确认，不与 Rust release gate 混在同一变更里 |
| Windows SQLite 动态库缺失或解析到不兼容 DLL | 文档记录并在 release validation 前固定 `PHS_SQLITE3_LIB`；容器 runtime 安装 `libsqlite3-0` |

## Definition Of Done

1. `cargo fmt --all -- --check` 通过。
2. `cargo clippy --workspace --all-targets -- -D warnings` 通过。
3. `cargo test --workspace --target x86_64-pc-windows-msvc` 通过。
4. Rust standalone verifier 全量数据 0 failure。
5. Rust cross verifier 采样数据 0 failure。
6. binary hot benchmark 0 error，`--verify-results` 0 mismatch。
7. SQLite benchmark 0 error。
8. compare report 不使用 `--allow-mismatch` 且 workload compatible。
9. cold benchmark 覆盖 9 个维度且 0 error。
10. 容器使用全量 `data/range-strata` 启动，并通过 `/health`、`/ready`、`/query`、`/batch` smoke。
11. `docs/progress.md` 反映最新 Phase 8 验收结果。
