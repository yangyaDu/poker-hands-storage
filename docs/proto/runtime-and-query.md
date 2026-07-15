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

## Concrete Line 语法

具体行动线由 `-` 连接的 action token 组成。当前 token 枚举为 `F`、`C`、`R`、`A`、`X`、`B`。
其中 `R`、`A`、`X` 的数值是同一个 token 的后缀，例如 `R2`、`A100`；数值不是单独的
`-` 分段。reader、metadata lookup 与 line-transition benchmark 都把 token 作为不透明字符串，
只通过移除最后一个 `-<token>` 得到父节点。

空字符串 `""` 是根节点，表示第一个人尚未行动前的 169 手牌矩阵预测。它是顶层 token
（例如 `F`、`C`、`R...`）的父节点，因此从根节点开始的路径形如
`"" -> F -> F-F -> F-F-R2`。本文不为这些 token 推断额外业务语义；其规范以导出数据为准。

### Line-transition 规范化

6-max 的 source 数据会省略一轮末尾的默认弃牌。例如 `F-F-R2-R7.5-A100` 不单独存储，
其可查询 matrix 是 `F-F-R2-R7.5-A100-F`。line-transition benchmark 对每个 token prefix
先精确查找；若不存在，仅依次追加最多 `player_count` 个 `F`，并且只有追加结果实际存在于
source concrete-lines 表时才视为规范节点。相邻 prefix 解析到同一规范节点时只查询一次。

该规则用于构造 benchmark 的行动线拓扑，不会在 reader 中凭空制造 concrete line；实际
Protobuf / SQLite 查询仍以 metadata 中存在的 canonical concrete line 为准。
