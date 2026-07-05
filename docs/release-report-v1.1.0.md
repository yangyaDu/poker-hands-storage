# poker-hands-storage v1.1.0 项目进展报告

## 一、项目概述

本项目是一个独立的 Rust 存储与查询服务，用于读取 `preflop-storage` 产出的 Range Strata Binary 数据。核心目标是：**用自定义二进制格式替代 SQLite 直接查询，实现更高的查询性能和更小的磁盘占用。**

### 数据链路

```
1.45GB SQLite 源数据 -> 345.5MB Range Strata Binary -> HTTP Service / Bun Native SDK
```

磁盘节省 **76%**，策略查询 QPS 提升 **6.4x-36.2x**。

---

## 二、五大核心模块详解

### 1. 二进制编码流程

#### 2.1.1 整体架构

项目定义了两种自定义二进制格式：

| 格式 | 文件后缀 | 作用 |
|------|---------|------|
| **PFSP** (Poker Face Strategy Pack) | `.bin` | 存储实际的策略数据（频率、EV） |
| **PFXI** (Poker Face Index) | `.idx` | 索引文件，告诉程序每个数据包在 .bin 中的位置 |

#### 2.1.2 编码流水线（从 SQLite 到二进制文件）

**第 1 步：发现维度**
- 从 SQLite 源数据库的 `range_data_*` 表中自动发现所有维度
- 维度命名格式：`{strategy}_{player_count}max_{depth}BB`，如 `default_6max_100BB`

**第 2 步：手牌编码（169 字典）**
- 德州扑克 169 种等价手牌被压缩为 `u8` (0-168)
- 排列型（AA, KK...）在对角线，同花不同花（AKs, AKo）分别编码
- 输入 `AsKh`、`KA`、`AKs` 等不同格式都会被规范化为统一的 169 手牌码

**第 3 步：动作 Schema 编码（9 字节/条）**
- 每条动作定义序列化：1 字节类型 + 4 字节 size(f32) + 4 字节 amount_bb(f32)
- 6 种动作类型：fold, call, check, bet, raise, allin
- 去重后存储在 `meta.db` 中，通过 `action_schema_id` 引用，避免重复存储

**第 4 步：Pack 打包（核心编码）**
- 每个 concrete_line 的数据打包为一个 "pack"
- 布局公式：`hand_count * (5 + action_count * 8)` 字节
  - 1 字节 hand_id (手牌序号)
  - 4 字节 action_mask (位掩码，表示哪些动作存在)
  - 8 字节 × action_count (frequency f32 + hand_ev f32)
- 例如：169 手牌 × 32 动作 = 169 × 261 = 44,109 字节

**第 5 步：写入文件**
- `.bin` 文件：16 字节 PFSP 头 + 所有 pack 顺序拼接
- `.idx` 文件：16 字节 PFXI 头 + 每条记录 22 字节（包含 pack 在 .bin 中的偏移、长度、CRC32C 校验码）

#### 2.1.3 解码查询（零拷贝 mmap）

查询时底层使用了 **mmap（内存映射）** 技术：
1. `.idx` 和 `.bin` 文件被整个映射到进程地址空间，无需 `read()` 系统调用
2. **O(1) 索引查找**：因为 idx 记录是连续的，`record_index = concrete_line_id - first_concrete_line_id`，直接数组下标访问
3. **零拷贝读包**：从 mmap 区域直接取 `&[u8]` 切片，无中间缓冲区
4. **二进制搜索手牌**：在 pack 的前 `hand_count` 字节中二分查找目标手牌
5. **位掩码过滤**：通过 action_mask 快速判断哪些动作存在，只解码有效数据

---

### 2. Benchmark 体系

#### 2.2.1 Hot Benchmark（热查询）

**场景**：服务已经打开，维度已经预加载到内存中

**测量流程**：
1. 从 SQLite 源数据中按种子随机采样 workload（保证可复现）
2. 打开 `StoreQueryService`，预加载 workload 涉及的所有维度
3. 对以下 5 类查询执行多次测量，记录延迟和 QPS：
   - `hand-strategy`：单手牌查询（指定 concrete_line_id + hole_cards）
   - `batch-hand-strategy`：批量查询（默认 batch_size）
   - `batch-size-{N}`：不同批次大小（1/5/10/50/100）的性能对比
   - `hands-by-actions`：按动作条件筛选手牌（需要解码整个 pack）
   - `drill-scenarios-metadata`：查询可用的抽象线路

**指标**：p50 / p90 / p95 / p99 延迟 + QPS

#### 2.2.2 Cold Benchmark（冷启动）

**场景**：全新进程启动，OS page cache 中没有数据

**测量流程**：
1. 根据模式清除 OS 缓存（Linux 下 `drop_caches`，Windows 下写填充文件）
2. 启动子进程测量 4 个阶段的时间：
   - `service_open`：打开 StoreQueryService（加载 manifest + meta.db）
   - `dimension_prewarm`：mmap 某个维度的 .idx/.bin 文件
   - `first_query`：执行第一次查询（包含 pack 解码）
   - `close`：关闭服务
3. 同时记录内存变化（RSS 增量）

**为什么用子进程**：确保每次测量都是全新的进程和文件描述符状态，不受之前查询的缓存影响。

#### 2.2.3 Workload 生成与采样

**两种模式**：
- `random`：完全随机选择 concrete_line_id 和手牌
- `abstract-local`：基于抽象线路局部采样，更符合真实使用场景

**采样方法**：
- 按维度行数加权随机选择维度
- 使用确定性种子（`SeededRandom`），保证每次生成的 workload 一致
- 可序列化到 JSON 文件，供多次 benchmark 复用

#### 2.2.4 性能提升的本质原因

| 技术 | 效果 |
|------|------|
| mmap 零拷贝 | 省去 read() 系统调用和内核态/用户态数据拷贝 |
| 密集索引 O(1) 查找 | 不用 B-tree 遍历，直接数组下标定位 |
| 固定大小 cell (8 字节) | 算术偏移直接计算，不用解析变长字段 |
| 位掩码过滤 | hands-by-actions 用位运算替代 SQL 的 OR 条件 |
| Schema 内存缓存 | action_schemas 加载为 HashMap，O(1) 查找替代 SQL JOIN |
| LRU Handle Pool | 维度文件保持打开，避免反复 mmap/unmap |

---

### 3. 验证体系

#### 3.1 Standalone Verification（独立验证）

**对比基准**：`manifest.json` 自身的一致性

验证层级：

| 层级 | 验证内容 | 失败原因示例 |
|------|---------|-------------|
| manifest | 格式、版本、维度声明 | INVALID_JSON, MISSING_FILE |
| meta.db | build_info 表、action_schemas CRC32C、维度表存在性 | SCHEMA_KEY_MISMATCH |
| .idx 文件 | 魔数、版本、记录连续性、hand_count 范围 | NON_DENSE_INDEX, HAND_COUNT_EXCEEDED |
| .bin 文件 | 魔数 PFSP、版本、端序、浮点类型 | INVALID_MAGIC |
| 索引-包交叉 | offset 合法、byte_length 匹配、CRC32C、hand_id 排序 | CHECKSUM_MISMATCH |

#### 3.2 Cross Verification（交叉验证）

**对比基准**：源 SQLite 数据库 vs 二进制文件

**采样机制**：
- 默认采样 10,000 条（可按维度大小比例分配）
- `sample_size = 0` 时全量比对（本次覆盖了 9 个维度、23,806,716 条记录）
- 使用确定性哈希排序实现可复现采样

**逐单元格对比**：
1. 从 SQLite 读取一行数据
2. 通过 .idx 找到对应的 pack，从 .bin 中解码
3. 对比：action_name、action_size、amount_bb、frequency、hand_ev
4. **Float32 精确匹配策略**：`f64 -> f32 -> f64` 截断后 bit-exact 比较，不是近似容差

**额外检测**：
- `extra_binary_records`：二进制中有但 SQLite 中没有的记录（反向不一致）
- Float32 量化误差统计（p95/p99 quantization error）

---

### 4. 断点续跑（Resume Build）

#### 4.1 机制设计

通过 `build-state.json` 文件记录构建进度：

```jsonc
{
  "version": 1,
  "source_checksum": "...",       // 源数据库校验和
  "max_concrete_lines": 10000,
  "dimensions": [
    {
      "key": "default:6:100",
      "status": "completed",       // pending | in_progress | completed | failed
      "idx_file": "...",
      "bin_file": "...",
      "concrete_line_count": 345,
      "pack_count": 345
    }
  ]
}
```

#### 4.2 Resume 流程

1. **检测状态文件**：如果 `--resume` 参数开启且 `build-state.json` 存在，加载已有状态
2. **校验一致性**：
   - 源数据库 checksum 必须匹配（防止用错数据源重建）
   - 维度列表必须匹配
   - `--max-concrete-lines` 必须一致
3. **跳过已完成维度**：`status == "completed"` 的维度直接跳过
4. **处理中断维度**：`status == "in_progress"` 的维度重新构建
5. **文件完整性**：对已完成的维度校验 .idx/.bin 文件大小和 checksum

#### 4.3 安全机制

- `--resume` 和 `--overwrite` 互斥，防止误操作
- 临时文件使用 `.tmp` 后缀，构建成功后原子 rename
- 维度级别的失败不影响其他维度

---

### 5. Native SDK 使用指南

#### 5.1 架构

```
Bun/Node.js 应用
    |
    v
index.js (JS 包装层)
    |
    v
index.node (napi-rs 原生绑定，链接 range-store-core)
    |
    v
.msi / .bin / .idx / meta.db (数据文件)
```

#### 5.2 安装与构建

```powershell
cd range-store-native
bun install
bun run build:native    # 编译 Rust 代码生成 index.node
```

#### 5.3 基本用法

```javascript
import { PokerHandsRange } from "./range-store-native/index.js";

// 1. 初始化（加载数据目录）
const store = new PokerHandsRange({
  dataDir: "./data/range-strata",
  maxOpenHandles: 2,       // 最大同时打开的维度 reader 数
  verifyChecksums: false,  // 是否启用 CRC32C 校验
});

// 2. 单手牌查询
const result = store.queryHandStrategy({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLineId: 42,
  holeCards: "AKs",
});
// result = { code: 0, data: { handCode: "AKs", actions: [...] }, message: null }

// 3. 批量查询
const batchResult = store.queryBatch({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  items: [
    { concreteLineId: 1, holeCards: "AA" },
    { concreteLineId: 2, holeCards: "KK" },
  ],
});

// 4. 按动作筛选手牌
const handsResult = store.handsByActions({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLineId: 42,
  actions: ["call", "fold"],
  frequency: 0.005,
});

// 5. 预加载维度
store.prewarm({ strategy: "default", playerCount: 6, depthBb: 100 });

// 6. 查看统计信息
const stats = store.stats();
// { schemaCount: 15, openHandleCount: 3, knownDimensions: [...] }
```

#### 5.4 错误处理

所有 API 返回统一信封格式：
```typescript
{ code: number, data: T | null, message: string | null }
```
- `code === 0`：成功
- `code !== 0`：失败，`message` 包含错误信息

#### 5.5 Singleton 模式

对于需要复用的场景，可以使用 singleton：
```javascript
import { getPokerHandsRangeSingleton } from "./range-store-native/index.js";

const store = getPokerHandsRangeSingleton({ dataDir: "./data" });
// 后续相同选项的调用会返回同一个实例
```

---

## 三、项目成果总结

### 3.1 技术指标

| 指标 | 结果 |
|------|------|
| 磁盘节省 | 76% (1,447 MB -> 346 MB) |
| 单手查询 QPS 提升 | 6.4x |
| 批量查询 QPS 提升 (size 20) | 36.2x |
| hands-by-actions QPS 提升 | 9.45x |
| 数据完整性验证 | 23.8M 记录，0 失败 |
| 单元测试 | 100+ 测试全部通过 |

### 3.2 项目结构（4 个 crate）

| Crate | 职责 | 代码规模 |
|-------|------|---------|
| `range-store-core` | 二进制存储引擎（mmap、编解码、查询） | 核心 |
| `service` | HTTP API 服务（axum + OpenAPI） | 8 个端点 |
| `range-store-native` | Bun/Node.js 进程内 SDK（napi-rs） | 完整 API 映射 |
| `storage-tools` | 离线工具（构建、验证、benchmark） | 全链路 |

### 3.3 v1.1.0 新增内容

- CachedMetadataReader：惰性加载元数据索引，消除批量查询中的重复 SQLite 查找
- Native benchmark runner：新增 metadata drill 和 hands_by_actions 单链路 benchmark
- P90 延迟指标：补充尾部延迟可视化
- 查询批错误传播：单个失败不影响整个 batch 的其余项
- Service directory refactor：模块职责更清晰
