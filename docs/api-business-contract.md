# API 业务逻辑和接口契约

更新日期：2026-06-28

## 总体说明

服务是只读 HTTP API，读取 Range Strata 运行目录中的 `manifest.json`、`meta.db`、`.idx/.bin` 文件。HTTP 框架为 axum，OpenAPI 文档由 utoipa 生成。

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
| 500 | 500 | 格式损坏、元数据库错误、内部异常等未归类错误 |

内部错误到公开业务码的映射：

| 内部错误 | 公开业务码 |
| --- | ---: |
| `UNKNOWN_HAND` | 1000 |
| `INVALID_ARGUMENT` | 1000 |
| `BIN_FILE_NOT_FOUND` | 404 |
| `PACK_NOT_FOUND` | 404 |
| `DATA_FILE_NOT_FOUND` | 404 |
| `DRILL_SCENARIO_NOT_FOUND` | 404 |
| `ABSTRACT_LINE_NOT_FOUND` | 404 |
| `DIMENSION_NOT_FOUND` | 404 |
| `CONCRETE_LINE_NOT_FOUND` | 404 |
| `HAND_STRATEGY_NOT_FOUND` | 404 |
| `ACTION_NOT_FOUND` | 404 |
| `HANDS_NOT_FOUND` | 404 |
| `SERVICE_UNAVAILABLE` | 503 |
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

ready 条件是服务已加载到至少一个可查询维度。接口不要求所有维度都已经 prewarm。

成功响应：

```json
{
  "code": 0,
  "data": {
    "status": "ready",
    "schema_count": 19404,
    "handles_open": 1,
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

错误：

| HTTP | 业务码 | 场景 |
| ---: | ---: | --- |
| 400 | 1000 | 参数非法 |
| 404 | 404 | 没有找到该 drill scenario 的 abstract lines |
| 500 | 500 | `meta.db` 读取异常 |

## `POST /range/concrete-lines`

业务含义：根据 abstract line 查询 concrete lines。

请求体：

```json
{
  "strategy": "default",
  "player_count": 6,
  "depth_bb": 100,
  "abstract_line": "F-F-F"
}
```

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
| 400 | 1000 | 参数非法或 `abstract_line` 为空 |
| 404 | 404 | 该 abstract line 没有 concrete lines |
| 500 | 500 | `meta.db` 读取异常 |

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
3. 通过 `.idx` 查找 `concrete_line_id`。
4. 从 `.bin` 解码目标 hand 的 action cells。
5. 使用 `meta.db.action_schemas` 把 action id 转成业务 action 字段。

响应：

```json
{
  "code": 0,
  "data": {
    "input_hole_cards": "AA",
    "hand_code": "AA",
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
- 维度打开失败会作为整个请求错误返回。
- 单个 item 的未知手牌、concrete line 不存在、手牌不在该 line 范围内，会写入该 item 的 `error`，HTTP 仍为 200。

响应：

```json
{
  "code": 0,
  "data": {
    "results": [
      {
        "concrete_line_id": 1,
        "input_hole_cards": "AA",
        "hand_code": "AA",
        "strategy": {
          "actions": []
        },
        "error": null
      }
    ]
  },
  "message": null
}
```

单项失败示例：

```json
{
  "concrete_line_id": 999999,
  "input_hole_cards": "AA",
  "hand_code": "AA",
  "strategy": null,
  "error": {
    "code": 404,
    "message": "Concrete line not found: concrete_line_id=999999, dimension=default:6:100"
  }
}
```

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
  "frequency": 0
}
```

字段规则：

| 字段 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- |
| `actions` | 否 | 空 | 空表示不限制 action name |
| `frequency` | 否 | 未传 | 未传表示过滤 `frequency > 0` |

frequency 语义：

- 不传 `frequency`：只返回存在且 `frequency > 0` 的手牌。
- 传 `frequency = x`：返回存在且 `frequency >= x` 的手牌。
- Swagger 默认值展示为 `0.0`，但业务上“未传”和“传 0”不同。

action 语义：

- 支持 `fold`、`call`、`check`、`bet`、`raise`、`allin`。
- `bet`、`raise`、`allin` 支持数值后缀，例如 `raise2.5`，表示精确匹配 amount。
- 传多个 action 时取交集：手牌必须同时满足所有传入 action filter。
- 可以有其他额外 action，只要包含请求的所有 action 条件即可。

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
- prewarm 会校验该维度 `.idx` 引用的 action schema 集合和 `dimension_action_schemas` 是否一致。

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
