# Proto V2 Replay Memory Benchmark Design

> 状态：设计已归档，尚未实现。本文不产生任何性能或内存结论。

## 目标

在真实 canonical replay 下，分别测量 Proto V2 与 SQLite range-table reader 的：

- process-cold 首次读取；
- warmup 后的 session P50/P95；
- 应用层缓存命中、淘汰和峰值占用；
- worker RSS 增量与稳定态峰值。

每个 replay request 固定包含 `dimension`、canonical `concrete_line_id` 和 `hole_cards`。计时前必须校验 Proto 与 SQLite 返回的 action、size、amount、frequency、EV 全量一致。

## 对照 Profile

| Profile | 缓存策略 | 用途 |
| --- | --- | --- |
| `sqlite-direct-prepared` | 长连接与 dimension 级 prepared statement cache；不缓存业务结果 | 当前 SQLite reader 的正式基线 |
| `sqlite-matrix-lru` | 在 `sqlite-direct-prepared` 上按 `dimension + concrete_line_id` 缓存完整矩阵 | 与 Proto decoded-matrix cache 的等价对照 |
| `proto-cache-off` | 不保留 decoded matrix | 隔离 `.lmidx + .lmbin + Protobuf decode` 成本 |
| `proto-matrix-lru` | decoded matrix LRU | 测量真实 replay 的最小有效 cache |

`sqlite-matrix-lru` 是敏感性对照，不替代 `sqlite-direct-prepared`。它回答“相同业务对象缓存下，文件布局是否仍有收益”，而不是把 SQLite 基线改造成另一种产品实现。

## 执行规则

1. 每个 engine/profile 使用独立 fresh process；每个配置至少重复 10-20 次。
2. 每次运行依次记录 `process start`、`reader open`、`warmup complete`、`timed replay complete` 四个阶段的内存快照。
3. warmup session 固定且不计时；随后缓存不清空地重复相同 replay，按 session 统计 P50/P95。
4. Proto 与 SQLite 使用同一请求顺序、同一过滤语义 `hand_ev IS NOT NULL`、同一错误处理和同一返回结果校验。
5. process-cold 仅刷新进程状态；若不能可靠清空 OS page cache，报告必须明确标为 `process-cold`，不得称为真实磁盘冷缓存。

## Cache 口径

每个 profile 必须输出：

- entries、resident bytes、peak bytes；
- hits、misses、evictions、revisit-after-eviction；
- 已打开 dimension 数及各 dimension 的 cache 占用；
- SQLite 的 prepared statement 数、`PRAGMA cache_size` 与 page size。

`sqlite-matrix-lru` 与 `proto-matrix-lru` 使用相同 key、entry capacity、byte budget 和 LRU 淘汰规则。byte 统计应按持有的业务矩阵对象估算，SQLite page cache 与 mmap resident pages 不混入应用层 cache bytes。

## RSS 口径

RSS 使用同一 worker 进程采样方式，在 Windows 上即 working set。报告至少列出：

- open delta RSS；
- warmup delta RSS；
- timed replay peak RSS；
- 总 delta RSS。

RSS 反映进程实际占用，受操作系统页回收影响；应用层 cache bytes 用于解释可控内存，两者均需保留，不能互相替代。

## 输出结论边界

最终报告必须分别回答：

1. `proto-cache-off` 相对 `sqlite-direct-prepared` 的读取路径收益；
2. 相同 LRU 容量下 Proto 与 SQLite 的缓存对象成本；
3. 真实 replay 的最小有效 Proto cache；
4. 多 dimension 混合 replay 的累计 RSS 是否满足服务部署预算。

在这些 profile 实现并通过全量结果校验前，现有 benchmark 不得用于宣称 Proto 比 SQLite 更快或更省内存。
