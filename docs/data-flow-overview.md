# 数据流总览：编码写入 -> 业务读取

## 一、构建阶段（Build）

### 1.1 入口：`build_store()`

`build_orchestrator.rs:107`

```
输入: BuildOptions { source_db, out_dir, dimensions, max_concrete_lines_per_dimension, overwrite, resume }

主流程:
1. 校验 --resume/--overwrite 互斥，source_db 存在
2. Connection::open(source_db) → discover_dimensions() → select_dimensions()
     └── 扫描源 DB 中 range_data_* 表名，发现所有可构建维度
3. sha256_file(source_db) → source_db_checksum
4. prepare_build_state() → 初始化 meta.db + build-state.json
     ├── 创建 out_dir（overwrite 时先删，resume 时校验 build-state.json 一致性）
     ├── init_meta_db() → 建表
     ├── copy_metadata() → 从源 DB 复制 concrete_lines / drill_scenario_lines 数据
     └── build_info 写入 source_checksum + built_at
5. 遍历每个维度:
     ├── completed_state_dimension() → 如果 resume 且该维度已完成，跳过，直接从 build-state.json 恢复 ManifestDimension
     └── build_dimension() → 构建 .bin/.idx，标记 completed，写入 build-state.json
6. 写入 manifest.json（含 dimensions 列表、files 列表、source_db_checksum、built_at）
```

### 1.2 创建 meta.db

`init_meta_db()` — `build_orchestrator.rs:519`

创建以下表：

- `build_info` — key/value 表，写入 source_checksum 和 built_at 两个键
- `action_schemas` — 去重后的动作定义
  ```sql
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  action_count INTEGER NOT NULL,
  action_blob BLOB NOT NULL,
  checksum INTEGER NOT NULL,         -- CRC32C(action_blob)
  schema_key TEXT NOT NULL UNIQUE     -- to_hex(action_blob)，用于跨维度去重
  ```
- `concrete_lines_{strategy}_{N}max_{BB}BB` — 具体行定义（每个维度一张）
  ```sql
  concrete_line_id INTEGER PRIMARY KEY,
  abstract_line TEXT NOT NULL,
  concrete_line TEXT NOT NULL,
  UNIQUE(abstract_line, concrete_line)
  ```
- `drill_scenario_lines_{strategy}` — 训练场景数据（每个 strategy 一张，全局共享）
  ```sql
  UNIQUE(drill_name, player_count, drill_depth, abstract_line)
  ```

copy_metadata() 从源 SQLite 的对应表中 INSERT OR IGNORE 到 meta.db 的对应表，在同一事务中完成。

### 1.3 核心循环：`build_dimension_files()`

`build_orchestrator.rs:739`

```
前置: 创建 .bin.tmp 和 .idx.tmp 文件，写 PFSP/PFXI 头

1. 从源 DB 读取 rows:
   SELECT concrete_line_id, hole_cards, action_name, action_size, amount_bb, frequency, hand_ev
   FROM range_data_{strategy}_{N}max_{BB}BB
   ORDER BY concrete_line_id, hole_cards, action_name

2. 遍历时按 concrete_line_id 分组:
   ├── concrete_line_id 变化 → flush_pack()
   ├── 达到 max_concrete_lines 上限 → break
   └── 流结束 → flush_pack() 最后一个组

3. 验证当前 `concrete_line_id` 等于下一条隐式 idx id（`pack_count + 1`）；不连续时构建失败。

4. 重写 .idx 头: record_count = pack_count
5. bin.sync_all() + idx.sync_all()
6. 返回 (pack_count, concrete_line_count)
```

### 1.4 编码单个 pack：`encode_concrete_line_pack()`

`build_orchestrator.rs:890`

```
输入: 同一 concrete_line_id 的所有 RangeRow

步骤:
1. 规范化:
   ├── 每行 hole_cards → hand_id: u8 (通过 get_hand_id)
   ├── 每行 action_name → action_type: u8 (normalize_action_type: fold=0, call=1, check=2, bet=3, raise=4, allin=5)
   ├── 构建 ActionKey { action_type, action_size_bits, amount_bb_bits } → HashSet 去重
   └── 构建 normalized_rows: (hand_id, action_key, frequency, hand_ev)

2. 排序 actions:
   └── 按 (action_type, action_size, amount_bb) 升序 → Vec<ActionKey>

3. 构建索引:
   ├── hand_index: hand_id → position (0..hand_count)
   └── action_index: action_key → position (0..action_count)

4. 构建 action_blob:
   └── 每个 action 9 字节 = type(u8) + action_size(f32 LE) + amount_bb(f32 LE)

5. 构建 payload:
   ├── hand_ids: sorted u8 数组 (hand_count 字节)
   ├── action_masks: 每手牌一个 u32 (1 << action_position)，hand_count × 4 字节
   └── cell_data: hand_count × action_count × 8 字节
         每手牌 × 每动作 = (frequency as f32, hand_ev as f32 或 NAN)
         按 hand 外循环、action 内循环排列

输出: EncodedPack { action_blob, action_count, hand_count, payload }

校验: action_count ∈ [1, 32], hand_count ≤ u16::MAX
Pack 大小公式: hand_count × (5 + action_count × 8)
```

### 1.5 写入磁盘：`flush_pack()`

`build_orchestrator.rs:841`

```
1. encode_concrete_line_pack(rows) → EncodedPack { action_blob, payload, ... }

2. 去重写入 action_schemas:
   ├── schema_key = to_hex(action_blob)
   ├── schema_ids_by_key (进程内 HashMap) 缓存 → 命中直接返回
   ├── meta.db 查询 action_schemas WHERE schema_key = ? → 命中直接返回
   └── 未命中 → INSERT INTO action_schemas(action_count, action_blob, checksum, schema_key)
         checksum = CRC32C(action_blob)

4. 写 .bin:
   ├── byte_length = payload.len() as u32
   ├── checksum = CRC32C(payload)
   ├── bin.seek(bin_offset) → bin.write_all(&payload)
   └── bin_offset += byte_length

5. 写 .idx (append):
   └── 18 字节记录: action_schema_id(4) + hand_count(2) +
       offset(4) + byte_length(4) + CRC32C(4)
```

### 1.6 收尾

`build_dimension()` — `build_orchestrator.rs:677`

```
1. build_dimension_files() 完成后:
   ├── .bin.tmp → .bin (rename，非原子但比 copy 快)
   └── .idx.tmp → .idx

2. 构建 ManifestDimension:
   ├── concrete_line_count = pack_count
   ├── bin_file_size_bytes = fs::metadata(bin_path)?.len()
   ├── idx_file_size_bytes = fs::metadata(idx_path)?.len()
   ├── bin_file_checksum = sha256_file(&bin_path)
   └── idx_file_checksum = sha256_file(&idx_path)

3. mark_state_dimension_completed():
   └── 更新 build-state.json 中该维度的 status="completed" + checksums + completed_at

4. 所有维度构建完成后:
   └── 写入 manifest.json (pretty-print + trailing newline)
```

### 1.7 断点续建：`build-state.json`

`build_orchestrator.rs:19`

```
文件: out_dir/build-state.json

结构:
{
  version: 1,
  source_db: path,
  source_db_checksum: sha256,
  output_dir: path,
  built_at: ISO8601,
  updated_at: ISO8601,
  max_concrete_lines_per_dimension: Option<usize>,
  dimensions: [
    {
      strategy, player_count, depth_bb,
      status: "pending" | "completed",
      concrete_line_count, pack_count,
      bin_file, idx_file,
      bin_file_size_bytes, idx_file_size_bytes,
      bin_file_checksum, idx_file_checksum,
      completed_at
    }
  ]
}

resume 模式:
1. 校验 build-state.json 版本、source_db_checksum、max_concrete_lines、dimensions 列表
2. 已完成的维度: 校验 bin/idx 文件大小和 SHA-256 一致性
3. 跳过的维度直接返回 ManifestDimension，不重新构建
4. 未完成的维度正常 build_dimension()，完成后更新 build-state.json
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

`range-store-core/src/query/store_query_service.rs:451`

```
外部入口:
- HTTP: POST /range/prewarm
  → service/src/routes/hand_query_routes.rs:414 prewarm()
  → service/src/query/hand_query_service.rs prewarm()
  → range-store-core/src/query/range_store_facade.rs:158 prewarm()
  → range-store-core/src/query/store_query_service.rs:451 StoreQueryService::prewarm()
- 服务启动时自动预加载 PHS_PREWARM 环境变量指定的维度列表

1. StoreQueryService::prewarm(dimension)
   ├── pool.get_or_open(dimension)
   │     ├── DimensionReader::open(idx_path, bin_path)
   │     ├── IdxReader::open() → mmap .idx，验证 PFXI 头和 dense record
   │     └── BinReader::open() → mmap .bin，验证 PFSP 头
   └── 返回 pool.open_count() — 当前已打开的 dimension handle 数量

2. HTTP prewarm 支持批量预加载:
   → 遍历请求中的 dimensions 列表，逐个调用 prewarm()
   → 返回 { prewarmed: 本次请求数, total_open: 池内总数 }

注意: prewarm 只提前打开/mmap 维度 .idx/.bin 文件；action_schemas 仍然在第一次查询用到具体 action_schema_id 时懒加载。
```

### 2.3 单次查询：`query()`

`range-store-core/src/query/store_query_service.rs:223`

```
外部入口:
- HTTP: POST /range/hand-strategy
  → service/src/routes/hand_query_routes.rs:299 query()
  → service/src/query/hand_query_service.rs query()
  → range-store-core/src/query/store_query_service.rs:223 StoreQueryService::query()
- Native SDK: PokerHandsRange.queryHandStrategy()
  → range-store-native/src/lib.rs:197 query_hand_strategy()
  → RangeStoreFacade::query_hand_strategy()
  → StoreQueryService::query()

输入: dimension + concrete_line_id + hole_cards

1. StoreQueryService::query()
   ├── parse_hole_cards(hole_cards)
   │     └── 将 "AsKh" / "AKs" / "AKo" 等输入归一化为 hand_id: u8 (0..168)
   └── pool.get_or_open(dimension)
         └── 获取或打开对应维度的 DimensionReader

2. DimensionReader::query(concrete_line_id, hand_id)
   ├── IdxReader.find(concrete_line_id)
   │     ├── dense idx 下直接计算 record 偏移:
   │     │     idx_offset = 16 + (concrete_line_id - 1) * 18
   │     └── 读取 18 字节 IdxRecord:
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
- .idx 的一条 18 字节 record 对应 .bin 中 offset..offset+byte_length 的一个 pack payload；第 N 条 record 隐式对应 concrete_line_id=N+1。
```

### 2.4 批量查询：`query_batch()`

`range-store-core/src/query/store_query_service.rs:255`

```
外部入口:
- HTTP: POST /range/hand-strategy-batch
  → service/src/routes/hand_query_routes.rs:334 batch()
  → service/src/query/hand_query_service.rs:93 query_batch()
  → range-store-core/src/query/store_query_service.rs:255 StoreQueryService::query_batch()
- Native SDK: PokerHandsRange.queryBatch()
  → range-store-native/src/lib.rs:217 query_batch()
  → RangeStoreFacade::query_batch()
  → StoreQueryService::query_batch()

输入: dimension + [{ concrete_line_id, hole_cards }]

1. StoreQueryService::query_batch()
   ├── pool.get_or_open(dimension)
   │     └── 整个 batch 只打开/复用一个 DimensionReader
   ├── Stage 1: 逐项 parse_hole_cards(hole_cards)
   │     ├── 成功: 得到 hand_id，记入 parsed_hand_ids
   │     └── 失败: 记录到 first_failure (min index)，记入 parsed_hand_ids 为 None
   ├── Stage 2: 按 concrete_line_id 分组
   │     ├── 只收集 parse 成功的 item: (原始 index, hand_id)
   │     ├── group_keys 记录首次出现的 line_id 顺序
   │     └── groups[group_pos] = [(min_index_in_group, hand_id), ...]
   ├── Stage 3: 对每个 group 调用 DimensionReader::query_many_hands()
   │     ├── IdxReader.find(concrete_line_id) → 只查一次 .idx record
   │     ├── BinReader.read_pack(offset, byte_length) → 只切一次 .bin pack payload
   │     ├── read_and_validate_pack() → 一次 pack 边界/长度/action_count/checksum 校验
   │     └── 对该组中的多个 hand_id 逐个 decode_pack_for_hand()
   │           → 返回 Vec<Option<PackDecodeResult>> (每项 None = hand 不在 pack 中)
   ├── Stage 4: 结果回填 (per-item)
   │     ├── LineNotFound: 记到 first_failure(min_index, ConcreteLineNotFound)
   │     ├── IO 错误: 记到 first_failure(min_index, Io)
   │     ├── action_schema 不存在: 记到 first_failure(min_index, ActionSchemaNotFound)
   │     ├── hand_id 不在 pack 中 (None): 记到 first_failure(min_index, HandStrategyNotFound)
   │     ├── cells_to_actions 失败: 记到 first_failure(min_index, Internal)
   │     └── 成功: results_vec[item_index] = Some(actions)
   └── Stage 5: first-failure-by-min-index
         ├── 遍历 first_failure，取 index 最小的那个作为全局唯一失败
         ├── 若存在失败: 返回 Err(StoreQueryError::BatchItem { index, source })
         └── 若无失败: 按输入顺序组装 QueryBatchResult

关键边界:
- `query_batch` 不是删除项：HTTP service、native SDK 和 benchmark 都有外部入口在使用。
- 当前实现走 `query_many_hands()`；同一 concrete_line_id 的多手牌共享一次 idx lookup、一次 bin pack read、一次 pack 校验。
- 错误语义是 first-failure-by-min-index：逐项探测所有失败（为了正确归因），但最终只返回最小 index 的那个失败，包装为 BatchItem 错误。单个 item 失败会让整个 batch 返回错误响应，而不是逐条填充 results。
- 分组只是内部优化；成功时 results 仍按输入 items 的顺序返回。
```

### 2.5 按动作过滤手牌：`hands-by-actions`

`range-store-core/src/query/store_query_service.rs:413`

```
外部入口:
- HTTP: POST /range/hands-by-actions
  → service/src/routes/hand_query_routes.rs:368 hands_by_actions()
  → service/src/query/hand_query_service.rs query_hands_by_actions()
  → range-store-core/src/query/store_query_service.rs:413 StoreQueryService::query_hands_by_actions()
- Native SDK: PokerHandsRange.handsByActions()
  → range-store-native/src/lib.rs:177 hands_by_actions()
  → RangeStoreFacade::hands_by_action_names()
  → StoreQueryService::query_hands_by_actions()

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
- `query()` 是查一个 hand 的 action 策略；`hands-by-actions` 是反向查询，查”哪些 hand 满足某些 action/frequency 条件”。
- `hands-by-actions` 必须完整解码一个 pack，因为它要扫描该 concrete line 下所有 hand/action cell。
- action filter 的语义是 OR：多个 actions 中任意一个满足频率阈值即可返回该手牌。
```

### 2.6 查询 concrete line 映射：`concrete-lines`

`range-store-core/src/query/range_store_facade.rs:64`

```
外部入口:
- HTTP: POST /range/concrete-lines
  → service/src/routes/metadata_routes.rs:203 concrete_lines()
  → service/src/query/hand_query_service.rs:111 get_concrete_lines()
  → range-store-core/src/query/range_store_facade.rs:64 get_concrete_lines()
  → range-store-core/src/metadata.rs:521 CachedMetadataReader::get_concrete_lines()

输入: dimension(strategy, player_count, depth_bb) + { abstract_line?, concrete_line? }
      (至少提供一个 abstract_line 或 concrete_line)

1. CachedMetadataReader::get_concrete_lines()
   ├── 参数路由:
   │     ├── (abstract, concrete) 均提供 → get_concrete_lines_by_abstract_and_concrete()
   │     ├── 仅 abstract 提供 → get_concrete_lines_by_abstract()
   │     └── 仅 concrete 提供 → get_concrete_lines_by_concrete()
   ├── 先查内存缓存 state.concrete_by_abstract / state.concrete_by_concrete
   │     └── cache hit → 直接返回
   └── cache miss → 从 meta.db 查询
         ├── 构造表名: concrete_lines_{strategy}_{player_count}max_{depth_bb}BB
         ├── SELECT concrete_line_id, abstract_line, concrete_line FROM {table} WHERE ...
         └── 回填缓存: concrete_by_abstract[key] = rows, concrete_by_concrete[row.concrete_line] = row

2. 返回 Vec<ConcreteLineRow>
   └── { concrete_line_id, abstract_line, concrete_line }

注意: CachedMetadataReader 是纯内存缓存，不 mmap 任何文件；所有数据来自 meta.db 的只读 SQLite 查询。
```

### 2.7 查询 drill scenario 抽象线：`drill-scenarios`

`range-store-core/src/query/range_store_facade.rs:87`

```
外部入口:
- HTTP: POST /range/drill-scenarios
  → service/src/routes/metadata_routes.rs:249 drill_scenario_lines()
  → service/src/query/hand_query_service.rs:121 get_drill_scenario_lines()
  → range-store-core/src/query/range_store_facade.rs:87 get_drill_scenario_lines()
  → range-store-core/src/metadata.rs:759 CachedMetadataReader::get_drill_scenario_lines()

输入: strategy + drill_name + player_count + drill_depth

1. CachedMetadataReader::get_drill_scenario_lines()
   ├── 先查缓存 state.drill_lines[(strategy, drill_name, player_count, drill_depth)]
   │     └── cache hit → 直接返回 Vec<String>
   └── cache miss → 从 meta.db 查询
         ├── 构造表名: drill_scenario_lines_{strategy}
         ├── SELECT abstract_line FROM {table}
         │     WHERE drill_name=? AND player_count=? AND drill_depth=?
         │     ORDER BY abstract_line
         └── 回填缓存: state.drill_lines[key] = lines

2. 返回 Vec<String>（抽象行动线名称，如 [“F-F-F”, “F-F-F-R2”]）

注意: drill scenario 表只按 strategy 区分，不按 player_count/depth_bb 建多张表。
```

## 三、构建阶段补充：meta.db 表结构

`CachedMetadataReader` 和 `ActionSchemaCache` 共同操作 `meta.db`，以下是核心表：

```
action_schemas:
  id (u32 PK) | action_count (u32) | action_blob (bytes)
  每个 action 占 9 字节: type(u8) + size(f32 LE) + amount_bb(f32 LE)

concrete_lines_{strategy}_{player_count}max_{depth_bb}BB:
  concrete_line_id (u32 PK) | abstract_line (TEXT) | concrete_line (TEXT)
  每个维度一张独立表

drill_scenario_lines_{strategy}:
  id (u32 PK) | drill_name (TEXT) | abstract_line (TEXT)
  | player_count (u32) | drill_depth (u32)
  全局共享表，按 strategy 命名
```

## 四、hand_id 说明

**hand_id** 是德州扑克 169 种起手牌的数值编码，将组合数学上的 169 种 hand 映射为 `u8` (0..168)。

编码规则基于 13×13 矩阵（RANKS = [A,K,Q,J,T,9,8,7,6,5,4,3,2]）：

| hand_id 公式                 | 含义           | 示例                 |
| ---------------------------- | -------------- | -------------------- |
| `row * 13 + row`             | 对子 (pair)    | `AA` → 0, `22` → 168 |
| `row * 13 + col` (row < col) | 同花 (suited)  | `AKs` → 1            |
| `col * 13 + row` (row < col) | 杂色 (offsuit) | `AKo` → 13           |

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

[idx record 1][idx record 2]...  (each 18 bytes)
  action_schema_id(4) + hand_count(2) +
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
