# API 业务逻辑和接口契约

更新日期：2026-07-16

## 总体说明

服务是只读 HTTP API，默认读取 Proto V3 根目录中的维度 manifest 和三组 `.pb/.idx` 文件。运行时不打开 SQLite，也不读取 V2。HTTP 框架为 axum，OpenAPI 文档由 utoipa 生成。

本文只维护 HTTP service 的请求、响应、业务错误码和业务语义。Bun/Node native SDK 的直接 payload 与 `RangeStoreError` 契约见 `sdk-and-query-chain-explanation.md`。

运行中可访问：

```text
GET /swagger
GET /api-docs/openapi.json
```

## 响应包络

成功响应统一为：

```json
{
  "code": 0,
  "data": {},
  "message": null
}
```

错误响应统一为：

```json
{
  "code": 1000,
  "data": null,
  "message": "request validation failed: concrete_line_id must be greater than 0"
}
```

说明：

- `code = 0` 表示业务成功。
- 错误响应的 `code` 是公开业务错误码。
- 当前响应体不单独返回内部 `AppError.code`，内部错误码只用于映射 HTTP 状态和公开业务码。

## HTTP 状态码和业务错误码

| HTTP 状态 | 业务 `code` | 场景 |
| ---: | ---: | --- |
| 200 | 0 | 请求成功 |
| 400 | 1000 | JSON 解析失败、参数校验失败、未知手牌、非法参数 |
| 404 | 404 | 维度、文件、concrete line、drill scenario、手牌策略或筛选结果不存在 |
| 503 | 503 | 服务未 ready |
| 500 | 500 | V3 manifest、索引或 payload 损坏，以及未归类内部异常 |

内部错误到公开业务码的映射：

| 内部错误 | 公开业务码 |
| --- | ---: |
| `INVALID_ARGUMENT` | 1000 |
| `BIN_FILE_NOT_FOUND` | 404 |
| `PACK_NOT_FOUND` | 404 |
| `DATA_FILE_NOT_FOUND` | 404 |
| `DRILL_SCENARIO_NOT_FOUND` | 404 |
| `ABSTRACT_LINE_NOT_FOUND` | 404 |
| `DIMENSION_NOT_FOUND` | 404 |
| `ACTION_SCHEMA_NOT_FOUND` | 404 |
| `CONCRETE_LINE_NOT_FOUND` | 404 |
| `HAND_STRATEGY_NOT_FOUND` | 404 |
| `ACTION_NOT_FOUND` | 404 |
| `HANDS_NOT_FOUND` | 404 |
| `SERVICE_UNAVAILABLE` | 503 |
| `INVALID_FORMAT` | 500 |
| 其他错误 | 500 |

## 通用参数规则

当前数据集支持：

| 字段 | 默认值 | 允许值 |
| --- | --- | --- |
| `strategy` | `default` | `default` |
| `player_count` | `6` | `6, 8, 9` |
| `depth_bb` | `100` | `100, 200, 300` |
| `drill_depth` | `100` | `100, 200, 300` |

通用校验：

- `concrete_line_id` 必须大于 0。
- `/range/concrete-lines` 的 `abstract_line` 和 `concrete_line` 至少传一个；字段缺失、`null` 或空字符串都不是有效查询条件。
- `hole_cards` 不能为空，支持 169 手牌编码和两张具体牌编码，例如 `AA`、`AKs`、`AKo`、`AsKh`。
- batch 请求最多 500 条。
- prewarm 请求最多 64 个维度。

## `GET /health`

业务含义：进程存活检查。

响应：

```json
{
  "code": 0,
  "data": {
    "status": "ok",
    "uptime_secs": 12.345
  },
  "message": null
}
```

状态码：

| HTTP | 说明 |
| ---: | --- |
| 200 | HTTP 进程可响应 |

## `GET /ready`

业务含义：数据目录是否可用于 range 查询。

ready 条件是服务已从 `manifest.json` 加载到至少一个可查询维度。接口不要求所有维度都已经 prewarm，也不要求 action schema 已加载。

成功响应：

```json
{
  "code": 0,
  "data": {
    "status": "ready",
    "schema_count": 0,
    "handles_open": 0,
    "dimensions_known": [
      "default_6max_100BB"
    ]
  },
  "message": null
}
```

状态码：

| HTTP | 业务码 | 说明 |
| ---: | ---: | --- |
| 200 | 0 | 服务 ready |
| 503 | 503 | 没有加载到可查询维度 |

## `POST /range/drill-scenarios`

业务含义：根据 drill scenario 查询可用 abstract lines。

请求体：

```json
{
  "strategy": "default",
  "drill_name": "rfi",
  "player_count": 6,
  "drill_depth": 100
}
```

字段：

| 字段 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `strategy` | 否 | `default` | 当前只允许 `default` |
| `drill_name` | 否 | `rfi` | 必须以字母开头，不能是纯数字，只能包含字母、数字、`_`、`-` |
| `player_count` | 否 | `6` | 允许 `6, 8, 9` |
| `drill_depth` | 否 | `100` | 允许 `100, 200, 300` |

响应：

```json
{
  "code": 0,
  "data": {
    "abstract_lines": ["F-F-F"]
  },
  "message": null
}
```

### 组合调用示例：line-transition 查询

完整行动线通常由业务侧逐步拼接和解释。以 6 人桌、100BB、2 人对战为例：

```text
F-F-F-R2-F-R7-R15
```

该行动线可解释为 `BB vs BTN 4bet` 节点。业务侧需要拆成两个查询节点：

| 节点 | 行动线 | 查询目标 |
| --- | --- | --- |
| 前序节点 | `F-F-F-R2-F-R7` | BTN 的手牌范围 |
| 当前节点 | `F-F-F-R2-F-R7-R15` | BB 的手牌范围和 BB 当前 actions |

推荐调用流程：

1. 业务侧根据完整行动线和位置映射规则解析当前行动者、前序范围所属玩家和下注尺度。
2. 通过 `/range/concrete-lines` 的 `concrete_line` 精确查询或业务后端本地映射得到前序行动线的 `concrete_line_id`。
3. 通过 `/range/hands-by-actions` 查询前序行动线中 BTN 的手牌范围。
4. 通过 `/range/concrete-lines` 的 `concrete_line` 精确查询或业务后端本地映射得到完整行动线的 `concrete_line_id`。
5. 通过 `/range/hands-by-actions` 查询完整行动线中 BB 的手牌范围。
6. 如需某个具体手牌在当前节点的策略，用 `/range/hand-strategy` 查询该 `concrete_line_id + hole_cards` 的 actions。

该访问模式不是“同一 `abstract_line` 下 concrete ids 轮转”。当前 `benchmark-native` 已覆盖单条 `concrete_line -> concrete_line_id -> handsByActions` 链路；完整业务 `line-transition` 仍需补 prefix/full 双节点组合 workload。

错误：

| HTTP | 业务码 | 场景 |
| ---: | ---: | --- |
| 400 | 1000 | 参数非法 |
| 404 | 404 | 没有找到该 drill scenario 的 abstract lines |
| 500 | 500 | V3 metadata page 或索引损坏 |

## `POST /range/concrete-lines`

业务含义：根据 abstract line 查询 concrete lines，或根据 concrete line 精确查询对应的 `concrete_line_id`。

按 abstract line 查询：

```json
{
  "strategy": "default",
  "player_count": 6,
  "depth_bb": 100,
  "abstract_line": "F-F-F"
}
```

按 concrete line 精确查询：

```json
{
  "strategy": "default",
  "player_count": 6,
  "depth_bb": 100,
  "concrete_line": "F-F-F-R2-F-R7-R15"
}
```

字段规则：

| 字段 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- |
| `abstract_line` | 否 | 无 | 抽象行动线；和 `concrete_line` 至少传一个 |
| `concrete_line` | 否 | 无 | 具体行动线；用于精确查询 `concrete_line_id` |

说明：

- `abstract_line` 和 `concrete_line` 都不传时返回 400。
- 传空字符串时返回 400。
- 传 `null` 时返回 400；`null` 不等价于未传。
- 两个字段同时传入时按两个条件同时匹配。

响应：

```json
{
  "code": 0,
  "data": {
    "lines": [
      {
        "concrete_line_id": 1,
        "abstract_line": "F-F-F",
        "concrete_line": "F-F-F"
      }
    ]
  },
  "message": null
}
```

错误：

| HTTP | 业务码 | 场景 |
| ---: | ---: | --- |
| 400 | 1000 | 参数非法；`abstract_line` 和 `concrete_line` 都未传；字段为空字符串或 `null` |
| 404 | 404 | 该 abstract line 没有 concrete lines，或 concrete line 不存在 |
| 500 | 500 | V3 metadata page 或索引损坏 |

## `POST /range/hand-strategy`

业务含义：查询一个 concrete line 下某个手牌的 action strategy。

请求体：

```json
{
  "strategy": "default",
  "player_count": 6,
  "depth_bb": 100,
  "concrete_line_id": 1,
  "hole_cards": "AA"
}
```

业务流程：

1. 校验维度和必填字段。
2. 解析 `hole_cards` 为固定 169 手牌 `hand_id`。
3. 通过 `hand-strategies.idx` 直接定位 V3 ID 对应的 payload。
4. 从 `hand-strategies.pb` 解码目标 hand 的 action cells。
5. action identity 已在 `HandStrategy` payload 中，不需要 metadata SQLite。

响应：

```json
{
  "code": 0,
  "data": {
    "actions": [
      {
        "action_name": "raise",
        "action_size": 2.5,
        "amount_bb": 2.5,
        "frequency": 0.75,
        "hand_ev": 1.0
      }
    ]
  },
  "message": null
}
```

错误：

| HTTP | 业务码 | 场景 |
| ---: | ---: | --- |
| 400 | 1000 | 未知手牌或参数非法 |
| 404 | 404 | 维度、concrete line 或该手牌策略不存在 |
| 500 | 500 | 文件格式或元数据异常 |

## `POST /range/hand-strategy-batch`

业务含义：在同一个维度中批量查询多个 hand strategy。

请求体：

```json
{
  "strategy": "default",
  "player_count": 6,
  "depth_bb": 100,
  "requests": [
    {
      "concrete_line_id": 1,
      "hole_cards": "AA"
    }
  ]
}
```

规则：

- `requests` 必须至少 1 条。
- `requests` 最多 500 条。
- 当前 batch 是 all-or-nothing：任一 item 失败，整个 HTTP 请求失败。
- 失败 item 不会写入 `results[].error`；错误通过统一错误响应返回。
- 错误 message 会包含失败 item 的索引，例如 `Batch item requests[1] failed`。

响应：

```json
{
  "code": 0,
  "data": {
    "results": [
      {
        "concrete_line_id": 1,
        "hole_cards": "AA",
        "actions": [
          {
            "action_name": "raise",
            "action_size": 2.5,
            "amount_bb": 2.5,
            "frequency": 0.75,
            "hand_ev": 1.0
          }
        ]
      }
    ]
  },
  "message": null
}
```

失败示例：

```json
{
  "code": 404,
  "data": null,
  "message": "Batch item requests[1] failed: Concrete line not found: concrete_line_id=999999, dimension=default:6:100 from concrete_line_id=999999, dimension=default:6:100"
}
```

错误：

| HTTP | 业务码 | 场景 |
| ---: | ---: | --- |
| 400 | 1000 | JSON 或参数校验失败；任一 item 的 `hole_cards` 非法 |
| 404 | 404 | 维度、concrete line 或任一 item 的手牌策略不存在 |
| 500 | 500 | 文件格式或元数据异常 |

## `POST /range/hands-by-actions`

业务含义：查询一个 concrete line 中满足 action 条件和 frequency 条件的手牌集合。

请求体：

```json
{
  "strategy": "default",
  "player_count": 6,
  "depth_bb": 100,
  "concrete_line_id": 1,
  "actions": ["fold", "raise"],
  "frequency": 0.005
}
```

字段规则：

| 字段 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- |
| `actions` | 否 | 空 | 空表示不限制 action name，但仍要求至少一个有效 action 超过频率阈值 |
| `frequency` | 否 | `0.005` | 未传表示过滤 `frequency > 0.005` |

frequency 语义：

- 不传 `frequency`：只返回存在且 `frequency > 0.005` 的手牌。
- 传 `frequency = x`：返回存在且 `frequency > x` 的手牌。
- Swagger 默认值展示为 `0.005`；当前语义下“未传”和“传 0.005”都表示 `frequency > 0.005`。

action 语义：

- 支持 `fold`、`call`、`check`、`bet`、`raise`、`allin`。
- `bet`、`raise`、`allin` 支持数值后缀，例如 `raise2.5`，表示精确匹配 amount。
- 传多个 action 时按 SQL `IN (...)` / OR 语义取并集：手牌只要任意一个 action filter 满足频率条件即可返回。
- 如果传入的某个 action 在当前 action schema 中不存在，但其他 action 可以命中，则仍返回其他 action 命中的手牌。

响应：

```json
{
  "code": 0,
  "data": {
    "hands": ["AA", "AKs"]
  },
  "message": null
}
```

错误：

| HTTP | 业务码 | 场景 |
| ---: | ---: | --- |
| 400 | 1000 | action 字符串非法，frequency 不在 0..1 |
| 404 | 404 | concrete line 不存在，或没有手牌满足筛选条件 |
| 500 | 500 | 文件或元数据异常 |

## `POST /range/prewarm`

业务含义：提前打开并校验指定维度的 `.idx/.bin` reader，放入 handle pool。

请求体：

```json
{
  "dimensions": [
    {
      "strategy": "default",
      "player_count": 6,
      "depth_bb": 100
    }
  ]
}
```

规则：

- `dimensions` 必须至少 1 个。
- 最多 64 个。
- 每个维度必须满足通用维度规则。
- prewarm 只打开并校验指定维度的 `.idx/.bin` reader，放入 handle pool。
- action schema 仍按真实查询命中的 `action_schema_id` 懒加载。

响应：

```json
{
  "code": 0,
  "data": {
    "prewarmed": 1,
    "total_open": 1
  },
  "message": null
}
```

错误：

| HTTP | 业务码 | 场景 |
| ---: | ---: | --- |
| 400 | 1000 | 请求参数非法 |
| 404 | 404 | 未知维度或维度文件不存在 |
| 500 | 500 | 格式校验或元数据异常 |

## 与 readiness 的关系

`/ready` 只表示服务已经打开数据目录并知道可查询维度，不表示所有维度都已 prewarm。

生产环境建议：

- 只把少量热点维度配置进 `PHS_PREWARM`。
- 不要为了让 `/ready` 成功而全量打开所有维度。
- 对非热点维度接受首次请求触发按需打开，或通过后台流量预热。
- action schema 按查询命中的 `action_schema_id` 懒加载；`/range/prewarm` 只打开维度 reader，不主动加载全部 schema。
