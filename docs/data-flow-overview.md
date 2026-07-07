# 数据流总览：编码写入 -> 业务读取

## 一、构建阶段（Build）

### 1.1 入口：`build_store()`
`storage-tools/src/range_store_builder/build_orchestrator.rs:107`

```
源 SQLite (range.db)
  ├── discover_dimensions()          → 扫描 range_data_* 表名，发现所有维度
  ├── prepare_build_state()          → 创建 meta.db + 初始化表结构
  └── 遍历每个维度:
        └── build_dimension()        → 构建 .bin + .idx + 更新 dimension_action_schemas
```

### 1.2 创建 meta.db
`init_meta_db()` — `build_orchestrator.rs:519`

创建以下表：
- `build_info` — 构建时间戳 + 源 DB SHA-256
- `action_schemas` — 去重后的动作定义
- `dimension_action_schemas` — 维度与 schema 的映射
- `concrete_lines_{strategy}_{N}max_{BB}BB` — 具体行定义
- `drill_scenario_lines_{strategy}` — 训练场景数据

### 1.3 核心循环：`build_dimension_files()`
`build_orchestrator.rs:739`

```
1. 创建 .bin.tmp 和 .idx.tmp 文件
2. 写 PFSP 头到 .bin (16 bytes)
3. 写 PFXI 头到 .idx (record_count=0, 最后重写)
4. 从源 SQLite 读取 rows (ORDER BY concrete_line_id, hole_cards, action_name)
5. 遍历时按 concrete_line_id 分组：
   └── concrete_line_id 变化 → flush_pack()
```

### 1.4 编码单个 pack：`encode_concrete_line_pack()`
`build_orchestrator.rs:890`

```
输入: 同一 concrete_line_id 的所有 rows

步骤:
1. 收集所有 unique actions → HashSet<ActionKey>
2. 按 (action_type, action_size, amount_bb) 排序
3. 构建 action_blob: 每个动作 9 字节 = type(u8) + size(f32) + amount_bb(f32)
4. 构建 hand_ids: 排序后的 u8 数组 (0..168，代表该 concrete_line 覆盖的手牌)
5. 构建 action_masks: 每手牌一个 u32 位掩码
6. 构建 cell_data: 每手牌 × 每个动作 = (freq f32, ev f32) 交替排列

输出: EncodedPack { hand_ids, action_masks, cell_data }
      action_schema_id (通过 get_or_insert_action_schema 获取/插入)
```

Pack 大小公式: `hand_count × (5 + action_count × 8)`

### 1.5 写入磁盘：`flush_pack()`
`build_orchestrator.rs:841`

```
1. encode_concrete_line_pack() → EncodedPack
2. get_or_insert_action_schema() → 去重写入 action_schemas + dimension_action_schemas
3. bin.seek_to(bin_offset) → 写 pack bytes 到 .bin
4. 写 idx record (22 bytes) 到 .idx:
   - concrete_line_id (u32)
   - action_schema_id (u32)
   - hand_count (u16)
   - offset into .bin (u32)
   - byte_length (u32)
   - CRC32C checksum (u32)
5. bin_offset += pack_byte_length
```

### 1.6 收尾
```
1. 重写 .idx 头: record_count = 实际记录数
2. .bin.tmp → .bin (原子重命名)
3. .idx.tmp → .idx (原子重命名)
4. 写 manifest.json
```

---

## 二、运行时阶段（Service）

### 2.1 启动：`serve()`
`service/src/http/server.rs`

```
QueryService::open_with_meta(data_dir, meta_db_path)
  └── RangeStoreFacade::open_with_meta()
      ├── CachedMetadataReader::load(data_dir, meta_db_path)
      │     ├── 加载 manifest.json → 解析可查询维度
      │     ├── 打开只读 meta.db 连接
      │     └── concrete_lines / drill_lines 元数据按查询 key 懒加载并缓存
      └── StoreQueryService::open_with_meta()
            ├── 加载 manifest.json → 解析可查询维度
            ├── ActionSchemaCache::new(meta_db)
            │     └── 持有只读 SQLite 连接，按 action_schema_id 懒加载 ActionDef 列表
            └── HandlePool::new(data_dir, dimensions, max_open_handles)
                  └── 只记录可查询维度和 LRU 容量；不立即 mmap .idx/.bin
```

### 2.2 Prewarm：打开维度句柄
`range-store-core/src/query/store_query_service.rs:397`

```
1. HTTP 启动时读取 PHS_PREWARM，或外部调用 /range/prewarm
2. QueryService::prewarm()
   └── RangeStoreFacade::prewarm()
       └── StoreQueryService::prewarm()
3. pool.get_or_open(dimension)
   ├── DimensionReader::open(idx_path, bin_path)
   ├── IdxReader::open() → mmap .idx，验证 PFXI 头和 dense record
   └── BinReader::open() → mmap .bin，验证 PFSP 头
4. 返回当前已打开的 dimension handle 数量

注意: prewarm 只提前打开/mmap 维度 .idx/.bin 文件；action_schemas 仍然在第一次查询用到具体 action_schema_id 时懒加载。
```

### 2.3 单次查询：`query()`
`range-store-core/src/query/store_query_service.rs:164`

```
输入: dimension + concrete_line_id + hole_cards

1. StoreQueryService::query()
   ├── parse_hole_cards(hole_cards)
   │     └── 将 "AsKh" / "AKs" / "AKo" 等输入归一化为 hand_id: u8 (0..168)
   └── pool.get_or_open(dimension)
         └── 获取或打开对应维度的 DimensionReader

2. DimensionReader::query(concrete_line_id, hand_id)
   ├── IdxReader.find(concrete_line_id)
   │     ├── dense idx 下直接计算 record 偏移:
   │     │     idx_offset = 16 + (concrete_line_id - 1) * 22
   │     └── 读取 22 字节 IdxRecord:
   │           concrete_line_id
   │           action_schema_id
   │           hand_count
   │           offset
   │           byte_length
   │           checksum
   ├── BinReader.read_pack(record.offset, record.byte_length)
   │     └── 从 .bin mmap 中切出该 concrete_line_id 对应的一个 pack payload
   ├── read_and_validate_pack()
   │     ├── 校验 hand_count > 0
   │     ├── 校验 offset + byte_length 不越界
   │     ├── 由 hand_count + byte_length 反推出 action_count:
   │     │     action_count = (byte_length / hand_count - 5) / 8
   │     ├── 校验 byte_length == hand_count * (5 + action_count * 8)
   │     ├── 校验 action_count <= 32
   │     └── 如果开启 verify_checksum，校验 pack payload 的 CRC32C
   └── decode_pack_for_hand(pack, hand_count, action_count, hand_id)

3. pack payload 内部解析
   ├── hand_ids 段:
   │     hand_ids[hand_count]
   │     在已排序 hand_ids 中查找目标 hand_id（详见第五节）
   ├── action_masks 段:
   │     action_masks[hand_count]
   │     取目标 hand_index 对应的 u32 mask
   └── cells 段:
         cells[hand_count][action_count]
         cell_offset = cells_start + hand_index * action_count * 8
         每个 cell 为 frequency f32 + hand_ev f32

4. action_mask 与 cells 的映射
   ├── action_id 从 0 遍历到 action_count - 1
   ├── 如果 (mask >> action_id) & 1 == 0:
   │     跳过该 action，说明这个手牌在该 action 上没有有效策略值
   └── 如果 mask 位为 1:
         从 cells[hand_index][action_id] 读取 frequency / hand_ev
         返回 DecodedCellResult { action_id, frequency, hand_ev }

5. action_schema 语义映射
   ├── DimensionReader 返回:
   │     PackDecodeResult { action_schema_id, cells }
   ├── StoreQueryService 用 action_schema_id 查询 ActionSchemaCache
   │     └── cache miss 时从 meta.db.action_schemas 读取 action_blob 并解码
   ├── action_blob 解码得到:
   │     action_count
   │     actions[action_id] = (action_name/action_type, action_size, amount_bb)
   └── 将每个 DecodedCellResult.action_id 映射到对应 action 定义:
         ActionResult {
           action_name,
           action_size,
           amount_bb,
           frequency,
           hand_ev
         }

关键边界:
- hand_count 来自 .idx record，不来自 action_schema。
- action_count 在查询热路径中由 .idx 的 hand_count + byte_length 推导；action_schema 中的 action_blob 提供 action_id 的业务语义。
- .idx 负责定位和边界，.bin pack 负责具体手牌策略数值，meta.db.action_schemas 负责动作定义。
- .idx 的一条 22 字节 record 对应 .bin 中 offset..offset+byte_length 的一个 pack payload。
```

### 2.4 批量查询：`query_batch()`
`range-store-core/src/query/store_query_service.rs:210`

```
外部入口:
- HTTP: POST /range/hand-strategy-batch
- Native SDK: PokerHandsRange.queryBatch()
- Core: RangeStoreFacade::query_batch() / StoreQueryService::query_batch()

输入: dimension + [{ concrete_line_id, hole_cards }]

1. StoreQueryService::query_batch()
   ├── pool.get_or_open(dimension)
   │     └── 整个 batch 只打开/复用一个 DimensionReader
   ├── 逐项 parse_hole_cards(hole_cards)
   │     ├── 成功: 得到 hand_id，进入后续分组
   │     └── 失败: 包装成带 requests[index] 上下文的 batch error，整个 batch 返回错误
   └── 按 concrete_line_id 分组:
         concrete_line_id -> [(原始 index, hole_cards, hand_id)]

2. 每个 concrete_line_id 分组调用一次 DimensionReader::query_many_hands()
   ├── IdxReader.find(concrete_line_id)
   │     └── 只查一次 .idx record
   ├── BinReader.read_pack(record.offset, record.byte_length)
   │     └── 只切一次 .bin pack payload
   ├── read_and_validate_pack()
   │     └── 只做一次 pack 边界、长度、action_count、checksum 校验
   └── 对该组中的多个 hand_id 逐个 decode_pack_for_hand()
         └── 返回 Vec<Option<PackDecodeResult>>

3. 结果组装与错误传播
   ├── concrete_line_id 不存在:
   │     包装成带 requests[index] 上下文的 batch error，整个 batch 返回错误
   ├── 某个 hand_id 不在该 pack 的 hand_ids 段:
   │     包装成对应 item 的 batch error，整个 batch 返回错误
   ├── action_schema_id 存在:
   │     对该组只查一次 ActionSchemaCache
   └── 成功 item:
         cell.action_id -> action_schema[action_id]
         输出 concrete_line_id/hole_cards/actions

4. 保持原始请求顺序
   └── 分组只是内部优化；成功时 results 仍按输入 items 的顺序返回。

关键边界:
- `query_batch` 不是删除项：HTTP service、native SDK 和 benchmark 都有外部入口在使用。
- 当前实现已经走 `query_many_hands()`；同一 concrete_line_id 的多手牌共享一次 idx lookup、一次 bin pack read、一次 pack 校验。
- 错误语义是 all-or-nothing：单个 item 失败会让整个 batch 返回错误，不返回 item-level `error`。
```

### 2.5 按动作过滤手牌：`hands-by-actions`
`range-store-core/src/query/store_query_service.rs:359`

```
外部入口:
- HTTP: POST /range/hands-by-actions
- Native SDK: PokerHandsRange.handsByActions()
- Core: RangeStoreFacade::hands_by_actions() / StoreQueryService::query_hands_by_actions()

输入: dimension + concrete_line_id + actions? + frequency?

1. HTTP/SDK 请求层
   ├── actions 为空或缺省:
   │     表示不过滤具体动作，只要求手牌至少有一个有效 action 超过 frequency 阈值
   ├── actions 非空:
   │     parse_action_filters()
   │     支持 fold/check/call/bet/raise/allin
   │     bet/raise/allin 可以带 amount 后缀，例如 raise2.5
   └── frequency 缺省:
         使用 DEFAULT_HANDS_BY_ACTIONS_FREQUENCY = 0.005

2. StoreQueryService::query_hands_by_actions()
   ├── pool.get_or_open(dimension)
   ├── reader.query_all(concrete_line_id)
   │     ├── IdxReader.find(concrete_line_id)
   │     ├── BinReader.read_pack(record.offset, record.byte_length)
   │     ├── read_and_validate_pack()
   │     └── decode_pack(pack, hand_count, action_count)
   └── 得到 FullRangeDecodeResult:
         action_schema_id
         DecodedPack { hand_ids, action_masks, cells }

3. 完整 pack 解码方式
   ├── hand_ids:
   │     pack 中实际存在的手牌列表
   ├── action_masks:
   │     每个 hand_index 一个 u32 mask
   └── cells:
         cells[hand_count][action_count]
         每个 cell 带 exists/frequency/hand_ev
         exists 由对应 hand 的 action_mask 位决定

4. action filter 映射成 bit mask
   ├── 通过 action_schema_id 读取 action_blob
   ├── action_blob 解码出 action_schema[action_id]
   ├── 每个 ActionFilter 去匹配 action_schema:
   │     action_name 必须一致
   │     如果 filter 指定 amount_bb，则 amount_bb 也必须一致
   └── 得到 filter_mask:
         bit(action_id) = 1 表示这个 action_id 被请求的 filters 命中

5. match_hands_by_actions()
   ├── 初始化 hand_masks[169] = 0
   ├── 遍历 pack.cells:
   │     如果 !cell.exists，跳过
   │     如果 cell.frequency <= frequency_threshold，跳过
   │     如果 cell.action_id >= 32，跳过
   │     否则:
   │       hand_masks[cell.hand_id] |= 1 << cell.action_id
   └── 遍历 pack.hand_ids:
         actions 为空:
           hand_masks[hand_id] != 0 就返回该手牌
         actions 非空:
           hand_masks[hand_id] & filter_mask != 0 就返回该手牌

6. 输出
   ├── StoreQueryService 返回 Vec<hand_code>
   ├── RangeStoreFacade::hands_by_actions() 会把空 Vec 转成 NoHandsFound 业务错误
   └── HTTP/SDK 最终返回:
         { holeCards: ["AA", "AKs", ...] }

关键边界:
- `query()` 是查一个 hand 的 action 策略；`hands-by-actions` 是反向查询，查“哪些 hand 满足某些 action/frequency 条件”。
- `hands-by-actions` 必须完整解码一个 pack，因为它要扫描该 concrete line 下所有 hand/action cell。
- action filter 的语义是 OR：多个 actions 中任意一个满足频率阈值即可返回该手牌。
```

## 四、hand_id 说明

**hand_id** 是德州扑克 169 种起手牌的数值编码，将组合数学上的 169 种 hand 映射为 `u8` (0..168)。

编码规则基于 13×13 矩阵（RANKS = [A,K,Q,J,T,9,8,7,6,5,4,3,2]）：

| hand_id 公式 | 含义 | 示例 |
|---|---|---|
| `row * 13 + row` | 对子 (pair) | `AA` → 0, `22` → 168 |
| `row * 13 + col` (row < col) | 同花 (suited) | `AKs` → 1 |
| `col * 13 + row` (row < col) | 杂色 (offsuit) | `AKo` → 13 |

查询接口接收两种输入格式，都归一化为 hand_id：
- 标准 169-hand code：`"AA"`, `"AKs"`, `"AKo"`
- 花色两牌：`"AsKh"`, `"AcAd"`, `"QdJs"` → 自动归一化为上述格式

---

## 五、pack 内 hand_id 查找为何使用二分查找

`decode_pack_for_hand` 在 pack 的 `hand_ids` 段中查找目标 hand_id。`hand_ids` 是一个已排序的 u8 数组（最多 169 个元素），当前使用二分查找。选择二分查找而非直接下标的原因：

### 1. pack 是稀疏的

每个 concrete_line 的 pack 不一定包含全部 169 手牌。例如某些 action line 可能只覆盖了 100 种起手牌。因此 `hand_ids` 段的长度是可变的，不能用 `hand_ids[target_hand_id]` 这种固定偏移访问。

### 2. 不能改用 lookup table 的原因

如果要构建 `lookup[hand_id] → index` 的 169 字节映射表，需要在 pack 中额外存储这张表，或者在运行时动态构建。

- **存磁盘**：改变 pack 二进制格式，所有已有数据需要重建
- **运行时构建**：每次查询都多出一趟 O(hand_count) 的构建成本，抵消了查找收益
- **DimensionReader 级别缓存**：169 字节对单个 reader 影响不大，但 LRU pool 中每个 reader 都持有一份，且 hand_ids 本身就在 L1 cache 中，额外引入 lookup table 反而增加了数据结构复杂度

### 3. 二分查找在此场景下已经足够快

- 最多 169 元素 → 不超过 8 次比较
- 数据全程在 L1 cache（`hand_ids` 段仅 169 字节）
- 分支预测器能很好地处理固定模式的查询
- 真正的性能瓶颈在 mmap 页表查找和 pack 解码（8 字节/手的浮点读写），而非 8 次整数比较

---

## 六、文件格式速查

> 详细格式定义以 [`range-db-binary-storage-design.md`](./range-db-binary-storage-design.md) 为准，此处为快速参考。

### PFSP (.bin)
```
Offset  Size  Field
0       4     Magic: "PFSP"
4       2     Version: 1
6       1     Endian: 1
7       1     Float type: 1
8       1     Layout: 1
9       1     Compression: 0
10      2     Header size: 16
12      4     Reserved

[pack1][pack2][pack3]...  (concatenated, each hand_count × (5 + action_count × 8) bytes)
```

### PFXI (.idx)
```
Offset  Size  Field
0       4     Magic: "PFXI"
4       2     Version: 1
6       2     Reserved
8       4     Record count
10      2     Header size: 16
12      2     Reserved

[idx record 1][idx record 2]...  (each 22 bytes)
  concrete_line_id(4) + action_schema_id(4) + hand_count(2) +
  offset(4) + byte_length(4) + CRC32C(4)
```

### action_blob (每动作 9 字节)
```
action_type(u8) + action_size(f32 LE) + amount_bb(f32 LE)
type: 0=fold, 1=call, 2=check, 3=bet, 4=raise, 5=allin
```

### pack 内部结构 (每手牌)
```
hand_ids:    hand_count bytes  (sorted u8, 0..168)
action_masks: hand_count × 4 bytes (u32 bitset)
cell_data:   hand_count × action_count × 8 bytes (f32 freq + f32 ev)
```
