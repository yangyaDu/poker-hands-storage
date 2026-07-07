# SQLite 与二进制数据一致性验证报告

更新日期：2026-07-05

## 目标

验证分为两类：

1. 二进制产物自洽：`manifest.json`、`meta.db`、`.idx`、`.bin` 的格式、引用和 checksum 是否正确。
2. 与源 SQLite 一致：从源 `range.db` 抽样或全量读取 rows，解码二进制 pack 后逐项比对。

当前工具位于 `storage-tools`，运行时核心格式能力位于 `range-store-core`。

## 验证覆盖面总览

当前验证分为六层：

| 层级 | 覆盖内容 | 主要入口 |
| --- | --- | --- |
| 格式自洽 | `manifest.json`、`meta.db`、`.idx`、`.bin`、header、record 边界、CRC32C | `storage-tools verify --mode standalone` |
| 源数据一致性 | 二进制解码结果与源 SQLite `range_data_*` rows 对齐 | `storage-tools verify --mode cross` |
| Float32 精度 | `frequency`、`hand_ev` 按 IEEE754 Float32 bit-exact 比对 | cross verify |
| 查询结果抽样 | benchmark workload 下 Binary 和 SQLite result count / case 兼容性 | `benchmark --verify-results`、`benchmark-compare` |
| API 边界 | HTTP 路由、OpenAPI、请求校验、错误码、batch all-or-nothing 语义 | `service/tests/http/*` |
| Native 边界 | Bun SDK 直接 payload、`RangeStoreError`、lazy schema cache、native 与 HTTP 抽样一致性 | `range-store-native/tests/*` |

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

2026-07-05 的新增工作主要是 native/HTTP benchmark、drill metadata microbenchmark、metadata lazy cache 与 indexed meta 复测；full cross verify 结果仍以 2026-07-01 的全量报告为当前数据正确性快照。若重新生成发布目录，应对新目录重新执行 full cross verify。

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
- `INVALID_ARGUMENT`
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
