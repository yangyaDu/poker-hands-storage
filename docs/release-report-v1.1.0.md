# poker-hands-storage v1.1.0 项目进展报告

## 一、项目概述

本项目是一个独立的 Rust 存储与查询服务，用于读取 `preflop-storage` 产出的 Range Strata Binary 数据。核心目标是：**用自定义二进制格式替代 SQLite 直接查询，实现更高的查询性能和更小的磁盘占用。**

### 数据链路

```
1.45GB SQLite 源数据 -> 345.5MB Range Strata Binary -> HTTP Service / Bun Native SDK
```

| 指标                        | 结果                                  |
| --------------------------- | ------------------------------------- |
| 磁盘节省                    | 76% (1,447 MB -> 346 MB)              |
| 单手查询 QPS 提升           | 6.4x                                  |
| 批量查询 QPS 提升 (size 20) | 36.2x                                 |
| hands-by-actions QPS 提升   | 9.45x                                 |
| 数据完整性验证              | 23.8M 记录，0 失败，float32 bit-exact |
| 单元测试                    | 100+ 测试全部通过                     |

### 项目结构（4 个 crate）

```
range-store-core    ← 纯 Rust 库：二进制存储引擎（mmap、编解码、查询），不依赖 HTTP 或 CLI
    ^
    |--- service    ← HTTP API 服务（axum + OpenAPI），8 个端点
    |--- range-store-native  ← Bun/Node.js 进程内 SDK（napi-rs）
    |--- storage-tools       ← 离线工具：构建、验证、benchmark CLI
```

核心 crate 不依赖任何其他项目 crate，这是关键的设计约束，保证存储引擎可以独立被任何消费者引用。

---

## 二、二进制编码流程详解

### 2.1 两种自定义二进制格式

| 格式     | 文件后缀 | 全称                     | 作用                                               |
| -------- | -------- | ------------------------ | -------------------------------------------------- |
| **PFSP** | `.bin`   | Poker Face Strategy Pack | 存储实际的策略数据（frequency、hand_ev）           |
| **PFXI** | `.idx`   | Poker Face Index         | 索引文件，告诉程序每个 pack 在 .bin 中的位置和长度 |

### 2.2 PFSP 文件格式（.bin）

```
Offset  Size  字段
0-3     4B    Magic: "PFSP"
4-5     2B    Version: u16 LE = 1
6       1B    Endianness: 1 = little-endian
7       1B    Float type: 1 = float32
8       1B    Layout: 1 = sparse hand-major v1
9       1B    Compression: 0 = none
10-11   2B    Header size: u16 LE = 16
12-15   4B    Reserved (zero)
16+     var   Pack 1 | Pack 2 | Pack 3 | ...
```

Pack 之间没有分隔符，通过 `.idx` 文件中记录的 `offset` 和 `byte_length` 来定位每个 pack 的边界。

### 2.3 PFXI 文件格式（.idx）

```
Offset  Size  字段
0-3     4B    Magic: "PFXI"
4-5     2B    Version: u16 LE = 1
6-7     2B    Reserved
8-11    4B    Record count: u32 LE (等于相应维度下concrete line的总数)
12-13   2B    Header size: u16 LE = 16
14-15   2B    Reserved
16+     var   Record 1 (22B) | Record 2 (22B) | ...
```

每条 IdxRecord（22 字节）：

```
Offset  Size  字段
0-3     4B    concrete_line_id: u32 LE
4-7     4B    action_schema_id: u32 LE
8-9     2B    hand_count: u16 LE
10-13   4B    offset: u32 LE（pack 在 .bin 文件中的偏移）
14-17   4B    byte_length: u32 LE（pack 在 .bin 中的字节长度）
18-21   4B    checksum: u32 LE（pack 数据的 CRC32C）
```

**关键约束**：记录要求 `concrete_line_id` 连续递增、无跳跃。在 open 阶段通过 `validate_dense_index_layout()` 验证，如果有 gap 则拒绝打开。这使得查找不需要二分搜索，直接 `index = concrete_line_id - first_concrete_line_id` 下标访问，O(1) 定位。

### 2.4 编码流水线（从 SQLite 到二进制文件）

**第 1 步：发现维度**

- 从 SQLite 源数据库的 `range_data_*` 表中自动发现所有维度
- 维度命名格式：`{strategy}_{player_count}max_{depth}BB`，如 `default_6max_100BB`
- 通过查询 `sqlite_schema` 表动态发现，不硬编码

**第 2 步：手牌编码（169 字典）**

- 标准德州扑克有 1326 种不同的两张牌组合，但由于对称性（AKs = KAs），可以压缩到 169 种
- 用 13x13 矩阵编码：
  - 对角线：排列型（AA=0, KK=13, QQ=26, ..., 22=156），公式 `hand_id = rank_idx * 13 + rank_idx`
  - 上三角：同花不同花（AKs=13, AQs=14, ...），公式 `hand_id = high_idx * 13 + low_idx`
  - 下三角：不同花（AKo=78, AQo=79, ...），公式 `hand_id = low_idx * 13 + high_idx`
- 每种手牌用一个 `u8` (0-168) 表示
- 输入 `AsKh`、`KA`、`AKs` 等不同格式都会被 `parse_hole_cards()` 规范化为统一的 169 手牌码

**第 3 步：动作 Schema 编码（9 字节/条）**

- 每条动作定义序列化：1 字节 action_type + 4 字节 action_size(f32) + 4 字节 amount_bb(f32)
- 6 种动作类型：fold(0), call(1), check(2), bet(3), raise(4), allin(5)
- 去重后存储在 `meta.db` 的 `action_schemas` 表中
- 通过 hex SHA-256 的 `schema_key` 作为唯一键，`action_schema_id` 作为自增主键
- CRC32C 校验 action_blob 完整性
- 通过 `action_schema_id` 引用，避免在每个 pack 中重复存储动作定义

**第 4 步：Pack 打包（核心编码）**

- 按 `concrete_line_id` 分组，每组打包为一个 "pack"
- 布局公式：`byte_length = hand_count * (5 + action_count * 8)`
  - 1 字节：hand_id（u8，排序后存储）
  - 4 字节：action_mask（u32 位掩码，第 N 位为 1 表示第 N 个动作存在）
  - 8 字节 × action_count：每个动作的 frequency(f32) + hand_ev(f32)
- 例如：169 手牌 × 32 动作 = 169 × 261 = 44,109 字节
- 手牌在 pack 中按 hand_id 升序排列，支持二分查找
- hand_ev 为 NaN 时存储为 null（f32::NAN 的 bit pattern 是确定的）

**第 5 步：CRC32C 校验**

- 每个 pack 计算 CRC32C（Castagnoli 多项式），存储在 idx record 中
- 使用 `crc32c` crate，自动选择最优实现：SSE4.2 / ARM CRC / 纯软件
- 标准测试向量：`crc32c(b"123456789") == 0xE3069283`

**第 6 步：写入文件**

- 每个维度的输出先写 `.tmp` 文件，构建成功后原子 rename
- `.bin` 文件：16 字节 PFSP 头 + 所有 pack 顺序拼接（变长，pack 之间无分隔）
- `.idx` 文件：16 字节 PFXI 头 + 每条记录 22 字节（sorted by concrete_line_id）
- `manifest.json`：记录所有维度的元数据（strategy、player_count、depth_bb、文件路径、concrete_line_count、pack_count）
- `meta.db`：SQLite 文件，包含 build*info、action_schemas、dimension_action_schemas、concrete_lines*{dimension}、drill_scenario_lines 等表

### 2.5 解码查询（零拷贝 mmap 路径）

查询时底层使用了 **mmap（内存映射）** 技术，整个路径没有一次 `read()` 系统调用：

**Step 1: IdxReader.find() — O(1) 密集索引查找**

```
index = concrete_line_id - first_concrete_line_id  // 一次减法
offset = index * 22  // 一次乘法
record = mmap[16 + offset .. 16 + offset + 22]  // 直接切片
```

对比 SQLite：SQL 解析 → 准备语句 → B-tree 索引查找（可能 3-4 层节点遍历）→ 页读取 → 行解析。

**Step 2: BinReader.read_pack() — 零拷贝读包**

```
pack = &mmap[offset .. offset + byte_length]  // 返回 &[u8] 引用
```

没有 `malloc`、没有 `memcpy`、没有中间缓冲区。OS 会在首次访问这些页时触发 page fault，从磁盘加载到 page cache。

**Step 3: decode_pack_for_hand() — 位运算解码**

```
hand_idx = binary_search_u8(hand_ids, target_hand_id)  // log(hand_count) ≈ 8 次比较
mask = pack[hand_count + hand_idx * 4 .. + 4]          // 读 u32
cell_offset = hand_count * 5 + hand_idx * action_count * 8  // 算术偏移
for action_id in 0..action_count {
    if (mask >> action_id) & 1 == 0 { continue; }      // 一条 CPU 指令
    freq = f32::from_le_bytes(cell_data[action_id * 8 .. + 4])
    ev   = f32::from_le_bytes(cell_data[action_id * 8 + 4 .. + 4])
}
```

- 固定大小 cell（8 字节），直接算术偏移计算，不用解析变长字段
- 位掩码过滤 `(mask >> action_id) & 1` 是一条 CPU 指令
- 结果存在栈上 `ArrayVec<32>`，不需要堆分配
- NaN hand_ev 解码为 `None`

**Step 4: ActionSchemaCache.get() — HashMap 查找**

- 懒加载：第一次查到 SQLite，之后缓存在 `HashMap<u32, Arc<Vec<ActionDef>>>`
- Cache hit 走 RwLock read path，不碰 Mutex，不查 SQLite
- `Arc::clone` 是 O(1) 指针操作

### 2.6 三种查询模式

| 查询类型             | 解码范围                       | 使用场景                                    |
| -------------------- | ------------------------------ | ------------------------------------------- |
| `query()`            | 解码单个 hand_id               | 单手牌查询                                  |
| `query_many_hands()` | 从同一个 pack 解码多个 hand_id | batch 查询（同一个 concrete_line 的多手牌） |
| `query_all()`        | 完全解码整个 pack（169 手牌）  | hands-by-actions、drill 场景                |

---

## 三、Benchmark 体系详解

### 3.1 Hot Benchmark（热查询）

**场景**：服务已经打开，维度已经预加载到内存中，测的是纯粹的数据查询和编解码性能。

**测量流程**：

1. 从 SQLite 源数据中按种子随机采样 workload（保证可复现）
2. 打开 `StoreQueryService`（max_open_handles=100，预加载所有需要的维度）
3. 预加载 workload 涉及的所有维度：`service.prewarm(&dimension)`
4. 对以下 5 类查询执行多次测量，记录延迟和 QPS

**测量的查询类型**：

| 查询类型                   | 代码路径                                          | 说明                                           |
| -------------------------- | ------------------------------------------------- | ---------------------------------------------- |
| `hand-strategy`            | `service.query()`                                 | 单手牌查询，一次 idx lookup + 一次 pack decode |
| `batch-hand-strategy`      | `service.query_batch()`                           | 批量查询，默认 batch_size                      |
| `batch-size-{N}`           | `service.query_batch()`                           | 批量大小扫掠 [1, 5, 10, 50, 100]               |
| `hands-by-actions`         | `service.query_hands_by_actions()`                | 解码整个 pack + 位掩码过滤                     |
| `drill-scenarios-metadata` | `CachedMetadataReader.get_drill_scenario_lines()` | 查询 drill 场景的抽象线路                      |

**指标**：p50 / p90 / p95 / p99 延迟 + QPS

**Percentile 计算**：对 `times_ms` 排序后用线性插值：`index = (p/100) * (len-1)`，取 floor/ceil 位置插值。

**QPS 计算**：`iterations / (total_ms / 1000)`

**结果验证**：抽样最多 100 次 hand-strategy 查询，对比 binary 返回的 actions.len() 和 SQLite `SELECT COUNT(*)` 的结果。

### 3.2 Cold Benchmark（冷启动）

**场景**：全新进程启动，OS page cache 中没有数据，测的是文件打开、mmap 建立、第一次数据解码的完整开销。

**测量流程**：

1. **清除 OS 缓存**：
   - `ProcessCold`：不清除，只是全新进程
   - `OsBestEffort`：写一个填充文件（大小 = max(512MB, dataset_size \* 2)）再删除，迫使 OS 将真实数据页换出
   - `LinuxDropCache`：`sync` + 写 `3` 到 `/proc/sys/vm/drop_caches`
2. **启动子进程**（`cold-worker` 子命令）测量 4 个阶段的时间
3. **记录内存变化**（Windows 用 `GetProcessMemoryInfo`，Linux 读 `/proc/self/status` VmRSS）

**4 个阶段**：

| 阶段                   | 测量内容                              | 含义                                                                              |
| ---------------------- | ------------------------------------- | --------------------------------------------------------------------------------- |
| `service_open_ms`      | `StoreQueryService::open_with_meta()` | manifest 解析 + meta.db 打开 + HandlePool 初始化                                  |
| `dimension_prewarm_ms` | `service.prewarm(&dimension)`         | mmap 某个维度的 .idx/.bin 文件                                                    |
| `first_query_ms`       | `service.query()`                     | 一次完整的 hand-strategy 查询（idx lookup + pack read + decode + schema resolve） |
| `close_ms`             | `drop(service)`                       | 释放 mmap、关闭文件                                                               |

**`store_open_and_first_query_ms`** = 前三者之和，代表用户从启动到拿到结果的端到端延迟。

**Phase Accounting 校验**：`phase_sum = open + prewarm + query + close`，`unaccounted_ms = worker_total - phase_sum`。如果 unaccounted > 1ms 或 > 1%，说明测量有误差，需要排查。

**为什么用子进程**：确保每次测量都是全新的进程和文件描述符状态。如果在同一个进程中多次测量 cold start，之前的 mmap 文件描述符、OS page cache、Rust 内存分配器缓存都会影响结果。

**SQLite Cold Worker**：同样的结构但用 raw SQLite，没有 prewarm 阶段（`dimension_prewarm_ms = 0`），因为 SQLite 每次查询都打开连接。

### 3.3 Native Benchmark（Core / HTTP Service / SDK）

**三种模式**，在同一轮测试中 fair-ordered（用 seed 随机排序 entry order，防止 OS page cache 偏差）：

| 模式         | 实现方式              | 测量内容                                      |
| ------------ | --------------------- | --------------------------------------------- |
| Core         | 同进程 Rust 库        | 直接调用 range-store-core，最接近底层性能     |
| HTTP Service | 启动独立 service 进程 | POST 请求到 axum HTTP 端点，含网络开销        |
| SDK (Bun)    | `bun run worker.mjs`  | 导入 index.node 原生模块，含 N-API 序列化开销 |

**Native SDK 额外测量的阶段**：import time、constructor time、first query time、warmup time，以及各阶段的 memory snapshot。

### 3.4 Workload 生成与采样

**两种模式**：

- `random`：完全随机选择 concrete_line_id 和手牌
- `abstract-local`：基于抽象线路局部采样——先随机选一个 abstract_line，再在该 line 的 concrete_line_ids 列表中取连续窗口

**采样方法**：

- 从 SQLite 源数据中 `discover_dimensions()` 发现所有 `range_data_*` 表
- 按维度行数加权随机选择维度
- 使用 `SeededRandom`（additive PRNG + bit mixing），给定相同 seed 总是产生相同序列
- 可序列化到 JSON 文件，供多次 benchmark 复用

**Batch 采样**：

- 随机选择维度（按行数加权）
- 生成 batch_size 个随机 (concrete_line_id, hole_cards) 对
- 用 HashSet 去重，最多重试 batch_size \* 3 次

**Hands-by-actions 采样**：

- 随机选一个 concrete_line_id
- 从源 SQLite 查询 DISTINCT action_names（frequency > 0.005）
- 生成 action filter 列表

### 3.5 性能提升的本质原因

| 技术             | SQLite 路径                                       | Binary 路径                         | 效果                |
| ---------------- | ------------------------------------------------- | ----------------------------------- | ------------------- |
| 定位数据         | SQL 解析 → 准备语句 → B-tree 遍历                 | `index = id - first_id`，直接下标   | O(log n) → O(1)     |
| 读取数据         | read() 系统调用 → 页缓存复制 → 行解析             | mmap 零拷贝 → `&[u8]` 切片          | 一次系统调用 → 零次 |
| 解码数据         | 文本字段解析 → 类型转换 → 变长字段                | 固定偏移算术计算 → 位运算           | 动态 → 静态         |
| 动作名称         | SQL JOIN meta.db → 另一次 B-tree 查找             | HashMap.get(action_schema_id)       | 2 次 I/O → 0 次 I/O |
| hands-by-actions | WHERE (action='X' OR action='Y') AND freq > 0.005 | hand_mask & filter_mask != 0        | 字符串比较 → 位运算 |
| Batch 查询       | N 次独立 SQL 执行                                 | 一次 pool.get_or_open() + 共享 pack | 重复打开 → 复用     |

---

## 四、验证体系详解

### 4.1 Standalone Verification（独立验证）

**对比基准**：`manifest.json` 声明的内容与实际文件是否一致。

**验证层级**（按执行顺序）：

| 层级                       | 验证内容                                                                    | 失败原因示例                                       |
| -------------------------- | --------------------------------------------------------------------------- | -------------------------------------------------- |
| manifest.json              | 格式 ("PFSP")、版本 (1)、每个维度的 idxFile/binFile 存在                    | INVALID_JSON, UNSUPPORTED_FORMAT, MISSING_FILE     |
| meta.db 存在性             | 文件存在                                                                    | IO_ERROR                                           |
| meta.db build_info         | build_info 表有 built_at 和 source_checksum 行                              | MISSING_BUILD_INFO                                 |
| meta.db action_schemas     | 每个 action_blob.length == action_count \* 9，CRC32C 匹配，schema_key 匹配  | SCHEMA_KEY_MISMATCH, ACTION_BLOB_CHECKSUM_MISMATCH |
| meta.db 维度表             | 每个维度的 concrete*lines*{dim} 表和 drill*scenario_lines*{strategy} 表存在 | MISSING_TABLE                                      |
| .idx 文件魔数              | 前 4 字节 = "PFXI"                                                          | INVALID_MAGIC                                      |
| .idx 文件版本              | 字节 4-5 = 1                                                                | UNSUPPORTED_VERSION                                |
| .idx 文件记录连续性        | concrete_line_id 严格递增、无跳跃、无重复                                   | NON_DENSE_INDEX, DUPLICATE_CONCRETE_LINE_ID        |
| .idx 文件 hand_count       | 0 < hand_count <= 169                                                       | HAND_COUNT_EXCEEDED                                |
| .idx 文件 action_schema_id | 引用存在于 meta.db 的 schema                                                | ACTION_SCHEMA_NOT_FOUND                            |
| .bin 文件魔数              | 前 4 字节 = "PFSP"                                                          | INVALID_MAGIC                                      |
| .bin 文件头                | 端序=LE, float=float32, layout=sparse-v1, compression=none                  | INVALID_HEADER_FIELDS                              |
| 索引-包偏移                | record.offset >= 16（不能落在 header 区域）                                 | INVALID_OFFSET                                     |
| 索引-包边界                | offset + byte_length <= file_size                                           | OUT_OF_BOUNDS                                      |
| 索引-包大小                | byte*length == hand_count * (5 + action*count * 8)                          | PACK_SIZE_MISMATCH                                 |
| 索引-包 CRC32C             | 可选，需 `--verify-checksum` 参数                                           | CHECKSUM_MISMATCH                                  |
| 索引-包 hand_id            | pack 中的 hand_ids 升序排列、每个在 [0, 168]                                | HAND_IDS_NOT_SORTED                                |

### 4.2 Cross Verification（交叉验证）

**对比基准**：源 SQLite 数据库 vs 二进制文件逐单元格对比。

**采样机制**：

- 从源 SQLite 的动态表名（`range_data_{strategy}_{N}max_{BB}BB`）中采样
- 按比例分配：`quota_for_dimension = floor(dimension_row_count / total_rows * sample_size)`
- 默认 `sample_size = 10,000`；`sample_size = 0` 时全量比对
- 使用确定性哈希排序：`ORDER BY ((concrete_line_id * 1103515245 + id * 12345) & 0x7FFFFFFF)`
- 全量模式：`ORDER BY concrete_line_id, hole_cards, action_name`

**逐单元格对比流程**：

1. 从 SQLite 读取一行：`concrete_line_id, hole_cards, action_name, action_size, amount_bb, frequency, hand_ev`
2. `hole_cards` → `get_hand_id()` 解析为 0-168 的 hand_id
3. 通过 .idx 找到对应的 concrete_line_id 的 record，获取 pack 的 offset 和 byte_length
4. 从 .bin 读取 pack 数据，用 `decode_pack_for_hand()` 解码
5. 匹配 action：normalize `action_name`（trim, lowercase, strip `-` 和 `_`），在 action_schemas 中查找匹配的 (action_name, action_size, amount_bb)
6. 计算 cell_index = hand_index \* actions.len() + action.action_id
7. 对比：
   - `cell.exists` 必须为 true
   - `frequency`：bit-exact 比较（f64→f32→f64 截断后比对 bit pattern）
   - `hand_ev`：nullable bit-exact 比较（双方都 null 或 bit-exact）

**反向检测**：当 `sample_size = 0` 时，解码完所有源数据后，遍历 pack 中所有 `cell.exists == true` 的单元格，检查是否有 SQLite 中没有的记录（`extra_binary_records`）。

**Float32 精度统计**：

- `quantization_abs_error` = `|source_f64 - (source_f64 as f32 as f64)|`
- `quantization_relative_error` = `quantization_abs_error / max(|source|, 1.0)`
- `implementation_abs_error` = `|stored_f32_as_f64 - ideal_truncated_f64|`
- reservoir sampling (size 8192) 跟踪 p95/p99 quantization error
- 记录 top-20 最大误差样本

**为什么用 bit-exact 而非容差**：如果允许 1e-6 的 tolerance，SQLite pager 读取错误、pack 解码偏移错误、action_schema 查找错误都可能被 tolerance 掩盖。bit-exact 确保任何非精度损失的差异都能被捕获。

### 4.3 验证成果

- 全量 cross verify 覆盖 9 个维度、23,806,716 条源记录，失败数为 0
- Float32 bit-exact 匹配
- 量化误差：f32 尾数 23 位，有效数字约 7 位十进制，对 poker strategy 的 frequency (0-1) 和 hand_ev (0-5 BB) 完全可接受

---

## 五、断点续跑（Resume Build）详解

### 5.1 build-state.json 结构

```jsonc
{
  "version": 1,
  "source_checksum": "sha256_of_source_sqlite_file",
  "max_concrete_lines": 10000,
  "dimensions": [
    {
      "key": "default:6:100",
      "status": "completed",
      "idx_file": "ranges_default_6max_100BB.idx",
      "bin_file": "ranges_default_6max_100BB.bin",
      "concrete_line_count": 345,
      "pack_count": 345,
    },
  ],
}
```

**状态枚举**：`pending` → `in_progress` → `completed` / `failed`

### 5.2 Resume 完整流程

1. **检测状态文件**：`--resume` 参数开启且 `build-state.json` 存在 → 加载已有状态
2. **校验一致性**（防止用错数据源重建）：
   - `source_checksum` 必须匹配（源文件字节级 checksum）
   - 维度列表必须匹配
   - `--max-concrete-lines` 必须一致
3. **跳过已完成维度**：`status == "completed"` 的维度直接跳过
4. **处理中断维度**：`status == "in_progress"` 的维度重新构建
5. **文件完整性校验**：对 completed 维度重新检查 .idx/.bin 文件大小和 CRC32C

### 5.3 为什么 source_checksum 检测的是文件替换而非数据一致性

`source_checksum` 是对源 SQLite **整个文件内容**做 checksum（不是对表数据），所以：

- 同一个文件不变 → checksum 不变 → resume 通过
- ALTER TABLE（即使数据不变）→ SQLite 生成新文件（page 布局变化）→ checksum 变 → resume 拒绝
- 重新 export 相同数据但文件字节不同 → checksum 变 → resume 拒绝

更精细的数据校验靠的是 build 过程中记录的每维度 `pack_count` 和 `concrete_line_count`，resume 时对比这些数字。

### 5.4 安全机制

- `--resume` 和 `--overwrite` 互斥，防止误操作
- 临时文件使用 `.tmp` 后缀，构建成功后原子 rename
- 维度级别的失败不影响其他维度
- 中途崩溃后 `.tmp` 文件残留，下次 resume 时因 status 为 failed 会重新构建，rename 时覆盖

---

## 六、Native SDK 使用详解

### 6.1 架构分层

```
Bun/Node.js 应用
    |
    v
index.js (JS 包装层)                    ← 业务信封格式转换、错误处理
    |
    v
index.node (napi-rs 原生绑定)           ← Rust ↔ JS 类型转换、N-API 序列化
    |
    v
RangeStoreFacade                        ← 统一门面：CachedMetadataReader + StoreQueryService
    |
    +-- CachedMetadataReader             ← meta.db 懒加载 + 内存缓存
    |       |                             (RwLock<HashMap> 读多写少)
    |       v
    |    meta.db (SQLite, 只读)
    |
    +-- StoreQueryService
            |
            +-- ActionSchemaCache        ← action_schemas 懒加载 HashMap
            |
            +-- HandlePool (LRU)         ← DimensionReader 缓存池
                    |
                    +-- IdxReader (mmap) ← .idx 文件，O(1) 密集索引查找
                    +-- BinReader (mmap) ← .bin 文件，零拷贝 pack 读取
```

### 6.2 安装与构建

```powershell
cd range-store-native
bun install
bun run build:native    # 编译 Rust 代码生成 index.node (napi-rs)
```

### 6.3 API 完整用法

```javascript
import { PokerHandsRange } from "./range-store-native/index.js";

// 1. 初始化
const store = new PokerHandsRange({
  dataDir: "./data/range-strata",
  maxOpenHandles: 2, // HandlePool 最大容量（默认 2）
  verifyChecksums: false, // 查询时是否校验 CRC32C
});

// 2. 单手牌查询
const result = store.queryHandStrategy({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLineId: 42,
  holeCards: "AKs",
});
// { code: 0, data: { inputHoleCards: "AKs", handCode: "AKs", actions: [...] }, message: null }

// 3. 批量查询（error-tolerant：单个失败不影响其他项）
const batchResult = store.queryBatch({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  items: [
    { concreteLineId: 1, holeCards: "AA" },
    { concreteLineId: 2, holeCards: "KK" },
  ],
});
// { code: 0, data: { results: [{ concreteLineId: 1, holeCards: "AA", actions: [...] },
//                            { concreteLineId: 2, holeCards: "KK", error: { code: 404, message: "..." } }] }, message: null }

// 4. 按动作筛选手牌
const handsResult = store.handsByActions({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLineId: 42,
  actions: ["call", "fold"], // OR 语义：任一匹配即可
  frequency: 0.005, // 默认 0.005，严格大于
});
// { code: 0, data: { holeCards: ["AA", "KK", "AKs", ...] }, message: null }

// 5. 查询 concrete lines
const linesResult = store.getConcreteLines({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  abstractLine: "F-F-F", // 可选：按 abstract 或 concrete 过滤
  concreteLine: "F-F-F-R2", // 可选
});
// { code: 0, data: { lines: [{ concreteLineId: 42, abstractLine: "F-F-F", concreteLine: "F-F-F-R2" }] } }

// 6. 查询 drill 场景的抽象线路
const abstractResult = store.getAbstractLines({
  strategy: "default",
  drillName: "rfi",
  playerCount: 6,
  drillDepth: 100,
});
// { code: 0, data: { abstractLines: ["F-F-F", "F-F-F-R2", ...] } }

// 7. 预加载维度
store.prewarm({ strategy: "default", playerCount: 6, depthBb: 100 });

// 8. 查看统计信息
const stats = store.stats();
// { code: 0, data: { schemaCount: 15, openHandleCount: 3, knownDimensions: [...] } }
```

### 6.4 错误处理

所有 API 返回统一信封格式：

```typescript
{ code: number, data: T | null, message: string | null }
```

错误码映射：

| 错误类型                                     | HTTP code | N-API code |
| -------------------------------------------- | --------- | ---------- |
| 非法参数（手牌格式错误、action filter 无效） | 1000      | 1000       |
| 资源不存在（维度/手牌/concrete_line 未找到） | 404       | 404        |
| 内部错误（manifest 损坏、文件 IO 失败）      | 500       | 500        |

### 6.5 Singleton 模式

```javascript
import { getPokerHandsRangeSingleton } from "./range-store-native/index.js";

const store = getPokerHandsRangeSingleton({ dataDir: "./data" });
// 后续相同 options 的调用返回同一个实例
// 不同 options 会抛错： "PokerHandsRange singleton was already initialized with different options"
```

### 6.6 SDK 与 HTTP Service 的关系

两者底层使用同一套 `range-store-core` 查询逻辑。SDK 通过 napi-rs 编译成 `.node` 原生模块嵌入 JS 进程，零网络开销；HTTP Service 是独立的 axum 服务，通过 REST API 访问，适合多语言客户端和分布式部署。

---

## 七、v1.1.0 新增内容

- **CachedMetadataReader**：惰性加载元数据索引，消除批量查询中的重复 SQLite 查找。构造时只读 manifest，查询时先查内存 HashMap，miss 才查 SQLite，结果缓存。使用 RwLock<HashMap> 而非 Mutex 的原因是 batch 查询中大量 cache hit 场景下多个并发请求可以同时持有读锁而不互斥。由于整个项目只有构建时写、运行时只读（meta.db 以 `read_only` 模式打开，`.bin/.idx` 以 mmap 只读映射），cache 是 append-only 的——LRU eviction 仅是内存淘汰不改文件。不用 `OnceLock<HashMap>`（一次性预加载全部 metadata）而用增量懒加载，是因为 concrete_lines 表可能很大，按需加载避免初始化时占用过多内存。
- **Native benchmark runner**：新增 metadata drill 和 `hands_by_actions` 单链路 benchmark，覆盖 Core/HTTP/SDK 三种模式
- **P90 延迟指标**：补充尾部延迟可视化
- **查询批错误传播**：单个失败不影响整个 batch 的其余项，每个 item 独立返回 `{ actions, error }`
- **Service directory refactor**：简化 service 模块组织，职责边界更清晰
- **文档更新**：扩展 README、docs index、verification guide、tier1 optimization plan；添加 agent references
