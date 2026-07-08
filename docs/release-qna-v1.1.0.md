# poker-hands-storage v1.1.0 汇报 Q&A

## 使用说明

这些问题不是入门科普，而是针对系统设计中的取舍和陷阱。回答时直接切入技术决策的 trade-off。

---

## 一、存储引擎设计

### Q: dense index 要求 concrete_line_id 连续，但如果某个 concrete_line 在源数据中缺失（比如某些手牌组合在某些 line 中不存在），你们怎么处理？

**回答**：dense index 的"连续"指的是 concrete_line_id 本身的序列连续，不是手牌连续。每个 concrete_line 对应一个独立的 pack，pack 内部的 hand_ids 才是稀疏的——用 action_mask 位掩码标记哪些手牌-动作组合存在。

具体来说：
- `.idx` 记录要求 `concrete_line_id` 从 `first_concrete_line_id` 开始严格递增、无跳跃。验证时在 open 阶段调用 `validate_dense_index_layout()`，如果发现 gap 会标记为 `NON_DENSE_CONCRETE_LINE_ID`。
- 但 pack 内部的 169 种手牌是稀疏存储的。实际出现在源数据中的手牌可能只有 100 种，pack 里只存这 100 个 hand_id，通过 `hand_count` 字段记录实际数量。
- 查询时先通过 dense index O(1) 定位到 concrete_line_id 对应的 pack，然后在 pack 内部对 hand_ids 做 binary search（`binary_search_u8`）。

所以 dense index 解决的是 concrete_line 层面的定位，pack 内部的手牌仍然是稀疏的。两层设计：第一层 O(1) 找到 pack，第二层 O(log n) 找到手牌。

### Q: pack 格式 `hand_count * (5 + action_count * 8)` 里，action_count 是从哪里来的？不同 concrete_line 的 action_count 不同怎么办？

**回答**：有两个口径，但它们在正确产物中必须一致。

- `meta.db.action_schemas.action_count` 是 schema 层的权威语义值，表示这个 `action_schema_id` 下有多少个 action definition。
- `.idx` record 里的 `byte_length` 和 `hand_count` 可以反推出该 pack 物理布局中的 `action_count`。运行时热路径用这个反推值来切分 pack：

```rust
action_count = (byte_length / hand_count - 5) / 8
```

`.idx` record 同时保存 `action_schema_id`，所以同一条 concrete line 的完整解释链路是：

```text
concrete_line_id -> idx record(action_schema_id, hand_count, byte_length, offset)
                 -> 由 hand_count + byte_length 反推 pack action_count
                 -> 通过 action_schema_id 查 action_schemas.action_count/action_blob
```

这两个 `action_count` 不应该各说各话：standalone verify 会用 `action_schemas.action_count` 校验 `.idx.byte_length` 是否等于 `hand_count * (5 + action_count * 8)`。如果不一致，说明构建产物损坏或构建逻辑有 bug，应该 fail fast，而不是在运行时容忍。

同一个 dimension 的不同 concrete_line 可以有不同数量的 actions。比如一个 line 只有 fold+call 两个动作（action_count=2），另一个 line 可能有 15 个动作。每个 concrete line 指向自己的 `action_schema_id`，也有自己的 `byte_length`，所以 pack 可以是变长的。

不同 action_count 的 pack 直接顺序拼接在 .bin 文件中，通过 idx record 中的 `offset + byte_length` 分隔。解码时先读 idx record 拿到 pack 边界和物理布局，再用 `action_schema_id` 找 schema，把 action_id 翻译成业务 action name、size 和 amount。

这也意味着 pack 之间不是定长的——这是有意的设计取舍。定长 pack 可以实现更简单的 offset 计算，但会为 action_count 小的 concrete_line 浪费大量空间。我们的数据中 action_count 差异很大，所以选择了变长，并用 verify 保证 schema 语义值和 pack 物理长度一致。

### Q: ActionSchemaCache 用了 Mutex + RwLock 的双重锁结构，为什么不直接用 HashMap？

**回答**：因为 action schema 的数据来源是 meta.db 这个 SQLite 文件，每次 cache miss 都需要做一次数据库查询。

双重锁的设计意图是分离两个不同的资源：
- `RwLock<ActionSchemaCacheState>`：保护内存中的 HashMap<u32, Arc<Vec<ActionDef>>>，读多写少，所以用 RwLock 允许多个并发查询共享 cache hit
- `Mutex<LockedActionSchemaConnection>`：保护 SQLite 连接。我们的 Connection 用了 `SQLITE_OPEN_NOMUTEX`，本身不是 Send/Sync 的，需要用 Mutex 串行化所有数据库读写

关键路径是 cache hit——走 RwLock read path，不需要碰 Mutex。只有 cache miss 时才获取 Mutex 查 SQLite，然后把结果 `Arc::clone` 插入 HashMap。Arc clone 是 O(1) 的指针操作，不持有写锁太久。

如果全部预加载到 HashMap 里，初始化时会阻塞所有线程查 SQLite。懒加载的好处是如果某个 query 只涉及 3 个 action_schema_id，就只查 3 次 SQLite，其余直接走内存。v1.1.0 引入的 CachedMetadataReader 也是同样的懒加载思路，只是把粒度从 action_schema 扩展到了 concrete_line 和 drill scenario 查询。

### Q: mmap 在 Windows 上的表现怎么样？和 Linux 比有什么坑？

**回答**：Windows 上用 `memmap2::Mmap::map()` 底层调的是 Windows API `CreateFileMapping` + `MapViewOfFile`。主要差异：

1. **页预取行为不同**：Linux 的 read-ahead 对 sequential access 友好，但我们 dense index 的查找是 random access（concrete_line_id 随机查询），Linux 上 OS 也能较好地处理 random mmap fault。Windows 上同样如此，没有特别的 read-ahead 优化，但实际 benchmark 没有观察到显著差异。

2. **文件删除/替换**：Windows 上如果文件被替换（我们的 versioned deploy 策略），已有的 mmap 仍然可以正常读取旧数据（文件句柄还在），但新进程 mmap 新文件时会读到新版本。这在 Linux 上是语义一致的（unlink + rename），所以没有平台差异问题。

3. **内存提交限制**：Windows 有 commit charge 限制，mmap 大文件会占用虚拟地址空间 + 物理提交。我们的 .bin 文件最大 ~350MB，加上 .idx 几乎可以忽略，所以不是问题。但如果未来扩展到 10GB+ 级别，需要考虑 sparse mmap 或分段文件。

---

## 二、Benchmark 深度问题

### Q: Cold benchmark 的 `store_open_and_first_query_ms` 把 service_open + prewarm + first_query 合并成一个指标，为什么不分开看？

**回答**：分开看确实有意义，所以我们同时在 `ColdWorkerTimings` 里记录了四个独立 phase 的时间：

```
service_open_ms      — manifest 解析 + meta.db 打开 + HandlePool 初始化
dimension_prewarm_ms — 某个维度的 .idx/.bin mmap 建立
first_query_ms       — 一次完整的 hand-strategy 查询（idx lookup + pack read + decode + schema resolve）
close_ms             — drop service，释放 mmap
```

`store_open_and_first_query_ms` 是前三个的和，代表用户从启动到拿到结果的端到端延迟。分开看用于定位瓶颈（比如 prewarm 占了 80% 说明文件大），合并看用于 SLA 评估（用户关心的是 total time to first result）。

另外 cold benchmark 有 `phase_accounting` 检查：`phase_sum = service_open + prewarm + first_query + close`，`unaccounted_ms = worker_total - phase_sum`。如果 unaccounted > 1ms 或 > 1%，说明测量有误差（比如子进程 spawn 开销被错误计入），需要排查。

### Q: Cold benchmark 里 service_open 用了 `max_open_handles=2`，这个值是怎么定的？

**回答**：这是刻意设置的保守值，目的是测量最小启动开销。`max_open_handles` 控制 HandlePool 的容量——默认 HTTP service 用 2（`PHS_MAX_OPEN_HANDLES` 环境变量），cold benchmark 也用了 2 以保持一致。

但 cold benchmark 的关键是它只 prewarm **一个**维度，所以实际上 Pool 只需要容纳 1 个 DimensionReader。设为 2 是因为：
1. 和 production 默认值一致，避免 benchmark 环境和实际部署差异过大
2. Pool 容量不影响 mmap 的行为，只影响内存分配。2 个 handle 的内存开销可以忽略

如果把 max_open_handles 设得很大（比如 100），open 阶段的内存分配会更多，service_open_ms 会变大，但这不代表真实场景——真实场景中不会一次性 prewarm 所有维度。

### Q: Hot benchmark 的 batch_size 为什么选 [1, 5, 10, 50, 100] 这几个值？

**回答**：覆盖三个使用场景区间：

- **1/5**：单手牌查询的对比基线。batch_size=1 应该和 hand-strategy 查询性能一致（同一代码路径）
- **10/50**：典型前端交互场景。比如 UI 上一个表格同时展示 10-50 只手牌的选择，一次发 batch 请求
- **100**：压力测试上限。HTTP API 的 batch endpoint 限制 max 500，100 是中等偏大的批量

实际 benchmark 中 batch 性能提升不是线性的——batch_size=100 相比 batch_size=1 的 QPS 提升远大于 100 倍，原因是：
1. 一次 `pool.get_or_open()` 打开 dimension handle，100 次查询复用同一个 handle
2. pack 解码时，同一个 concrete_line 的多个 hand 可以共享同一个 pack 读取（`query_many_hands` 路径）
3. 网络开销（HTTP 场景）被摊薄

### Q: Native benchmark 的 Core/HttpService/Sdk 三种模式，为什么 entry order 要用 seed 打乱？

**回答**：防止 OS page cache 带来的偏差。

假设三个模式的测试顺序固定是 Core -> Http -> Sdk：
1. Core 先跑，数据被加载到 OS page cache
2. Http 跑的时候，同样的数据已经在 cache 里了，cold 成分被污染
3. Sdk 同理

虽然 native benchmark 测的是 hot path（服务已运行），但同一轮测试中不同模式的 entry order 如果固定，会导致后续模式的 mmap 文件已经部分被 OS prefetch。用 seed 做 MurmurHash 风格的随机排序，保证每次运行的测试顺序不同，长期平均下来抵消这个偏差。

---

## 三、验证体系深度问题

### Q: Float32 bit-exact 验证中，如果源 SQLite 的 frequency 本身就是 f64 精度（比如 0.123456789），截断到 f32 后损失了多少？你们有没有统计过这个精度损失的分布？

**回答**：有统计。`Float32PrecisionStatsAccumulator` 收集了以下指标：

- `quantization_abs_error`：`|source_f64 - (source_f64 as f32 as f64)|`，即截断的绝对误差
- `quantization_relative_error`：`quantization_abs_error / max(|source|, 1.0)`
- `implementation_abs_error`：`|stored_f32_as_f64 - ideal_truncated_f64|`，即实际存储值和理论截断值的差距（理想情况为 0）

同时用 reservoir sampling（size 8192）跟踪 quantization error 的 p95/p99 分布，并记录 top-20 最大误差样本。

f32 的尾数只有 23 位，有效数字约 7 位十进制。所以 `0.123456789` 截断到 f32 大约是 `0.12345679`（最后一两位丢失），绝对误差在 1e-7 量级。对 poker strategy 来说，frequency 是概率值（0-1 之间），hand_ev 是期望值（通常 0-5 BB），这个精度损失完全可以接受。

关键在于：我们用 bit-exact 而不是 tolerance，是为了区分"精度损失"和"编码错误"。如果允许 1e-6 的 tolerance，那么 SQLite pager 读取错误、pack 解码偏移错误、action_schema 查找错误都可能被 tolerance 掩盖。bit-exact 确保任何非精度损失的差异都能被捕获。

### Q: Cross verify 的采样 SQL 用 `(concrete_line_id * 1103515245 + id * 12345) & 0x7FFFFFFF` 做伪随机排序，这个哈希函数是怎么选的？

**回答**：这是一个线性同余生成器（LCG）的变种，选这些常数不是因为有什么特殊的数学性质，而是因为：

1. **1103515245**：glibc `rand()` 的 multiplier，广泛已知有合理的分布特性
2. **12345**：简单的 additive constant，和 multiplier 没有公因子
3. **& 0x7FFFFFFF**：去掉符号位，保证结果为正数（SQLite 的 ORDER BY 对负数排序行为一致，但我们只需要排序，不需要数值意义）

这个哈希的目的不是加密或统计学意义上的均匀分布，而是：
- 打破 id 的自然顺序，避免采样集中在某个范围
- 确定性（相同数据永远产生相同采样）
- 计算便宜（一次乘法、一次加法、一次位运算，SQLite 原生支持）

如果需要更好的分布，可以用 `random()` 函数，但那是非确定性的。也可以用更复杂的 hash，但在 2380 万行的源数据上做 ORDER BY，hash 计算的开销占比很小（SQLite 的排序瓶颈在 I/O，不在比较函数）。

### Q: Standalone verification 中 idx record 要求 dense（无 gap），但如果源 SQLite 中 concrete_line_id 本身有 gap 呢？

**回答**：这是个好问题。在 build 阶段，concrete_line_id 是从 SQLite 的 `id` 字段分配的，从 1 开始自增。`storage-tools` 的 build orchestrator 在读取源数据时：

```sql
SELECT concrete_line_id, hole_cards, action_name, ... FROM range_data_* ORDER BY concrete_line_id, hole_cards, action_name
```

concrete_line_id 是源表的 `id` 列，是自增主键，不会有 gap。如果源数据本身有 gap（比如手动删除了某些行），那说明源数据就不干净，应该在 build 之前就修复。

我们的验证策略是：standalone 验证假设 build 过程是正确的，验证的是 build 产出的文件一致性。如果源数据有问题，cross verify 会发现（因为 SQLite 的 concrete_line_id 序列和 idx 的不匹配会导致某些记录找不到对应的 pack）。

---

## 四、Resume Build 深度问题

### Q: build-state.json 的 source_checksum 是怎么计算的？如果源 SQLite 做了 ALTER TABLE 但数据没变，checksum 会变吗？

**回答**：source_checksum 是对源 SQLite 文件的整个文件内容做 checksum（不是对表数据），所以：

- 如果源文件内容完全不变（同一个文件），checksum 不变，resume 可以通过
- 如果对源文件做了 ALTER TABLE（即使数据不变），SQLite 会生成新的文件（page 布局可能变化），checksum 会变，resume 会拒绝
- 如果是重新 export 了一份相同数据的 SQLite 文件，但文件字节不同，checksum 也会变

这个设计的意图是：source_checksum 检测的是"源文件是否被替换过"，而不是"数据是否一致"。更精细的数据校验靠的是 build 过程中计算的每 dimension 的 pack_count 和 concrete_line_count，resume 时会对比这些数字。

如果要强制重新构建，用 `--overwrite` 参数丢弃 build-state.json 从头来。

### Q: Resume 时如果某个维度 build 中途崩溃，.idx.tmp 和 .bin.tmp 文件会残留吗？

**回答**：会残留。build 过程中每个维度的输出先写 `.tmp` 文件，只有整个维度构建成功后才 rename 为正式文件。如果中途进程被 kill：

- `build-state.json` 中该维度的 status 会被设为 `"failed"`（panic handler 会写状态）
- `.tmp` 文件会残留，但下次 resume 时因为 status 是 failed，会重新构建，rename 时 `--overwrite` 行为会覆盖 tmp 文件
- 如果手动删除了 tmp 文件也不影响 resume，重新构建时自然会创建新的

安全机制：`--resume` 和 `--overwrite` 互斥，但 resume 过程中对 failed 维度的重建本质上等同于局部 overwrite，这是允许的。

---

## 五、Native SDK 深度问题

### Q: SDK 的 queryBatch 中单个 item 失败（比如 hand 不存在），为什么不影响同 batch 的其他 items？

**回答**：代码路径是这样的：

```rust
// StoreQueryService::query_batch
requests.iter().map(|(concrete_line_id, hole_cards)| {
    match self.query_single(&reader, dimension, *concrete_line_id, hole_cards) {
        Ok(result) => BatchItemResult { actions: Some(...), error: None },
        Err(error) => BatchItemResult { actions: None, error: Some(...) },
    }
})
```

每个 item 独立 `match`，`query_single` 返回 `StoreQueryError`（可能是 `NotFound`、`HandParse` 等），被捕获后转为该 item 的 `error` 字段，继续处理下一个 item。

这是因为 batch 的使用场景是前端批量查询——用户在一个表格里看了 50 只手牌，其中可能有一两只不在当前 dimension 的 concrete_line 覆盖范围内（比如某些极端 hand 组合在某些 line 中不存在）。如果因为一个 hand 不存在就返回整个 batch 错误，用户体验很差。

HTTP service 的 error code 映射也配合了这个设计：
- `NotFound` -> HTTP 404
- `HandParse`（非法手牌格式）-> HTTP 1000
- 其他内部错误 -> HTTP 500

每个 item 的 error 对象里包含 `code` 和 `message`，前端可以根据 code 做差异化处理。

### Q: CachedMetadataReader 和原来的 MetadataReader 有什么区别？为什么叫 Cached？

**回答**：`MetadataReader`（旧版）每次查询都打开一个新的 SQLite 连接执行查询。比如查询 concrete_line "F-F-F-R2"，会：
1. `Connection::open(meta.db)`
2. 拼 SQL：`SELECT ... FROM concrete_lines_default_6max_100BB WHERE concrete_line = ?`
3. 执行、取结果、关连接

如果 batch 查询中有 100 个 hand 都来自同一个 concrete_line，就会打开/close meta.db 100 次。

`CachedMetadataReader`（v1.1.0 引入）的"cached"指的是**查询结果的内存缓存**，不是 SQLite 的连接缓存：

1. 构造时只读 manifest，不查 SQLite
2. 第一次 `get_concrete_lines("default", 6, 100, None, Some("F-F-F-R2"))` 时：
   - 查 RwLock<HashMap>，miss
   - 打开 SQLite 连接查询，结果存入 `concrete_by_concrete` HashMap
3. 第二次同样的查询：
   - 查 RwLock<HashMap>，hit，直接返回 `Arc::clone`

缓存的 key 是 `(strategy, player_count, depth_bb, concrete_line)` 组成的结构体，实现 `Hash + Eq`。缓存的是 `ConcreteLineRow` 的可克隆值（不是 Arc），因为结构很小（3 个字段）。

读锁用 RwLock 而不是 Mutex，因为缓存查询是读多写少——多个并发查询可以同时读 cache，只有 cache miss 时才需要写锁。

trade-off：缓存会占用内存，但每个 concrete_line 查询结果只有 ~100 字节，即使缓存 10000 个结果也只占 ~1MB。对于 batch 查询场景，这个内存开销换来的是 SQLite 查询次数从 N 次降到 1 次。

---

## 六、架构取舍

### Q: HandlePool 默认 max_open_handles=2，但一个 dimension 对应一对 .idx + .bin 文件。2 个 handle 是不是太小了？

**回答**：是的，2 确实小，但是有意为之。

原因：
1. **mmap 不需要保持 file descriptor 打开**：Linux 上 `mmap` 后即使 close fd，数据仍在 OS page cache 中。Windows 上 `MapViewOfFile` 同理。所以 handle 数量不影响已 mmap 数据的可用性，只影响"是否需要重新 mmap"。
2. **默认场景下用户只查少数几个 dimension**：如果用户只查 `default:6:100` 这一个 dimension，handle=2 绰绰有余。
3. **LRU 淘汰的成本很低**：evict 一个 handle 只是 drop Arc<DimensionReader>，析构时 close mmap。下次再查同一个 dimension 时重新 mmap，成本主要在 OS page fault，不是文件 I/O。

`PHS_MAX_OPEN_HANDLES` 环境变量可以让生产环境调大。我们的 benchmark 中 hot benchmark 用的是 100（"预加载所有需要的维度"），cold benchmark 用的是 2（"最小启动开销"）。

如果用户的查询模式覆盖所有 9 个维度，建议设 >= 9。设太小只会增加反复 mmap 的开销，不会导致数据错误。

### Q: 为什么 meta.db 继续用 SQLite，而策略数据用自定义二进制？meta.db 不会也成为瓶颈吗？

**回答**：meta.db 的体量和访问模式完全不同：

- **体量**：meta.db 只有几十 KB 到几 MB（action_schemas 表最多几百条，concrete_lines 表每个维度几千条，每条就三个字段）。而策略数据 .bin 文件有 345 MB。
- **访问模式**：meta.db 的查询是 point query（WHERE concrete_line = ?），命中索引后一次 B-tree 跳转就能拿到结果。策略数据是 sequential scan + random access（需要解码整个 pack）。
- **缓存命中率**：meta.db 太小，整个文件可以常驻内存。我们的 CachedMetadataReader 更是把查询结果缓存在进程内存里，实际运行时 meta.db 的 SQLite 查询次数趋近于 0。

瓶颈永远在 pack 解码（decode_pack_for_hand 是 hot path），不在 metadata lookup。而且 action_schema 查一次缓存一次，concrete_line 查一次也缓存一次。一个 batch 100 个 hand 如果来自同一个 dimension，meta.db 只被查 1-2 次（一次 schema + 一次 concrete_line）。
