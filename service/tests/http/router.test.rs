#[path = "../support/api_test_fixture.rs"]
mod api_test_fixture;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use poker_hands_storage_service::http::{error_code, router};
use poker_hands_storage_service::query::QueryService;
use serde_json::{json, Value};
use tower::ServiceExt;

#[tokio::test]
async fn serves_query_and_metadata_workflows() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = api_test_fixture::build_api_test_store(directory.path());
    let service = Arc::new(QueryService::open(&data_dir, 2, true).unwrap());
    let app = router(service);

    let (status, health) = call_json(&app, Method::GET, "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health["code"], 0);
    assert_eq!(health["data"]["status"], "ok");
    assert!(health["message"].is_null());

    let (status, ready) = call_json(&app, Method::GET, "/ready", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["code"], 0);
    assert_eq!(ready["data"]["dimensions_known"][0], "default_6max_100BB");
    assert!(ready["message"].is_null());

    let (status, openapi) = call_json(&app, Method::GET, "/api-docs/openapi.json", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(openapi["info"]["title"], "Poker Hands Storage API");
    assert!(openapi["paths"].get("/range/hand-strategy").is_some());
    assert!(openapi["paths"].get("/range/hand-strategy-batch").is_some());
    assert!(openapi["paths"].get("/range/hands-by-actions").is_some());
    assert!(openapi["paths"].get("/range/prewarm").is_some());
    assert!(openapi["paths"].get("/range/concrete-lines").is_some());
    assert!(openapi["paths"].get("/range/drill-scenarios").is_some());
    // Old alias paths should not exist
    assert!(openapi["paths"].get("/query").is_none());
    assert!(openapi["paths"].get("/batch").is_none());
    assert!(openapi["paths"].get("/prewarm").is_none());
    assert!(openapi["paths"].get("/concrete-lines").is_none());
    assert!(openapi["paths"].get("/drill-scenario-lines").is_none());
    assert!(openapi["components"]["schemas"]
        .get("QueryResponse")
        .is_some());
    assert_eq!(
        openapi["components"]["schemas"]["HandsByActionsRequest"]["properties"]["frequency"]
            ["default"],
        json!(0.005)
    );
    assert_eq!(
        openapi["components"]["schemas"]["DrillScenarioLinesRequest"]["properties"]["drill_name"]
            ["default"],
        json!("rfi")
    );

    let (status, swagger) = call_text(&app, Method::GET, "/swagger").await;
    assert_eq!(status, StatusCode::OK);
    assert!(swagger.contains("Scalar.createApiReference"));
    assert!(swagger.contains("/api-docs/openapi.json"));

    let query = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "hole_cards": "AsAh"
    });
    let (status, result) = call_json(&app, Method::POST, "/range/hand-strategy", Some(query)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["code"], 0);
    assert_eq!(result["data"]["hand_code"], "AA");
    assert!(result["data"].get("exists").is_none());
    assert_eq!(result["data"]["actions"].as_array().unwrap().len(), 2);
    assert!(result["message"].is_null());

    let invalid_payload = json!({
        "strategy": "",
        "player_count": 0,
        "depth_bb": 0,
        "concrete_line_id": 0,
        "hole_cards": ""
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy",
        Some(invalid_payload),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert!(error["data"].is_null());
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &[
            "strategy",
            "player_count",
            "depth_bb",
            "concrete_line_id",
            "hole_cards",
        ],
    );

    let invalid_query = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "hole_cards": "AsXx"
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy",
        Some(invalid_query),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert!(error["data"].is_null());
    assert_no_details(&error);

    let missing_line_query = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 999,
        "hole_cards": "AA"
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy",
        Some(missing_line_query),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], error_code::NOT_FOUND);
    assert!(error["data"].is_null());
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &[
            "Concrete line not found: concrete_line_id=999",
            "dimension=default:6:100",
        ],
    );

    let missing_strategy_query = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "hole_cards": "AKs"
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy",
        Some(missing_strategy_query),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], error_code::NOT_FOUND);
    assert!(error["data"].is_null());
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &[
            "Hand AKs is outside the range for action line",
            "concrete_line_id=1",
            "dimension default:6:100",
        ],
    );

    let batch = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "requests": [
            { "concrete_line_id": 1, "hole_cards": "AA" },
            { "concrete_line_id": 1, "hole_cards": "AsXx" }
        ]
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy-batch",
        Some(batch),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["code"], 0);
    assert!(result["data"]["results"][0]["strategy"]
        .get("exists")
        .is_none());
    assert!(result["data"]["results"][0]["strategy"]
        .get("hand_code")
        .is_none());
    assert_eq!(
        result["data"]["results"][1]["error"]["code"],
        error_code::BAD_REQUEST
    );

    let prewarm = json!({
        "dimensions": [
            { "strategy": "default", "player_count": 6, "depth_bb": 100 }
        ]
    });
    let (status, result) = call_json(&app, Method::POST, "/range/prewarm", Some(prewarm)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["code"], 0);
    assert_eq!(result["data"]["prewarmed"], 1);
    assert_eq!(result["data"]["total_open"], 1);

    let concrete_lines = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "abstract_line": "F-F-F"
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/concrete-lines",
        Some(concrete_lines),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["code"], 0);
    assert_eq!(result["data"]["lines"][0]["concrete_line_id"], 1);

    let drill_lines = json!({});
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/drill-scenarios",
        Some(drill_lines),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["code"], 0);
    assert_eq!(result["data"]["abstract_lines"], json!(["F-F-F"]));

    let missing_drill_lines = json!({
        "strategy": "default",
        "drill_name": "RFI",
        "player_count": 6,
        "drill_depth": 100
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/drill-scenarios",
        Some(missing_drill_lines),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], error_code::NOT_FOUND);
    assert!(error["data"].is_null());
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &[
            "No abstract lines found for drill",
            "strategy=default",
            "drill_name=RFI",
            "player_count=6",
            "drill_depth=100",
        ],
    );

    // hands-by-actions: query all hands in the concrete line
    let hands_by_actions = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["code"], 0);
    assert_eq!(result["data"]["hands"], json!(["AA", "KK"]));
    assert!(result["data"].get("concrete_line_id").is_none());
    assert!(result["data"].get("actions").is_none());

    // hands-by-actions: empty actions means all hands
    let hands_by_actions_empty = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "actions": []
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions_empty),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["data"]["hands"], json!(["AA", "KK"]));

    // hands-by-actions: filter by action "fold" (no amount)
    let hands_by_actions_fold = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "actions": ["fold"]
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions_fold),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["data"]["hands"], json!(["AA"]));

    // hands-by-actions: action names use OR semantics, and amount-bearing names match any size
    let hands_by_actions_raise = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "actions": ["fold", "raise"]
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions_raise),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["data"]["hands"], json!(["AA", "KK"]));

    // hands-by-actions: absent action names do not suppress other action matches
    let hands_by_actions_raise_with_absent = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "actions": ["raise", "check"]
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions_raise_with_absent),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["data"]["hands"], json!(["AA", "KK"]));

    // hands-by-actions: filter with action absent from schema returns 404
    let hands_by_actions_none = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "actions": ["check"]
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions_none),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], error_code::NOT_FOUND);
    assert!(error["data"].is_null());
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &[
            "No hands found for actions=check at frequency>0.005",
            "concrete_line_id=1",
            "dimension=default:6:100",
        ],
    );

    // hands-by-actions: frequency filter excludes low-frequency actions but returns hands
    let hands_by_actions_freq = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "frequency": 0.5
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions_freq),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["data"]["hands"], json!(["AA", "KK"]));

    // hands-by-actions: explicit frequency is strict greater-than, not greater-than-or-equal
    let hands_by_actions_freq_strict = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "actions": ["fold"],
        "frequency": 0.25
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions_freq_strict),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], error_code::NOT_FOUND);
    assert!(error["data"].is_null());
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &[
            "No hands found for actions=fold at frequency>0.25",
            "concrete_line_id=1",
            "dimension=default:6:100",
        ],
    );

    // hands-by-actions: filters resolve but no hands meet frequency
    let hands_by_actions_no_hands = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 1,
        "actions": ["raise2.5"],
        "frequency": 0.9
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions_no_hands),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], error_code::NOT_FOUND);
    assert!(error["data"].is_null());
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &[
            "No hands found for actions=raise2.5 at frequency>0.9",
            "concrete_line_id=1",
            "dimension=default:6:100",
        ],
    );

    let hands_by_actions_missing_line = json!({
        "strategy": "default",
        "player_count": 6,
        "depth_bb": 100,
        "concrete_line_id": 999
    });
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(hands_by_actions_missing_line),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error["code"], error_code::NOT_FOUND);
    assert!(error["data"].is_null());
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &[
            "Concrete line not found: concrete_line_id=999",
            "dimension=default:6:100",
        ],
    );
}

#[tokio::test]
async fn ready_returns_503_when_no_queryable_dimensions_are_loaded() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = api_test_fixture::build_empty_store(directory.path());
    let service = Arc::new(QueryService::open(&data_dir, 2, true).unwrap());
    let app = router(service);

    let (status, error) = call_json(&app, Method::GET, "/ready", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["code"], error_code::SERVICE_UNAVAILABLE);
    assert!(error["data"].is_null());
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &["Service is not ready: no queryable dimensions loaded"],
    );
}

#[tokio::test]
async fn returns_boundary_validation_messages_for_range_endpoints() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = api_test_fixture::build_api_test_store(directory.path());
    let service = Arc::new(QueryService::open(&data_dir, 2, true).unwrap());
    let app = router(service);

    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy-batch",
        Some(json!({
            "strategy": "default",
            "player_count": 6,
            "depth_bb": 100,
            "requests": []
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(&error, &["requests must contain at least one item"]);

    let too_many_requests = (0..501)
        .map(|_| json!({ "concrete_line_id": 1, "hole_cards": "AA" }))
        .collect::<Vec<_>>();
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy-batch",
        Some(json!({
            "strategy": "default",
            "player_count": 6,
            "depth_bb": 100,
            "requests": too_many_requests
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(&error, &["requests must contain at most 500 items"]);

    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hand-strategy-batch",
        Some(json!({
            "strategy": "default",
            "player_count": 6,
            "depth_bb": 100,
            "requests": [
                { "concrete_line_id": 0, "hole_cards": "" }
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &["requests[0].concrete_line_id", "requests[0].hole_cards"],
    );

    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(json!({
            "strategy": "",
            "player_count": 0,
            "depth_bb": 0,
            "concrete_line_id": 0
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &["strategy", "player_count", "depth_bb", "concrete_line_id"],
    );

    for action in ["", "x"] {
        let (status, error) = call_json(
            &app,
            Method::POST,
            "/range/hands-by-actions",
            Some(json!({
                "strategy": "default",
                "player_count": 6,
                "depth_bb": 100,
                "concrete_line_id": 1,
                "actions": [action]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["code"], error_code::BAD_REQUEST);
        assert_no_details(&error);
        assert_error_message_contains(
            &error,
            &["actions[0] must be one of fold, check, call, bet, raise, allin"],
        );
    }

    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(json!({
            "strategy": "default",
            "player_count": 6,
            "depth_bb": 100,
            "concrete_line_id": 1,
            "actions": ["raiseabc"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(&error, &["actions[0] must have a valid numeric suffix"]);

    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/hands-by-actions",
        Some(json!({
            "strategy": "default",
            "player_count": 6,
            "depth_bb": 100,
            "concrete_line_id": 1,
            "actions": ["fold123"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(&error, &["actions[0] must not have a numeric suffix"]);

    for frequency in [-0.1, 1.1] {
        let (status, error) = call_json(
            &app,
            Method::POST,
            "/range/hands-by-actions",
            Some(json!({
                "strategy": "default",
                "player_count": 6,
                "depth_bb": 100,
                "concrete_line_id": 1,
                "frequency": frequency
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["code"], error_code::BAD_REQUEST);
        assert_no_details(&error);
        assert_error_message_contains(&error, &["frequency must be between 0 and 1"]);
    }

    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/prewarm",
        Some(json!({ "dimensions": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(&error, &["dimensions must contain at least one item"]);

    let too_many_dimensions = (0..65)
        .map(|_| json!({ "strategy": "default", "player_count": 6, "depth_bb": 100 }))
        .collect::<Vec<_>>();
    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/prewarm",
        Some(json!({ "dimensions": too_many_dimensions })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(&error, &["dimensions must contain at most 64 items"]);

    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/prewarm",
        Some(json!({
            "dimensions": [
                { "strategy": "", "player_count": 0, "depth_bb": 0 }
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &[
            "dimensions[0].strategy",
            "dimensions[0].player_count",
            "dimensions[0].depth_bb",
        ],
    );

    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/concrete-lines",
        Some(json!({
            "strategy": "",
            "player_count": 0,
            "depth_bb": 0,
            "abstract_line": ""
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &["strategy", "player_count", "depth_bb", "abstract_line"],
    );

    let (status, error) = call_json(
        &app,
        Method::POST,
        "/range/drill-scenarios",
        Some(json!({
            "strategy": "",
            "drill_name": "",
            "player_count": 0,
            "drill_depth": 0
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], error_code::BAD_REQUEST);
    assert_no_details(&error);
    assert_error_message_contains(
        &error,
        &["strategy", "drill_name", "player_count", "drill_depth"],
    );
}

fn assert_no_details(error: &Value) {
    assert!(
        error.get("details").is_none(),
        "error response must not include details: {error:?}"
    );
}

fn assert_error_message_contains(error: &Value, expected_parts: &[&str]) {
    let message = error["message"].as_str().unwrap();
    for expected_part in expected_parts {
        assert!(
            message.contains(expected_part),
            "missing error message part {expected_part}; message={message}"
        );
    }
}

async fn call_json(
    app: &Router,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn call_text(app: &Router, method: Method, path: &str) -> (StatusCode, String) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}
