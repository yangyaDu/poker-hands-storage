# API Boundary Conditions Design

This document defines the boundary-condition contract for the HTTP query API.
The service is a standalone business query service, so a syntactically valid
request that cannot resolve the requested business object should return an
explicit business error instead of a successful empty result.

## Response Envelope

All endpoints use the same envelope.

Success:

```json
{
  "code": 0,
  "data": {},
  "message": null
}
```

Error:

```json
{
  "code": 404,
  "data": null,
  "message": "No abstract lines found for drill: strategy={strategy}, drill_name={drill_name}, player_count={player_count}, drill_depth={drill_depth}"
}
```

Rules:

- Success responses must use `code = 0`.
- Success responses must put the endpoint payload in `data`.
- Success responses must use `message = null`.
- Error responses must use the public `code` defined in the catalog below.
- Error responses must use `data = null`.
- Error responses must put the user-facing reason in `message`.
- Error responses must not include a `details` field.
- HTTP status and public `code` must not conflict. Parameter validation failures
  use `HTTP 400` with public `code = 1000`; do not invent an HTTP 1000 status.

## Error Code Catalog

| HTTP | Code | Meaning |
| --- | ---: | --- |
| 200 | 0 | Success. |
| 400 | 1000 | Invalid JSON, request parameter validation failure, unknown hand, invalid action filter, or value out of the documented enum/range. |
| 404 | 404 | Requested business resource or required data file was not found. |
| 500 | 500 | Binary store read/decode/corruption error or unexpected internal invariant failure. Examples: invalid `.idx`/`.bin` magic, unsupported binary version, incompatible pack byte length, CRC32C mismatch when checksum verification is enabled, full-pack decode failure, or action schema/blob inconsistency. |
| 503 | 503 | Readiness failure for health/readiness endpoints only. |

Implementation note: internal errors may keep stable names such as
`UNKNOWN_HAND`, `DRILL_SCENARIO_NOT_FOUND`, or `ACTION_SCHEMA_NOT_FOUND` for
logging and routing, but public response `code` must follow this catalog.
Clients should use `message` for the specific business reason.

Do not use `500` for request validation, unsupported drill enum values, missing
dimensions, missing drills, missing concrete lines, or empty business query
results. Those are client/input or business-not-found outcomes and must map to
`1000` or `404`.

## Shared Request Rules

Dimension fields appear on most range endpoints.

| Field | Default | Validation |
| --- | --- | --- |
| `strategy` | `default` where the endpoint allows omission | Trimmed string, non-empty, table-name safe after existing naming validation. |
| `player_count` | `6` where the endpoint allows omission | Integer enum: `6`, `8`, or `9`. |
| `depth_bb` | `100` where the endpoint allows omission | Integer, `> 0`. |
| `concrete_line_id` | none | Integer, `> 0`. |
| `hole_cards` | none | Trimmed string, non-empty, parseable by hand dictionary. |

Request parameter validation failures must return `HTTP 400`, public
`code = 1000`, and a concrete `request validation failed: ...` message.

Validation message examples:

- `strategy must not be empty`
- `player_count must be one of 6, 8, 9`
- `depth_bb must be greater than 0`
- `concrete_line_id must be greater than 0`
- `hole_cards must not be empty`

Resource resolution order:

1. Validate JSON and request fields.
2. Resolve dimension if the endpoint depends on one.
3. Resolve concrete line or metadata object.
4. Resolve hand/action filters.
5. Execute query.
6. Treat missing business result as a 404 business error, not as an empty success.

Error messages must be formatted from the actual request parameters. Do not
hard-code the sample values shown in examples. Message templates use braces for
request-derived values:

- `Dimension not found: strategy={strategy}, player_count={player_count}, depth_bb={depth_bb}`
- `No abstract lines found for drill: strategy={strategy}, drill_name={drill_name}, player_count={player_count}, drill_depth={drill_depth}`
- `No concrete lines found for abstract_line={abstract_line} in dimension {strategy}:{player_count}:{depth_bb}`
- `Unknown hand: {hole_cards}`
- `Concrete line not found: concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}`
- `Hand {hole_cards} is outside the range for action line concrete_line_id={concrete_line_id} in dimension {strategy}:{player_count}:{depth_bb}`
- `No hands found for actions={actions} at frequency={frequency}, concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}`

## `POST /range/drill-scenarios`

Purpose: return abstract lines for a named drill scenario.

Request:

```json
{
  "drill_name": "RFI",
  "strategy": "default",
  "player_count": 6,
  "drill_depth": 200
}
```

Defaults:

| Field | Default |
| --- | --- |
| `strategy` | `default` |
| `player_count` | `6` |
| `drill_depth` | `100` |
| `drill_name` | none, required |

Allowed drill dimensions:

| Field | Allowed values |
| --- | --- |
| `player_count` | `6`, `8`, `9` |
| `drill_depth` | `100`, `200`, `300` |

`drill_name` validation:

- Required.
- Trimmed value must not be empty.
- Must not be a pure numeric string.
- Must start with an ASCII letter.
- May contain ASCII letters, digits, `_`, and `-`.
- Preserve value casing for lookup. Business values such as `RFI` and `UTG` must remain valid.

Invalid examples:

| Request issue | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| `drill_name` missing | 400 | 1000 | `request validation failed: drill_name must not be empty` |
| `drill_name = ""` | 400 | 1000 | `request validation failed: drill_name must not be empty` |
| `drill_name = "123"` | 400 | 1000 | `request validation failed: drill_name must start with a letter; drill_name must not be numeric-only` |
| `drill_name = "1RFI"` | 400 | 1000 | `request validation failed: drill_name must start with a letter` |
| `drill_name = "RFI!"` | 400 | 1000 | `request validation failed: drill_name may only contain letters, digits, underscore, or hyphen` |
| `player_count` is not `6`, `8`, or `9` | 400 | 1000 | `request validation failed: player_count must be one of 6, 8, 9` |
| `drill_depth` is not `100`, `200`, or `300` | 400 | 1000 | `request validation failed: drill_depth must be one of 100, 200, 300` |

Not found:

| Condition | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| Drill metadata table is absent or no rows match the request | 404 | 404 | `No abstract lines found for drill: strategy={strategy}, drill_name={drill_name}, player_count={player_count}, drill_depth={drill_depth}` |

Success:

```json
{
  "code": 0,
  "data": {
    "abstract_lines": ["F-F-F"]
  },
  "message": null
}
```

Empty `abstract_lines` must not be returned as success.

## `POST /range/concrete-lines`

Purpose: return concrete lines for one abstract action line in a dimension.

Request:

```json
{
  "strategy": "default",
  "player_count": 6,
  "depth_bb": 100,
  "abstract_line": "F-F-F"
}
```

Defaults:

| Field | Default |
| --- | --- |
| `strategy` | `default` |
| `player_count` | `6` |
| `depth_bb` | `100` |
| `abstract_line` | none, required |

Validation:

| Request issue | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| `abstract_line` missing | 400 | 1000 | `request validation failed: abstract_line must not be empty` |
| `abstract_line = ""` | 400 | 1000 | `request validation failed: abstract_line must not be empty` |
| `player_count` is not `6`, `8`, or `9` | 400 | 1000 | `request validation failed: player_count must be one of 6, 8, 9` |
| `depth_bb = 0` | 400 | 1000 | `request validation failed: depth_bb must be greater than 0` |

Not found:

| Condition | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| Concrete-line metadata table is absent or no rows match `abstract_line` | 404 | 404 | `No concrete lines found for abstract_line={abstract_line} in dimension {strategy}:{player_count}:{depth_bb}` |

Success:

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

Empty `lines` must not be returned as success.

## `POST /range/hand-strategy`

Purpose: return the strategy for a valid hand at a concrete line.

Request:

```json
{
  "strategy": "default",
  "player_count": 6,
  "depth_bb": 100,
  "concrete_line_id": 1,
  "hole_cards": "AA"
}
```

Defaults:

| Field | Default |
| --- | --- |
| `strategy` | `default` |
| `player_count` | `6` |
| `depth_bb` | `100` |
| `concrete_line_id` | none, required |
| `hole_cards` | none, required |

Validation:

| Request issue | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| `concrete_line_id = 0` | 400 | 1000 | `request validation failed: concrete_line_id must be greater than 0` |
| `hole_cards = ""` | 400 | 1000 | `request validation failed: hole_cards must not be empty` |
| `hole_cards = "AsXx"` | 400 | 1000 | `Unknown hand: {hole_cards}` |
| `strategy = ""` | 400 | 1000 | `request validation failed: strategy must not be empty` |

Not found:

| Condition | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| Dimension is unavailable or concrete line id does not exist | 404 | 404 | `Concrete line not found: concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}` |
| Hand is valid but is outside the requested action-line range | 404 | 404 | `Hand {hole_cards} is outside the range for action line concrete_line_id={concrete_line_id} in dimension {strategy}:{player_count}:{depth_bb}` |

Success:

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

`exists: false` should be removed from the public success contract or kept only
for a deprecated compatibility mode. A business lookup that does not exist
should be an error.

## `POST /range/hand-strategy-batch`

Purpose: query many `(concrete_line_id, hole_cards)` items in one dimension.

Request:

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

Defaults:

| Field | Default |
| --- | --- |
| `strategy` | `default` |
| `player_count` | `6` |
| `depth_bb` | `100` |
| `requests` | none, required |

Whole-request validation:

| Request issue | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| `requests` missing | 400 | 1000 | `request validation failed: requests must contain at least one item` |
| `requests = []` | 400 | 1000 | `request validation failed: requests must contain at least one item` |
| `requests.len() > 500` | 400 | 1000 | `request validation failed: requests must contain at most 500 items` |
| `requests[0].concrete_line_id = 0` | 400 | 1000 | `request validation failed: requests[0].concrete_line_id must be greater than 0` |
| `requests[0].hole_cards = ""` | 400 | 1000 | `request validation failed: requests[0].hole_cards must not be empty` |
| Dimension is unavailable | 404 | 404 | `Dimension not found: strategy={strategy}, player_count={player_count}, depth_bb={depth_bb}` |

Per-item failures:

Batch item failures should stay inside `data.results[*].error` when the overall
request is valid and the dimension exists. This lets clients receive partial
results without retrying the whole batch.

| Per-item condition | Item error code | Item message |
| --- | --- | --- |
| Unknown hand | `1000` | `Unknown hand: {hole_cards}` |
| Concrete line not found | `404` | `Concrete line not found: concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}` |
| Hand is outside the requested action-line range | `404` | `Hand {hole_cards} is outside the range for action line concrete_line_id={concrete_line_id} in dimension {strategy}:{player_count}:{depth_bb}` |

Success with partial item errors:

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
      },
      {
        "concrete_line_id": 999,
        "input_hole_cards": "AA",
        "hand_code": "AA",
        "strategy": null,
        "error": {
          "code": 404,
          "message": "Concrete line not found: concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}"
        }
      }
    ]
  },
  "message": null
}
```

## `POST /range/hands-by-actions`

Purpose: return the hand list that matches action filters for a concrete line.
This endpoint is a hand-list query, not an action-grouping response.

Request:

```json
{
  "strategy": "default",
  "player_count": 6,
  "depth_bb": 100,
  "concrete_line_id": 1,
  "actions": ["raise2.5"],
  "frequency": 0.05
}
```

Defaults:

| Field | Default |
| --- | --- |
| `strategy` | `default` |
| `player_count` | `6` |
| `depth_bb` | `100` |
| `actions` | omitted or `[]` means all hands in the concrete line |
| `frequency` | `0.0` |

Action filter syntax:

| Filter form | Meaning |
| --- | --- |
| `fold` | Match hands with a fold action. |
| `call` | Match hands with a call action. |
| `check` | Match hands with a check action. |
| `bet{amountBB}` | Match hands with a bet action at the requested amount in BB, for example `bet3`. |
| `raise{amountBB}` | Match hands with a raise action at the requested amount in BB, for example `raise2.5`. |
| `allin{amountBB}` | Match hands with an all-in action at the requested amount in BB, for example `allin100`. |

When multiple filters are provided, the hand must satisfy all filters after the
`frequency` threshold is applied. If `actions` is omitted or is an empty array,
return all hands present in the concrete line after applying `frequency`.

Validation:

| Request issue | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| `concrete_line_id = 0` | 400 | 1000 | `request validation failed: concrete_line_id must be greater than 0` |
| `actions[{index}]` is empty or does not match the supported filter syntax | 400 | 1000 | `request validation failed: actions[{index}] must match one of fold, call, check, bet{amountBB}, raise{amountBB}, allin{amountBB}` |
| `frequency` is outside `0..=1` | 400 | 1000 | `request validation failed: frequency must be between 0 and 1` |

Not found:

| Condition | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| Dimension is unavailable or concrete line does not exist | 404 | 404 | `Concrete line not found: concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}` |
| Requested action is absent from the schema or no hands meet the action/frequency filters | 404 | 404 | `No hands found for actions={actions} at frequency={frequency}, concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}` |

Success:

```json
{
  "code": 0,
  "data": {
    "hands": ["AA", "AKs"]
  },
  "message": null
}
```

The response intentionally omits `concrete_line_id`, `action_name`,
`action_size`, `amount_bb`, and `frequency`; those values belong to the request
and filtering rules, while this endpoint's business payload is the matching
hand list. Empty `hands` should not be returned as success when filters are
provided and no hand matches. If `actions` is omitted or `[]` and the concrete
line exists, return all hands in the line.

## `POST /range/prewarm`

Purpose: open and validate one or more dimensions in the handle pool.

Request:

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

Validation:

| Request issue | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| `dimensions` missing | 400 | 1000 | `request validation failed: dimensions must contain at least one item` |
| `dimensions = []` | 400 | 1000 | `request validation failed: dimensions must contain at least one item` |
| `dimensions.len() > 64` | 400 | 1000 | `request validation failed: dimensions must contain at most 64 items` |
| `dimensions[0].strategy = ""` | 400 | 1000 | `request validation failed: dimensions[0].strategy must not be empty` |
| `dimensions[0].player_count` is not `6`, `8`, or `9` | 400 | 1000 | `request validation failed: dimensions[0].player_count must be one of 6, 8, 9` |
| `dimensions[0].depth_bb = 0` | 400 | 1000 | `request validation failed: dimensions[0].depth_bb must be greater than 0` |

Not found:

| Condition | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| Any requested dimension is unavailable or its required data file is missing | 404 | 404 | `Dimension not found: strategy={strategy}, player_count={player_count}, depth_bb={depth_bb}` |

Success:

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

Prewarm should be atomic from the API client's perspective: if any requested
dimension is invalid or unavailable, return an error instead of reporting a
partial success.

## `GET /health`

Purpose: process liveness only.

Rules:

- Does not verify range data availability.
- Returns 200 when the HTTP process can respond.
- Response `message` must be `null`.

Success:

```json
{
  "code": 0,
  "data": {
    "status": "ok",
    "uptime_secs": 1.23
  },
  "message": null
}
```

## `GET /ready`

Purpose: data-readiness check for serving business queries.

Rules:

- Returns 200 only when metadata loaded and at least one queryable dimension is known.
- If no queryable dimensions are loaded, return 503 with a business error.
- Response `message` must be `null` on success.

Not ready:

| Condition | HTTP | Code | Message |
| --- | ---: | ---: | --- |
| No queryable dimensions loaded | 503 | 503 | `Service is not ready: no queryable dimensions loaded` |

Success:

```json
{
  "code": 0,
  "data": {
    "status": "ready",
    "schema_count": 1,
    "handles_open": 0,
    "dimensions_known": ["default_6max_100BB"]
  },
  "message": null
}
```

## Implementation Plan

1. Add request DTO defaults with `Option<T>` where fields may be omitted.
2. Add shared validators:
   - required string
   - positive integer
   - optional bounded float
   - drill name business identifier
   - optional arrays where `[]` may have endpoint-specific meaning
3. Expand `AppError` constructors for:
   - dimension not found
   - concrete line not found
   - hand outside action-line range
   - abstract line not found
   - no hands found by action filters
4. Map every public `AppError` to the public API code catalog while preserving the specific message.
5. Change query service methods so missing business results return `AppError`, not empty vectors or `exists: false`.
6. Keep batch as the only endpoint that can return per-item business errors inside a successful envelope.
7. Update OpenAPI schemas and route response annotations.
8. Add route tests for each boundary case listed in this document.
9. Update README API examples after tests pass.

## Test Checklist

- Success envelope uses `message: null`.
- Error envelope has no `details`.
- Invalid JSON returns HTTP `400` with code `1000`.
- Field validation returns HTTP `400` with code `1000`.
- `drill_name = "123"` returns HTTP `400` with code `1000`.
- Missing optional drill dimension fields use defaults.
- Missing drill abstract lines return 404.
- Missing abstract concrete lines return 404.
- Missing concrete line returns 404.
- Valid hand outside the requested action-line range returns 404.
- Unknown hand format returns HTTP `400` with code `1000`.
- `actions = []` returns all hands for `/range/hands-by-actions`.
- Invalid action filter syntax returns `400` with the unified action filter message; parsed actions absent from the concrete line schema are folded into the `No hands found...` 404 response.
- Out-of-range `frequency` returns HTTP `400` with code `1000`.
- Missing prewarm dimension returns 404.
- `/ready` with zero dimensions returns `503`.
- Binary read/decode/corruption errors return HTTP `500` with code `500`.
