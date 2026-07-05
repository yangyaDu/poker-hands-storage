# 查询链路详解：从 SDK 调用到结果返回

本文档追踪一次完整查询从入口到数据返回的全链路，说明每一步做了什么、为什么比 SQLite 快。

---

## 场景：通过 Native SDK 查询一手牌的战略

```javascript
import { PokerHandsRange } from "./range-store-native/index.js";
const store = new PokerHandsRange({ dataDir: "./data/range-strata" });
const result = store.queryHandStrategy({
  strategy: "default",
  playerCount: 6,
  depthBb: 100,
  concreteLineId: 42,
  holeCards: "AKs",
});
```

---

## 链路总览

```
JS (index.js)
  |
  v
N-API Binding (range-store-native/src/lib.rs)
  |
  v
RangeStoreFacade (range-store-core/src/query/range_store_facade.rs)
  |
  +-- get_concrete_lines --> CachedMetadataReader --> meta.db (SQLite, 缓存)
  |
  +-- query_hand_strategy --> StoreQueryService
       |
       v
  HandlePool.get_or_open()
       |
       v
  DimensionReader.query()
       |
       +-- IdxReader.find(concrete_line_id)  --> .idx mmap  --> O(1) 定位 pack
       |
       +-- BinReader.read_pack(offset, length) --> .bin mmap  --> 零拷贝取数据
       |
       +-- decode_pack_for_hand(pack, hand_id) --> pack_codec  --> 位运算解码
       |
       +-- ActionSchemaCache.get(action_schema_id) --> HashMap 缓存
       |
       v
  ActionResult[] 返回给 JS
```

---

## 第一步：SDK 构造函数（初始化阶段）

### 入口：`new PokerHandsRange({ dataDir })`

**JS 层** (`range-store-native/index.js:52-58`)：
```javascript
this.#native = new native.PokerHandsRange({
  dataDir: options.dataDir,
  maxOpenHandles: options.maxOpenHandles,
  verifyChecksums: options.verifyChecksums,
});
```

**N-API 绑定层** (`range-store-native/src/lib.rs:141-149`)：
```rust
#[napi(constructor)]
pub fn new(options: PokerHandsRangeOptions) -> Result<Self> {
    let inner = RangeStoreFacade::open(data_dir, max_open_handles, verify_checksums)?;
    Ok(Self { inner: Arc::new(inner) })
}
```

**RangeStoreFacade 打开两个组件** (`range_store_facade.rs:48-61`)：

1. **CachedMetadataReader**：读取 `manifest.json` 发现所有维度，打开 `meta.db`（SQLite）但不立即查询任何数据
2. **StoreQueryService**：打开 `meta.db` 的 action_schemas 查询通道，创建 HandlePool（LRU 缓存，默认容量 2）

这一阶段的耗时计入 cold benchmark 的 `service_open_ms`。

### 初始化做了什么，没做什么

| 做了 | 没做 |
|------|------|
| 解析 `manifest.json`（1KB JSON） | 没有 mmap `.idx` / `.bin` 文件 |
| 打开 `meta.db`（SQLite read-only） | 没有加载任何 action schema 到内存 |
| 创建空的 HandlePool | 没有读取任何策略数据 |

---

## 第二步：queryHandStrategy 调用

### N-API 绑定 (`range-store-native/src/lib.rs:206-225`)

```rust
#[napi]
pub fn query_hand_strategy(&self, request: QueryHandStrategyRequest) -> Result<...> {
    let dimension = DimensionRef::new("default", 6, 100);
    let result = self.inner.query_hand_strategy(&dimension, 42, "AKs")?;
    // 将 ActionResult 从 Rust f32/f64 转到 JS number
    Ok(QueryHandStrategyResponse { ... })
}
```

这里 `self.inner` 是 `Arc<RangeStoreFacade>`，clone 是 O(1) 指针操作。

### RangeStoreFacade 转发 (`range_store_facade.rs:102-111`)

```rust
pub fn query_hand_strategy(&self, dimension, concrete_line_id, hole_cards) -> Result<QueryResult> {
    Ok(self.query_service.query(dimension, concrete_line_id, hole_cards)?)
}
```

纯转发，无额外开销。

---

## 第三步：StoreQueryService.query() —— 核心查询路径

**文件**：`range-store-core/src/query/store_query_service.rs:164-207`

### 3.1 手牌解析

```rust
let parsed = parse_hole_cards("AKs")?;
// parsed.hand_id = 13  (AKs 在 169 字典中的索引)
// parsed.hand_code = "AKs"
```

`parse_hole_cards` 处理多种输入格式：`AKs`、`AsKh`、`KA`、`akS` 等，统一规范化为 169 手牌码。

### 3.2 获取 DimensionReader（HandlePool）

```rust
let reader = self.pool.get_or_open(dimension)?;
```

**HandlePool 逻辑** (`handle_pool.rs`)：

1. 查 HashMap，key = `"default:6:100"`
2. **Cache hit**：返回已有的 `Arc<DimensionReader>`，更新 LRU 位置
3. **Cache miss**：
   - 创建 `DimensionReader::open(.idx, .bin)`
   - `memmap2::Mmap::map(.idx)` — 内存映射索引文件
   - `memmap2::Mmap::map(.bin)` — 内存映射数据文件
   - 验证 `.idx` 的 dense layout（concrete_line_id 连续）
   - 验证 `.bin` 的 PFSP header（魔数、版本、端序）
   - 插入 HashMap，如果超过 `max_open_handles` 则淘汰 LRU

**关键**：`DimensionReader` 持有一个 `_file: File` 字段，保证 mmap 不会被 OS 回收。mmap 的生命周期和 `DimensionReader` 绑定。

### 3.3 DimensionReader.query()

**文件**：`range-store-core/src/dimension_reader.rs:42-65`

```rust
pub fn query(&self, concrete_line_id: u32, hand_id: u8, verify_checksum: bool) -> Option<PackDecodeResult> {
    // Step A: 通过 .idx 找到 pack 的位置
    let record: IdxRecord = match self.idx.find(concrete_line_id) { ... }

    // Step B: 从 .bin 中读取 pack 原始字节（零拷贝）
    let pack = self.read_and_validate_pack(concrete_line_id, &record, verify_checksum)?;

    // Step C: 从 byte_length 反推 action_count
    let action_count = action_count_from_pack(record.hand_count, record.byte_length);

    // Step D: 解码 pack，提取目标 hand_id 的 cells
    let cells = decode_pack_for_hand(pack, record.hand_count, action_count, hand_id);

    Ok(Some(PackDecodeResult { action_schema_id: record.action_schema_id, cells }))
}
```

---

## 第四步：IdxReader.find() —— O(1) 密集索引查找

**文件**：`range-store-core/src/idx_reader.rs:149-172`

```rust
fn find_by_dense_index(&self, concrete_line_id: u32) -> Option<IdxRecord> {
    let first_id = self.first_concrete_line_id?;       // 例如 1
    let index = concrete_line_id.checked_sub(first_id)?; // 42 - 1 = 41
    if index >= self.record_count { return None; }       // 边界检查

    // 直接计算 mmap 中的字节偏移
    let records_base = &self.mmap[IDX_HEADER_SIZE..];    // 跳过 16 字节 header
    let offset = index as usize * IDX_RECORD_SIZE;       // 41 * 22 = 902
    let record = decode_idx_record_at(records_base, offset);

    // 二次验证：防止 first_id 为 None 时的边界情况
    if record.concrete_line_id == concrete_line_id {
        Some(record)
    } else {
        None
    }
}
```

**对比 SQLite**：
- SQLite：`SELECT ... WHERE concrete_line_id = 42` → SQL 解析 → 准备语句 → B-tree 索引查找 → 页读取 → 行解析
- 我们的二进制：`index = 42 - 1 = 41; offset = 41 * 22 = 902; memcpy(&record, mmap + 902, 22)`

**省掉的开销**：SQL 解析（正则表达式匹配）、B-tree 节点遍历（可能 3-4 层）、页 I/O（即使有缓存也要从页缓存复制到用户态）、行解析（字段长度编码解码）。

---

## 第五步：BinReader.read_pack() —— 零拷贝读取

**文件**：`range-store-core/src/bin_reader.rs:64-100`

```rust
pub fn read_pack(&self, offset: u32, byte_length: u32) -> io::Result<&[u8]> {
    let start = offset as usize;
    let len = byte_length as usize;
    let end = start + len;

    // 边界检查
    if start < PFSP_HEADER_SIZE { return Err(...); }  // 不能读到 header 里
    if end > self.mmap.len() { return Err(...); }     // 不能越界

    // 返回 mmap 的切片 —— 零拷贝！
    Ok(&self.mmap[start..end])
}
```

**关键点**：返回的是 `&[u8]` 引用，指向 mmap 区域的原始内存。没有 `malloc`，没有 `memcpy`，没有中间缓冲区。OS 会在首次访问这些页时触发 page fault，从磁盘加载到 page cache。

---

## 第六步：decode_pack_for_hand() —— 位运算解码

**文件**：`range-store-core/src/pack_codec.rs:34-105`

这是整个查询链中最重的计算，但仍然是纯 CPU 操作，没有 I/O。

### Pack 内存布局（以 hand_count=100, action_count=15 为例）

```
Offset 0..100:       hand_ids[100]          (u8, 已排序)
Offset 100..500:     action_masks[100]      (u32 LE)
Offset 500.....:     cell_data[100*15*8]    (f32 LE, frequency + hand_ev)
```

### 解码步骤（目标 hand_id=13）

```rust
// Step 1: binary search hand_id=13 在 hand_ids[0..100] 中
let hand_idx = binary_search_u8(hand_ids, 13);  // 假设找到 index=25

// Step 2: 读取 action_mask
let mask_offset = 100 + 25 * 4;    // 500 + 100 = 600
let mask = u32::from_le_bytes(mmap[600..604]);  // 例如 0b1011 (actions 0,1,3 存在)

// Step 3: 读取 cell data
let cells_start = 100 + 100 * 4;   // 500
let floats_per_hand = 15 * 2;      // 30 floats = 120 bytes
let cell_offset = 500 + 25 * 120;  // 500 + 3000 = 3500

// Step 4: 遍历 action_id 0..14，根据 mask 只解码存在的 cell
for action_id in 0..15 {
    if (mask >> action_id) & 1 == 0 { continue; }  // 位掩码过滤
    let cell_base = action_id * 8;
    let freq = f32::from_le_bytes(cell_data[cell_base..cell_base+4]);
    let ev   = f32::from_le_bytes(cell_data[cell_base+4..cell_base+8]);
    result.push(ActionResult { frequency: freq as f64, hand_ev: ev_to_option(ev) });
}
```

### 为什么快

1. **固定偏移计算**：每个 cell 正好 8 字节（2 个 f32），`offset = hand_idx * action_count * 8` 是纯算术，不需要解析变长字段
2. **位掩码过滤**：`(mask >> action_id) & 1` 是一条 CPU 指令，比 SQL 的 `WHERE action_name IN (...)` 快几个数量级
3. **NaN 检测**：`ev_raw.is_nan()` 是一条指令，NaN 在手牌数据中表示"不存在"（null 的替代方案）
4. **ArrayVec<32>**：结果存在栈上的 ArrayVec，不需要堆分配。最多 32 个 actions，全部在栈上完成

---

## 第七步：ActionSchemaCache.get() —— HashMap 查找

**文件**：`range-store-core/src/query/store_query_service.rs:409-433`

`decode_pack_for_hand` 返回的是 `action_id`（0, 1, 2...），不是动作名字。需要通过 `action_schema_id` 查表得到 `Vec<ActionDef>`：

```rust
// 先查内存缓存（RwLock read path，无锁竞争）
let state = self.state.read()?;
if let Some(schema) = state.schemas.get(&action_schema_id) {
    return Ok(Arc::clone(schema));  // O(1) clone 指针
}

// Cache miss：查 SQLite
let connection = self.connection()?;  // Mutex 串行化
let schema = load_action_schema_from_connection(&connection, action_schema_id)?;
// 写入缓存
state.schemas.insert(action_schema_id, Arc::new(schema));
```

**对比 SQLite**：这里只需要一次 `SELECT action_blob FROM action_schemas WHERE id = ?`，结果缓存到 HashMap。后续同 schema_id 的查询完全不碰 SQLite。

---

## 第八步：组装 ActionResult 返回给 JS

**文件**：`range-store-core/src/query/store_query_service.rs:186-200`

```rust
for cell in fragment.cells {
    let action = action_schema.get(cell.action_id as usize).unwrap();
    actions.push(ActionResult {
        action_name: action.action_name.to_string(),  // "fold", "call", "raise2.5"
        action_size: action.action_size,
        amount_bb: action.amount_bb,
        frequency: cell.frequency,
        hand_ev: cell.hand_ev,
    });
}
```

---

## 第九步：N-API 序列化 → JS

**文件**：`range-store-native/src/lib.rs:218-224`

```rust
Ok(QueryHandStrategyResponse {
    input_hole_cards: result.input_hole_cards,
    hand_code: result.hand_code,
    actions: result.actions.into_iter()
        .map(|a| ActionResult {
            action_name: a.action_name,
            action_size: f64::from(a.action_size),  // f32 -> f64
            amount_bb: f64::from(a.amount_bb),
            frequency: a.frequency,
            hand_ev: a.hand_ev,
        })
        .collect(),
})
```

然后通过 napi-rs 的序列化器将 Rust struct 转为 JS object。

**JS 层包装** (`index.js:123-141`)：
```javascript
queryHandStrategy(request) {
    const result = this.#native.queryHandStrategy({...});
    return {
        code: 0,
        data: {
            inputHoleCards: result.inputHoleCards,
            handCode: result.handCode,
            actions: result.actions.map(fromNativeAction),
        },
        message: null,
    };
}
```

最终返回给调用方：
```json
{
  "code": 0,
  "data": {
    "inputHoleCards": "AKs",
    "handCode": "AKs",
    "actions": [
      { "actionName": "fold", "frequency": 0.25, "handEv": null },
      { "actionName": "call", "frequency": 0.60, "handEv": 1.234 },
      { "actionName": "raise2.5", "frequency": 0.15, "handEv": 2.567 }
    ]
  },
  "message": null
}
```

---

## 对比：SQLite 路径

同样的查询 `SELECT ... WHERE concrete_line_id = 42 AND hole_cards = 'AKs'` 在 SQLite 中的路径：

| 步骤 | SQLite | Binary |
|------|--------|--------|
| 1. 定位 | SQL 解析 → 准备语句 → B-tree 索引查找 → 页读取 | `index = 42 - 1 = 41; offset = 41 * 22` |
| 2. 读取 | read() 系统调用 → 页缓存复制 → 行解析 | mmap slice `&self.mmap[start..end]`（零拷贝） |
| 3. 过滤 | `WHERE hole_cards = 'AKs'` → 字符串比较 | `binary_search_u8(hand_ids, 13)` → 整数比较 |
| 4. 解码 | 解析每个字段：`action_name` (变长文本) → 类型转换 | 固定偏移：`cell_offset = hand_idx * 15 * 8` |
| 5. 关联 | `JOIN action_schemas ON id = schema_id` → 另一次 B-tree 查找 | `HashMap.get(action_schema_id)` → 指针运算 |
| 6. 返回 | 构建结果行 → 序列化 | `ArrayVec<32>` 栈分配 → 直接返回 |

**关键差异**：SQLite 的路径上有 2 次 B-tree 遍历、1 次字符串 JOIN、变长字段解析、可能的磁盘 I/O。二进制路径上只有 1 次整数减法、1 次乘法、1 次二分搜索（log 169 ≈ 8 次比较），全部在内存中完成。

---

## 批量查询的特殊优化：queryBatch

batch 查询比 N 次单独查询快的原因：

### 1. 一次 handle 打开

```rust
// 单次查询：N 次 pool.get_or_open()
for item in requests { self.query(dimension, item.line_id, item.hand)? }

// batch 查询：1 次 pool.get_or_open()
let reader = self.pool.get_or_open(dimension)?;
for item in requests { self.query_single(&reader, dimension, item.line_id, item.hand)? }
```

### 2. 同一个 concrete_line 共享 pack 读取

`DimensionReader::query_many_hands()` 方法：

```rust
pub fn query_many_hands(&self, concrete_line_id, hand_ids: &[u8]) -> ... {
    // 只读一次 pack
    let pack = self.bin.read_pack(record.offset, record.byte_length)?;

    // 对每个 hand_id 从同一个 pack 解码
    hand_ids.iter().map(|&hand_id| {
        decode_pack_for_hand(pack, ..., hand_id)  // pack 引用共享，无拷贝
    }).collect()
}
```

### 3. 一次 N-API 序列化

Rust → JS 的对象序列化是一次性完成的，比 N 次单独的序列化开销小。

---

## hands-by-actions 的特殊路径

```rust
// 1. 读取整个 pack（比单次查询读更多数据）
let full_pack = reader.query_all(concrete_line_id, verify_checksum)?;

// 2. 完全解码 pack（169 手牌 × action_count 个 cells）
let decoded = decode_pack(pack, hand_count, action_count)?;

// 3. 构建位掩码（纯 CPU 位运算）
let mut hand_masks = [0u32; 169];
for cell in &decoded.cells {
    if cell.exists && cell.frequency > threshold {
        hand_masks[cell.hand_id] |= 1u32 << cell.action_id;
    }
}

// 4. 过滤（一次位 AND）
let filter_mask = 0b1011;  // fold=0, call=1, raise=3
decoded.hand_ids.into_iter()
    .filter(|&hid| hand_masks[hid] & filter_mask != 0)
    .map(hand_code_from_id)
    .collect()
```

这一步完全没有 I/O，纯 CPU 位运算处理 169 手牌。对比 SQLite 的 `WHERE (action_name = 'fold' OR action_name = 'call' ...) AND frequency > 0.005`，后者需要对每行做字符串比较和浮点比较。
