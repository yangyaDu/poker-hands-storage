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
  ├── 加载 manifest.json → 解析 BuildManifest
  ├── queryable_dimensions() → 获取可查询维度列表
  ├── MetadataReader::open(meta_db)
  │     ├── load_action_schema_ids() → HashSet<u32>
  │     │     SELECT id FROM action_schemas
  │     ├── ActionSchemaCache::new(meta_db)
  │     │     持有只读 SQLite 连接，按 schema_id 懒加载 ActionDef 列表
  │     ├── validate_dimension_schema_refs() → FK 完整性检查
  │     └── dimension_action_schema_ids() → 查询维度映射
  ├── 为每个维度: DimensionReader::open(idx_path, bin_path)
  │     ├── IdxReader::open() → mmap .idx, 验证 PFXI 头
  │     └── BinReader::open() → mmap .bin, 验证 PFSP 头
  ├── 交叉验证: .idx 中的 action_schema_id 全部存在于 action_schemas id 集合
  └── 构建 HandlePool (LRU 缓存的 DimensionReader 池)
```

### 2.2 Prewarm 校验
`hand_query_service.rs:428`

```
1. 打开 DimensionReader
2. 查 dimension_action_schemas 表 → 期望的 action_schema_id 集合
3. 扫描 .idx 文件 → 实际的 action_schema_id 集合
4. 对比两者，不一致则报错
```

### 2.3 单次查询：`query()`
`hand_query_service.rs:147`

```
输入: dimension + concrete_line_id + hole_cards

1. parse_hole_cards("AsKh" → hand_id: 0..168)
2. pool.get_or_open(dimension) → Arc<DimensionReader>
3. reader.query(concrete_line_id, hand_id):
   ├── IdxReader.find(concrete_line_id):
   │     └── 直接计算: idx_offset = HEADER_SIZE + (concrete_line_id - first_concrete_line_id) × IDX_RECORD_SIZE
   ├── BinReader.read_pack(record.offset, record.byte_length) → &pack_bytes
   └── decode_pack_for_hand(pack, hand_count, action_count, hand_id):
         ├── 在 hand_ids 中查找目标 hand_id（详见第五节）
         ├── 读取 action_mask (u32)
         └── 只返回 mask 位为 1 的 cell: (action_id, frequency, hand_ev)
4. 通过 IdxRecord.action_schema_id 查 ActionSchemaCache，miss 时查 SQLite 并缓存 → Vec<ActionDef>
5. 每个 cell: action_id → ActionDef.action_name → ActionResult
```

### 2.4 批量查询：`query_batch()`
`hand_query_service.rs:214`

```
Phase 1: 解析所有 hole_cards → hand_ids
Phase 2: 按 concrete_line_id 分组
Phase 3: 每组调用 reader.query_many_hands() (一次 pack 解码多手牌)
Phase 4: 填充错误条目
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
