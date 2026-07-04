# 存储架构调研报告

更新日期：2026-07-01

## 结论

当前业务建议采用“瘦身后的 SQLite 元数据 + `.idx/.bin` 二进制策略数据 + Rust core 复用层”的混合方案。运行入口可以是 Rust 独立 HTTP 服务，也可以是 Bun/TypeScript 后端进程内 native SDK。

这个方案的核心判断是：

- 源 SQLite `range.db` 继续作为离线构建输入和验证基准，不进入线上容器热路径。
- 线上运行目录只包含 `manifest.json`、`meta.db`、每个维度一组 `.idx/.bin` 文件。
- 热路径查询避开 SQL range 表扫描和大量行对象构造，通过 `.idx` 定位 pack，再从 `.bin` 解码目标手牌。
- `meta.db` 保留 drill scenario、concrete line、action schema 这类强元数据能力，避免把所有业务元数据也手写成自定义二进制格式。
- Rust 负责存储读取、格式校验、HTTP 服务、native SDK 和离线工具，减少跨语言格式实现漂移。

第一阶段调研报告只给架构估算和风险判断，不记录具体冷启动耗时。冷启动数字依赖数据版本、Docker 资源、OS page cache、是否 prewarm、benchmark 模式和查询样本，应放在 benchmark 或部署验收报告中。

## 业务数据特征

当前 range 数据有几个明显特点：

- 数据以版本化静态文件为主，线上服务以读为主。
- 查询维度固定在 `strategy + player_count + depth_bb`。
- 高频查询是 `concrete_line_id + hole_cards` 到 action strategy 的读取。
- 元数据查询包括 drill scenario 到 abstract line、abstract line 到 concrete line。
- action 组合在不同 concrete line 之间有大量重复，适合抽出 `action_schemas` 复用。
- `frequency` 和 `hand_ev` 可以按 Float32 存储，前提是验证标准按 Float32 bit-exact 处理。

这些特征决定了它不适合把完整源 SQLite 原封不动放在线上热路径，也不适合把所有元数据都压成纯二进制后失去可查性。

## 方案对比

| 方案 | 说明 | 优点 | 缺点 | 适用性 |
| --- | --- | --- | --- | --- |
| 完整 SQLite | 容器直接读取原始 `range.db`，所有 range 和元数据都走 SQL | 实现简单，SQLite B-tree 成熟，运维认知成本低 | 运行体积大，热路径绑定 SQL 表结构，业务响应需要做大量行到对象转换 | 可作为基准和回退方案，不建议作为当前主路径 |
| SQLite 瘦身 | 保留 SQLite，但删除无关表或压缩部分列 | 改造风险小，元数据能力完整 | 只靠 SQLite 瘦身很难消除 range 表体积和 SQL 热路径成本 | 可作为过渡方案，但收益有限 |
| 纯二进制 | 所有数据，包括元数据、索引、策略，全部自定义二进制 | 理论体积最小，热路径最直接，部署文件可完全脱离 SQLite | 元数据查询、schema 演进、调试和验证成本高，工具链复杂 | 不建议当前一次性采用 |
| 混合方案 | `meta.db` 存元数据，`.idx/.bin` 存策略热数据 | 体积明显下降，热路径简单，元数据仍可 SQL 查询，验证可分层 | 仍需动态加载 SQLite 读取 `meta.db`，存在两种文件格式 | 推荐 |

## SQLite 瘦身判断

SQLite 瘦身的价值主要在于把它从“完整业务数据源”降级为“运行时元数据目录”。

保留在 `meta.db` 中的数据：

- `build_info`：构建时间和源库 checksum。
- `action_schemas`：action 组合定义和校验。
- `dimension_action_schemas`：维度和 action schema 的引用关系。
- `drill_scenario_lines_{strategy}`：drill scenario 到 abstract line。
- `concrete_lines_{strategy}_{player_count}max_{depth_bb}BB`：abstract line 到 concrete line。

从运行时 SQLite 中移出的数据：

- `range_data_*` 中每个手牌、action、frequency、hand_ev 的明细。
- pack 索引类热路径数据。

这样做后，SQLite 仍负责适合它的关系型元数据查询，二进制文件负责高频策略读取。

## 纯二进制方案判断

纯二进制方案看起来更彻底，但当前不优先采用，主要原因是元数据并不只是简单 key-value。

如果把 drill scenario、concrete line、action schema 都放进自定义二进制，需要额外解决：

- 字符串索引和排序规则。
- 多条件过滤，比如 `drill_name + player_count + drill_depth`。
- 未来新增 strategy 或维度时的兼容策略。
- 运维排查时无法用 SQLite 工具直接查看元数据。
- 验证工具必须覆盖更多自定义格式，失败定位成本上升。

当前业务的体积压力主要来自 range 明细，不来自元数据本身。因此把热数据二进制化，把元数据留在瘦身 SQLite，是更稳的工程折中。

## 混合方案判断

混合方案的读取路径是：

```text
请求参数
  -> 维度定位
  -> .idx dense 下标定位 concrete_line_id
  -> .bin 按 offset/length 读取 range pack
  -> meta.db action_schemas 解释 action_id
  -> 返回业务结构
```

元数据路径是：

```text
drill_name / abstract_line
  -> meta.db 查询
  -> 返回 abstract_lines 或 concrete_lines
```

这个边界比较清楚：

- `.idx/.bin` 是不可变、按维度拆分、服务热路径读取的策略数据。
- `meta.db` 是可 SQL 查询的元数据目录。
- `manifest.json` 是运行目录的入口和版本清单。
- 源 SQLite 是构建输入和 cross verify 基准，不是线上运行依赖。

## 编程语言选择

| 语言或形态 | 优点 | 问题 | 结论 |
| --- | --- | --- | --- |
| TypeScript/Bun 服务 | 与原项目一致，迭代快 | 热路径对象构造和 JS/Native 边界成本明显，二进制格式校验和 mmap 安全边界更难收束 | 不作为独立服务主实现 |
| Rust N-API 插件 | 可保留 Bun/TypeScript 业务进程，同时把热路径下沉 Rust | 需要维护 native addon 构建产物和 JS/Native 边界 | Bun 后端进程内访问的推荐形态 |
| Rust 独立服务 | 内存安全、mmap 和字节解析能力强，单进程 HTTP 服务，Docker 部署清晰，测试和工具可共用 core | 需要维护 Rust HTTP 和工具链，业务后端访问时仍有 HTTP 往返 | 已实现，适合作为独立部署、调试和兼容路径 |
| Go 独立服务 | 部署简单，HTTP 成熟 | 自定义二进制解析、mmap、Float32 bit-exact 验证和已有 Rust core 复用不如 Rust 直接 | 不优先 |
| C/C++ 服务 | 性能和底层控制力强 | 安全和维护成本高，业务迭代风险大 | 不采用 |

Rust 的优势不只是性能，还包括：

- `range-store-core` 可以同时服务 API、验证、benchmark 和构建工具。
- `.idx/.bin` 解析、CRC32C、Float32 bit-exact 校验可以集中在一套实现里。
- HTTP service 镜像只需要 service 和 core，`storage-tools` 不进入运行镜像；Bun 后端形态则需要携带 `range-store-native` 产物。
- 运行时错误可以统一映射为 HTTP 状态码和业务错误码。

## 业务接入与 SDK 边界

当前已实现形态是 Docker 部署后的 HTTP API：`poker-hands-storage-service` 作为独立只读查询服务，业务系统通过 `/range/*` API 访问。

如果业务后端是 Bun/TypeScript，并且希望像 SQLite 一样在业务进程内直接访问只读 RangeDB，则推荐新增进程内 native SDK，而不是继续新增 HTTP SDK。详细草案见 `bun-native-sdk-implementation-draft.md`。

两种接入形态的定位如下：

| 接入形态 | 定位 | 是否推荐作为 Bun 后端主路径 |
| --- | --- | --- |
| HTTP API | 已实现，可独立部署，适合 Swagger 调试、服务化部署、兼容验证 | 可保留，但不是进程内访问的最优路径 |
| HTTP SDK | 仅封装 HTTP 请求、超时、重试、错误映射 | 不优先，因为仍然依赖独立查询服务 |
| Native SDK | Bun 进程内加载 Rust `.node` addon，直接复用 `range-store-core` 读取只读 RangeDB | 推荐 |

因此，“查询 SDK / 查询接口”的新口径是：

- 本项目继续保留 HTTP 查询接口。
- 若业务后端选择进程内访问，应在本项目新增 `range-store-native`，提供 Bun/Node 可调用的 native SDK。
- native SDK 不重新实现二进制解析，只封装 `range-store-core`。
- `service`、`storage-tools`、`range-store-native` 三者保持平级，且只依赖 `range-store-core`。

多语言 SDK 暂不作为当前目标。只有多个独立系统长期接入、需要正式发布 TypeScript/Java/Python 客户端包，并有版本兼容维护能力时，才考虑额外建设独立客户端 SDK。

## 冷启动估算口径

本报告不写具体冷启动耗时，只记录架构层面的影响方向。

| 方案 | 冷启动影响估算 | 说明 |
| --- | --- | --- |
| 完整 SQLite | 低到中等 | 打开 DB 和首次查询成本较稳定，但完整 DB 文件体积大 |
| 纯二进制 | 可低也可高 | 如果只 mmap 文件，启动轻；如果 ready 前全量预热，内存和 ready 时间会上升 |
| 混合方案 | 中等且可控 | 启动读取 manifest/meta/action schema，并可选择性 prewarm 维度 |

需要注意：

- mmap 打开文件不等于把整个 `.bin` 文件读入物理内存。
- 真实 page fault 成本通常发生在首次访问对应文件页时。
- 把高成本放在容器 ready 之前可以保护用户请求，但会增加 ready 时间和部分常驻资源。
- `PHS_PREWARM` 应按生产流量选择少量热点维度，不建议盲目全量预热。

## 推荐落地策略

1. 继续保留源 SQLite 作为离线构建输入。
2. 线上只挂载构建后的 Range Strata 目录。
3. 查询热路径走 `.idx/.bin`，元数据查询走 `meta.db`。
4. 发布前必须跑 standalone verify 和 source cross verify。
5. Docker/Kubernetes 使用只读数据挂载和 readiness probe。
6. benchmark 数字单独维护，记录数据集、环境、命令和时间，不写进选型调研的固定结论中。
