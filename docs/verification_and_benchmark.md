# 数据验证与 Benchmark 脚本介绍

更新日期：2026-07-08

## 目标

验证分为两类：

1. 二进制产物自洽：`manifest.json`、`meta.db`、`.idx`、`.bin` 的格式、引用和 checksum 是否正确。
2. 与源 SQLite 一致：从源 `range.db` 抽样或全量读取 rows，解码二进制 pack 后逐项比对。

当前工具位于 `storage-tools`，运行时核心格式能力位于 `range-store-core`。

## 验证覆盖面总览

当前验证分为七层：

| 层级 | 覆盖内容 | 主要入口 |
| --- | --- | --- |
| 格式自洽 | `manifest.json`、`meta.db`、`.idx`、`.bin`、header、record 边界、CRC32C | `storage-tools verify --mode standalone` |
| 源数据一致性 | 二进制解码结果与源 SQLite `range_data_*` rows 对齐 | `storage-tools verify --mode cross` |
| Float32 精度 | `frequency`、`hand_ev` 按 IEEE754 Float32 bit-exact 比对 | cross verify |
| Benchmark 结果一致性 | Binary vs SQLite 查询结果 count 兼容，metadata exact lookup id 自检 | `benchmark --verify-results`、`benchmark-compare` |
| Native 运行时一致性 | core / SDK / HTTP 三路公平对比 | `benchmark-native` |
| API 边界 | HTTP 路由、OpenAPI、请求校验、错误码、batch 单项错误 | `service/tests/http/*` |
| Native 边界 | Bun SDK envelope、lazy schema cache、native 与 HTTP 抽样一致性 | `range-store-native/tests/*` |

验证口径说明：

- `verify` 是数据正确性的主证据；正式发布以 full cross verify 为准。
- benchmark 的 `errors=0` 和 `result match=true` 是性能报告的结果一致性护栏，不替代 full cross verify。
- native/HTTP consistency 测试证明两种运行时边界复用同一 core 语义，不证明源数据全量正确性。
- 运行时 `PHS_VERIFY_CHECKSUMS=true` 或 SDK `verifyChecksums=true` 可以在查询时校验 pack CRC32C，但会增加每次查询成本。

## 验证命令

Standalone 验证：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode standalone `
  --dir data\range-strata `
  --verify-checksum
```

Cross 验证：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode cross `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --sample-size 10000 `
  --verify-checksum
```

全量 Cross 验证：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode cross `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --sample-size 0 `
  --verify-checksum `
  --out reports\range-strata-verify-cross-full.json `
  --md reports\range-strata-verify-cross-full.md
```

说明：

- `--mode standalone` 不依赖源 SQLite。
- `--mode cross` 会先运行 standalone，再和源 SQLite 比对。
- `--sample-size 10000` 表示按维度分摊抽样。
- `--sample-size 0` 表示全量 source scan。
- `--verify-checksum` 会校验 pack CRC32C。

## 当前验证结果快照

当前 reports 中的验证结果：

| 报告 | 时间 | 结果 |
| --- | --- | --- |
| `reports/range-strata-verify-standalone.md` | 2026-06-26T15:10:25Z | 通过 |
| `reports/range-strata-verify-cross.md` | 2026-06-26T15:11:32Z | 通过 |
| `reports/range-strata-verify-cross-full.md` | 2026-07-01T14:52:14Z | 通过 |

2026-07-08 的新增工作主要是 Binary/SQLite hot benchmark 补齐 `concrete-lines-exact`、benchmark 报告公共 helper 收敛，以及相关测试复测；full cross verify 结果仍以 2026-07-01 的全量报告为当前数据正确性快照。若重新生成发布目录，应对新目录重新执行 full cross verify。

Standalone 摘要：

| 指标 | 值 |
| --- | ---: |
| Dimensions | 9 |
| Manifest OK | YES |
| Catalog OK | YES |
| Index Files OK | 9 / 9 |
| Pack Files OK | 9 / 9 |
| Index-Pack Cross Failures | 0 |

Full Cross 摘要：

| 指标 | 值 |
| --- | ---: |
| Dimensions | 9 |
| Checked Source Records | 23,806,716 |
| Failed Source Records | 0 |
| Extra Binary Records | 0 |
| Failures | 0 |

Float32 精度摘要：

| 字段 | checked | null | bit exact | mismatch | max implementation abs error |
| --- | ---: | ---: | ---: | ---: | ---: |
| `frequency` | 23,806,716 | 0 | 23,806,716 | 0 | 0 |
| `hand_ev` | 18,956,044 | 4,850,672 | 18,956,044 | 0 | 0 |

这说明全量源数据比对中没有发现编码、解码、字节序或 Float32 转换引入的额外误差。

## Standalone 验证内容

Standalone 验证只检查输出目录自身，不读取源 SQLite。

### Manifest 检查

检查项：

- `manifest.json` 是否存在。
- JSON 是否可解析。
- `format == "PFSP"`。
- `version == 1`。
- 成功维度是否包含 `idxFile` 和 `binFile`。

失败示例：

- `MISSING_FILE`
- `INVALID_JSON`
- `UNSUPPORTED_FORMAT`

### 文件存在性检查

检查：

- `meta.db` 是否存在。
- 每个成功维度的 `.idx` 是否存在。
- 每个成功维度的 `.bin` 是否存在。

### Catalog 检查

读取 `meta.db` 并检查：

- `build_info` 表存在。
- `build_info.built_at` 存在。
- `build_info.source_checksum` 存在。
- `action_schemas` 表存在且非空。
- `action_blob` 长度等于 `action_count * 9`。
- `action_count` 在 `1..=32`。
- `action_blob` 的 CRC32C 等于表内 checksum。
- `schema_key` 等于 `action_blob` 的 hex。
- 每个维度需要的 drill 和 concrete lines 表存在。

### `.idx` header 和记录检查

检查：

- 文件长度至少 16 字节。
- magic 为 `PFXI`。
- version 为 `1`。
- header size 为 `16`。
- 文件长度覆盖 `record_count * 22`。
- `concrete_line_id` 严格升序。
- `concrete_line_id` 在同一维度内连续递增，满足 dense 下标 lookup 前提。
- `hand_count <= 169`。
- `action_schema_id` 必须存在于 `meta.db.action_schemas`。

### `.bin` header 检查

检查：

- 文件长度至少 16 字节。
- magic 为 `PFSP`。
- version 为 `1`。
- endian 为 little-endian。
- float type 为 Float32。
- layout 为 sparse hand-major v1。
- compression 为 none。

### `.idx` 与 `.bin` 交叉检查

对每条 `.idx` record 检查：

- `offset >= 16`，不能指向 `.bin` header 内。
- `offset + byte_length` 不越过 `.bin` 文件长度。
- `byte_length == hand_count * (5 + action_count * 8)`。
- 如果开启 `--verify-checksum`，计算 pack CRC32C 并比对 `.idx.checksum`。
- pack 中的 hand ids 必须在 `0..=168`。
- pack 中的 hand ids 必须严格升序。

## Cross 验证内容

Cross 验证以源 SQLite 为基准，检查二进制解码结果是否一致。

流程：

1. 打开源 SQLite。
2. 发现所有 `range_data_*` 维度表。
3. 按 `sample_size` 为每个维度分配抽样 quota。
4. 读取源 rows：`concrete_line_id`、`hole_cards`、`action_name`、`action_size`、`amount_bb`、`frequency`、`hand_ev`。
5. 打开对应 `.idx/.bin`。
6. 通过 `.idx` 找 pack，通过 `.bin` 读 pack。
7. 解码 pack，按 `hole_cards` 和 action 找到对应 cell。
8. 比对 action name、action size、amount、frequency、hand_ev。
9. 全量模式下额外检查 binary 中是否存在源 SQLite 没有的 cell。

失败类型包括：

- `PACK_NOT_FOUND_IN_IDX`
- `ACTION_SCHEMA_NOT_FOUND`
- `PACK_READ_ERROR`
- `CHECKSUM_MISMATCH`
- `PACK_DECODE_ERROR`
- `UNKNOWN_HAND`
- `HAND_NOT_FOUND_IN_PACK`
- action 字段不一致
- Float32 bit-exact 不一致

## Float32 精度策略

二进制格式使用 Float32 存储 `frequency` 和 `hand_ev`。验证标准不是固定容差，而是：

```text
decoded value 必须等于 source value 按 IEEE754 Float32 正确舍入后的值
```

也就是：

```text
expected = source as f32
actual = decoded as f32
expected_bits == actual_bits
```

含义：

- 允许 Float32 格式本身不可避免的量化误差。
- 不允许编码、解码、字节序、Rust 转换或验证逻辑引入额外误差。
- `hand_ev = null` 必须解码为 `null`。
- `hand_ev` 非 null 时执行同样的 bit-exact 检查。
- 非有限 source 值视为失败，不混入容差逻辑。

## Benchmark 结果验证

热路径 benchmark 还支持 `--verify-results`：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --verify-results
```

它不是完整一致性验证，但可以在性能测试同时确认生成 workload 的查询结果和 SQLite baseline 的 action count 一致。

`concrete-lines-exact` 的结果校验在 hot/SQLite runner 的 case 内完成：每个样本按 `concrete_line` 精确查询，必须只命中 1 行，并且命中的 id 必须等于样本里的 `concrete_line_id`。这类 metadata lookup 自检和 `--verify-results` 的 action-count 抽样校验一样，都是 benchmark 护栏，不替代 full cross verify。

SQLite 对比 benchmark 的当前报告显示：

- `reports/benchmark-compare.md`
- workload compatible 为 true。
- 所有 case `errors` 为 `0/0`。
- `result match` 为 true。

Native benchmark 的一致性口径：

- `benchmark-native` 复用同一 workload JSON，对比 `core:*`、`native-sdk:*`、`http-service:*` case。
- 最新 9max:100BB fair benchmark 中这些入口的错误数均为 0。
- `range-store-native/tests/http-consistency.test.js` 可在启动 HTTP service 后对 native SDK 和 HTTP service 做抽样一致性验证：

```powershell
$env:PHS_HTTP_URL = "http://127.0.0.1:8080"
Set-Location range-store-native
bun run test:http-consistency
```

`test:http-consistency` 覆盖 `concrete-lines`、`drill-scenarios`、`hand-strategy`、batch 和 `hands-by-actions`。它是运行时边界一致性测试，不替代源 SQLite full cross verify。

## 发布前验收建议

发布前最小验证：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --target x86_64-pc-windows-msvc

cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode standalone --dir data\range-strata --verify-checksum

cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- verify `
  --mode cross --dir data\range-strata --source data\sqlite\range.db `
  --sample-size 0 --verify-checksum
```

严格数据发布使用 `--sample-size 0` 做全量扫描。日常开发可用 `--sample-size 10000` 做快速抽样回归。

## 局限

- 抽样 cross verify 不能证明未抽中 rows 一定正确；正式发布以全量 cross verify 为准。
- Cross verify 依赖源 SQLite 和二进制产物来自同一数据版本。
- `sourceDbChecksum` 可用于人工核对，但工具链仍应保证构建和验证使用同一源库。
- `PHS_VERIFY_CHECKSUMS=true` 会让运行时查询也做 CRC32C 检查，但会增加每次查询成本。

---

## Benchmark 脚本总览

所有 benchmark 脚本位于 `storage-tools` crate，通过不同 subcommand 选择不同引擎和模式。它们共用同一 workload 生成逻辑（`storage-tools/src/benchmark/workload.rs`），从源 SQLite 的 `range_data_*` 表抽取 hand/batch/hands-by-actions 查询样本，并从 metadata 表派生 `concrete_line` 精确 lookup 样本。

### 统一 workload 机制

workload 是每个 benchmark 的输入基础，包含：

| 字段 | 说明 |
|---|---|
| `hand_queries` | 单手策略查询样本（concrete_line_id + hole_cards） |
| `batch_queries` | 默认批量查询样本（同 `--batch-size`） |
| `batch_queries_by_size` | 多批量大小样本（1, 5, 10, 50, 100） |
| `hands_by_actions_queries` | hands-by-actions 查询样本 |
| `drill_scenario_queries` | drill scenario metadata 查询样本 |
| `dimensions` | 涉及的维度列表 |

`concrete-lines-exact` 不作为 workload JSON 的独立字段保存。runner 会基于 `hand_queries` 中的 `concrete_line_id` 回查 metadata 表得到 `concrete_line` 字符串，并跳过空字符串样本：

- Binary hot benchmark 从运行目录 `meta.db` 的 `concrete_lines_*` 表读取，使用 `concrete_line_id` 列。
- SQLite baseline benchmark 从源库 `concrete_lines_*` 表读取，使用源表 `id` 列。
- 测量时要求精确查询只返回 1 行，并且返回 id 必须等于样本中的 `concrete_line_id`。

workload 可以通过 `--write-workload` 导出为 JSON，供多个 benchmark 复用。也可以通过 `--workload` 加载已有 JSON，避免重新生成。

### 1. 热路径 Binary benchmark

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  [--verify-results] `
  [--dimension default:6:100] `
  [--workload workload.json] `
  [--write-workload workload.json] `
  [--seed 42] `
  [--iterations 3] `
  [--hand-iterations 100] `
  [--batch-iterations 50] `
  [--batch-size 10] `
  [--batch-sizes 1,5,10,50,100] `
  [--warmup-iterations 5] `
  [--workload-mode random|abstract-local] `
  [--verify-checksum]
```

**测量内容：**

| Case | 说明 |
|---|---|
| `concrete-lines-exact` | 按 `concrete_line` 精确 lookup `concrete_line_id`，通过 `CachedMetadataReader` 读取运行目录 `meta.db` |
| `hand-strategy` | 单 concrete_line_id + hand 查询，通过 `StoreQueryService::query()` 解码 pack |
| `batch-hand-strategy` | 默认批量大小（`--batch-size`）的 batch 查询，通过 `StoreQueryService::query_batch()` |
| `batch-size-{N}` | 各批量大小的 sweep 用例 |
| `hands-by-actions` | 完整解码 pack，按 action filter + frequency 阈值匹配手牌 |
| `drill-scenarios-metadata` | 从 meta.db 查询 drill scenario abstract lines |

**报告：** `reports/benchmark-range-strata-binary.json` / `.md`
包含 QPS、avg、p50、p95、p99、max、error count、内存快照。

### 2. SQLite baseline benchmark

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-sqlite `
  --source data\sqlite\range.db `
  [--workload workload.json] `
  [--write-workload workload.json] `
  [--seed 42] `
  [--iterations 3] `
  [--hand-iterations 100] `
  [--batch-iterations 50] `
  [--batch-size 10] `
  [--batch-sizes 1,5,10,50,100] `
  [--warmup-iterations 5]
```

**测量内容：** 与 Binary benchmark 相同的 case，但直接查询源 SQLite `range_data_*`、`concrete_lines_*` 和 `drill_scenario_lines_*` 表。

- `concrete-lines-exact`: `SELECT id FROM concrete_lines_{strategy}_{N}max_{BB}BB WHERE concrete_line=?`
- `hand-strategy`: `SELECT ... FROM range_data_{strategy}_{N}max_{BB}BB WHERE concrete_line_id=? AND hole_cards=?`
- `batch-hand-strategy`: 批量 UNION ALL 查询
- `hands-by-actions`: `SELECT DISTINCT hole_cards FROM ... WHERE concrete_line_id=? AND frequency > ?`
- `drill-scenarios-metadata`: 查询 `drill_scenario_lines_{strategy}` 表

**报告：** `reports/benchmark-sqlite.json` / `.md`

### 3. Binary vs SQLite 对比

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-compare `
  --binary reports\benchmark-range-strata-binary.json `
  --sqlite reports\benchmark-sqlite.json `
  [--out reports\benchmark-compare.json] `
  [--md reports\benchmark-compare.md] `
  [--allow-mismatch]
```

**对比逻辑：**

- 校验 workload 兼容性（dimensions、hand/batch/hands-by-actions 查询数量必须一致；`concrete-lines-exact` 样本由 `hand_queries` 派生）
- 按 case 名匹配 Binary 和 SQLite 报告，包括 `concrete-lines-exact`
- 计算延迟比（Binary / SQLite，>1 表示 Binary 更慢）
- 计算 QPS 比（Binary / SQLite，<1 表示 Binary 吞吐更低）
- 报告每个 case 的 error count 差异

**报告：** `reports/benchmark-compare.json` / `.md`

### 4. 冷启动 benchmark

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --mode process-cold `
  --runs 10 `
  [--dimension default:6:100] `
  [--cache-filler-mb 512] `
  [--query-policy all|fixed] `
  [--fixed-concrete-line-id 1] `
  [--fixed-hand AA] `
  [--fail-fast] `
  [--max-errors-per-dimension 3]
```

**测量阶段：**

| 阶段 | 说明 |
|---|---|
| `import` | Native SDK 模块加载（仅 native-sdk 模式） |
| `constructor` | RangeStoreFacade 构造（manifest 解析 + SQLite 连接打开） |
| `warmup` | prewarm 打开维度文件 |
| `first-query` | 首次查询（含 mmap 创建 + 首次 pack 解码） |
| `process-cold` | 新进程打开 + 查询（含 OS page cache 驱逐） |

**缓存驱逐策略：**

- `--mode process-cold`：新进程模式，不强制驱逐 OS page cache
- `--mode os-cold`：通过写入 filler 文件驱逐 OS page cache（需要 root/admin 权限）

**报告：** `reports/benchmark-cold-start.json` / `.md`

SQLite cold 对比：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-sqlite-cold `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --mode process-cold `
  --runs 10
```

Cold 对比：

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-cold-compare `
  --binary reports\benchmark-cold-start.json `
  --sqlite reports\benchmark-sqlite-cold-start.json
```

### 5. Native benchmark（core / SDK / HTTP 公平对比）

```powershell
cargo run -p poker-hands-storage-tools --target x86_64-pc-windows-msvc -- benchmark-native `
  --dir data\range-strata `
  --source data\sqlite\range.db `
  --native-entry range-store-native\index.js `
  --http-service-bin target\x86_64-pc-windows-msvc\debug\poker-hands-storage-service.exe `
  [--seed 42] `
  [--workload workload.json] `
  [--write-workload workload.json] `
  [--dimension default:6:100] `
  [--batch-size 10] `
  [--batch-sizes 1,5,10,50,100] `
  [--hand-iterations 100] `
  [--batch-iterations 50] `
  [--warmup-iterations 5]
```

**工作原理：** 用同一 workload JSON 在三个独立子进程中并行测试：

| Worker | 入口 | 说明 |
|---|---|---|
| `core` | `StoreQueryService` (Rust) | 直接调用 range-store-core API，无序列化开销 |
| `native-sdk` | `range-store-native` (Bun/NAPI) | 通过 NAPI 桥接，含 JS→Rust 序列化 |
| `http-service` | HTTP REST API | 通过 localhost HTTP 请求，含网络往返 |

每个 worker 独立测量：
- 内存快照（import/constructor/warmup 前后）
- metadata lookup：`concrete-lines-exact` 按 `concrete_line` 精确解析 `concrete_line_id`；`drill-scenarios-metadata` 按 drill 条件读取 abstract lines
- 策略/范围查询：`hand-strategy`、`batch-hand-strategy`、`batch-size-*`、`hands-by-actions`
- 组合链路：`line-to-hands-by-actions`，先做 `concrete_line -> concrete_line_id`，再执行 `hands-by-actions`

`concrete-lines-exact` 和 `drill-scenarios-metadata` 都走 `meta.db` + `CachedMetadataReader`，只是业务入口和 SQL 表不同。当前 Native benchmark 单独输出的是 concrete-line exact lookup；`abstract_line`、`concrete_line`、`abstract_line + concrete_line` 三种筛选语义由接口一致性测试覆盖，不作为这里的三个独立性能 case。

**随机化执行顺序：** 通过 `--seed` 随机化三个 worker 的执行顺序，避免 OS 调度偏差。

### 报告生成代码边界

benchmark 报告代码只抽取低层公共 helper，不合并各类报告的数据结构和渲染入口：

- `benchmark/report_support.rs` 放跨报告复用的写文件、UTC 时间、通用耗时格式、binary bytes 格式和 Markdown 表格 helper。
- `benchmark/report.rs` 保留 hot Binary、SQLite baseline、metadata 和 native benchmark 的主报告结构和渲染。
- `benchmark/compare/runner.rs` 读取 `benchmark/report.rs` 的 `BenchmarkRunReport` 作为 Binary/SQLite 输入；最终对比报告由 `benchmark/compare/report.rs` 渲染。
- `benchmark/compare/report.rs` 保留 Binary vs SQLite hot 对比报告。
- `benchmark/cold/report.rs` 和 `benchmark/cold/compare.rs` 保留冷启动报告及冷启动对比报告；cold-start 的时间和字节展示语义独立维护，不强行套用 hot 报告格式。

### 控制参数总览

| 参数 | 适用 | 说明 |
|---|---|---|
| `--seed` | 所有 | 随机种子，控制 workload 生成和 native benchmark 执行顺序 |
| `--iterations` | hot | 总迭代次数 |
| `--hand-iterations` | hot | 单手查询迭代次数 |
| `--batch-iterations` | hot | 批量查询迭代次数 |
| `--batch-size` | hot | 默认批量大小 |
| `--batch-sizes` | hot | 批量大小 sweep 列表 |
| `--warmup-iterations` | hot | 预热迭代次数（不计入结果） |
| `--workload-mode` | hot | `random`（随机采样）或 `abstract-local`（按 abstract_line 本地化） |
| `--workload` | hot/sqlite/native | 加载已有 workload JSON |
| `--write-workload` | hot/sqlite/native | 导出 workload JSON |
| `--dimension` | 所有 | 指定维度过滤（`default:6:100` 或 `default_6max_100BB`） |
| `--verify-results` | hot | 前 100 条查询结果与源 SQLite 校验 |
| `--verify-checksum` | hot/cold | 查询时校验 pack CRC32C |
| `--runs` | cold | 每个维度的运行次数 |
| `--mode` | cold | `process-cold`（新进程）或 `os-cold`（驱逐 page cache） |
| `--cache-filler-mb` | cold | 用于驱逐 OS page cache 的 filler 文件大小 |
| `--query-policy` | cold | `all`（所有维度）或 `fixed`（固定 concrete_line_id） |
| `--fixed-concrete-line-id` | cold | 固定查询的 concrete_line_id |
| `--fixed-hand` | cold | 固定查询的 hole_cards |
| `--fail-fast` | cold | 遇到失败立即停止 |
| `--max-errors-per-dimension` | cold | 每个维度最大允许错误数 |

### 注意事项

- 不同 workload mode、dimension、sample set 的报告不可直接对比
- 对比前确保 Binary 和 SQLite 使用相同 workload（通过 `--workload` 复用 JSON）
- 冷启动结果需区分：进程启动、metadata 打开、mmap 创建、首次查询、OS page-cache 影响
- `process-cold` 不强制驱逐 OS page cache，只代表当前机器的新进程 open/query 成本
- Native benchmark 的 OS page cache 仍跨 worker 进程共享，多次运行取稳定值
- 正式 benchmark 结论只更新 `docs/binary-vs-sqlite-benchmark-and-verification-report.md`
