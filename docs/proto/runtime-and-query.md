# Proto V2 运行时与查询

更新日期：2026-07-14

`CompactLineMatrixArchive` 以只读 mmap 打开 data/index 文件。按 `concrete_line_id`
读取时查询 index、切出 payload、按配置校验 CRC、Protobuf decode，并建立：

```text
hand_id_to_global_index[169]
action_global_to_local_index[action_count * valid_hand_count]
```

读取无需重复扫描 bitmap：`hand_id -> global_compact_index -> action_compact_index` 均为 O(1)。
当前选择解码时预建索引，而不是请求时现场计算 rank；变更这一实现必须由新的内存和尾延迟
测量驱动，不能改变 V2 位图语义。

每个 dimension handle 有容量 1024 的 decoded matrix LRU。缓存值为
`Arc<DecodedCompactLineMatrix>`，命中只 clone `Arc`，不深拷贝 payload 或索引。
`read_matrix_profiled` 分别记录 cache lookup、index/payload、Protobuf decode、紧凑索引和
cache insert 时间。

`ProtoRangeStoreFacade` 扫描 root 下的 manifest 目录，并用标准维度键选择目录。它的 LRU
handle 同时持有惰性 matrix reader、惰性只读 `lines.db` 连接、成功的 concrete-line 查询
缓存与 drill-scenario 查询缓存。淘汰 handle 时它们一并释放；Proto 存储运行时不可变，
故 handle 生命周期内 metadata 缓存有效。

已实现的 core-compatible 接口：

```text
query_hand_strategy(dimension, concrete_line_id, hole_cards) -> QueryResult
query_batch(dimension, requests) -> QueryBatchResult
query_hands_by_actions(dimension, concrete_line_id, filters, frequency) -> Vec<String>
query_hands_by_action_names(dimension, concrete_line_id, action_names, frequency) -> Vec<String>
get_concrete_lines(dimension, filter) -> Vec<ConcreteLineRow>
get_drill_scenario_lines(strategy, drill_name, player_count, drill_depth) -> Vec<String>
```

batch 按 `concrete_line_id` 分组，同一 matrix 最多读取一次后恢复输入顺序。
hands-by-actions 复用 core filter 语义：频率阈值严格比较，非空 action filters 为 OR，金额
使用量化后精确匹配。缺少维度、concrete line 或保留手牌时，分别返回
`DIMENSION_NOT_FOUND`、`CONCRETE_LINE_NOT_FOUND`、`HAND_STRATEGY_NOT_FOUND`；batch 失败返回
带最小失败下标的 `BATCH_ITEM_ERROR`。
