use std::path::Path;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use poker_hands_storage_service::http::router;
use poker_hands_storage_service::query::QueryService;
use poker_hands_storage_service::range_store_builder::{build_store, BuildOptions, DimensionSpec};
use poker_hands_storage_service::storage::sqlite::Connection;
use serde_json::{json, Value};
use tower::ServiceExt;

#[tokio::test]
async fn serves_query_and_metadata_workflows() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = build_test_store(directory.path());
    let service = Arc::new(QueryService::open(&data_dir, 2, true).unwrap());
    let app = router(service);

    let (status, health) = call_json(&app, Method::GET, "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health["status"], "ok");

    let (status, ready) = call_json(&app, Method::GET, "/ready", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["dimensions_known"][0], "default_6max_100BB");

    let (status, openapi) = call_json(&app, Method::GET, "/api-docs/openapi.json", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(openapi["info"]["title"], "Poker Hands Storage API");
    assert!(openapi["paths"].get("/range/hand-strategy").is_some());
    assert!(openapi["paths"].get("/range/hands-by-actions").is_some());
    assert!(openapi["components"]["schemas"]
        .get("QueryRequest")
        .is_some());

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
    assert_eq!(result["hand_code"], "AA");
    assert_eq!(result["exists"], true);
    assert_eq!(result["actions"].as_array().unwrap().len(), 2);

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
    assert_eq!(error["code"], "INVALID_ARGUMENT");
    assert_eq!(error["message"], "request validation failed");
    let fields = error["details"]["fields"].as_array().unwrap();
    assert!(fields
        .iter()
        .any(|field| field["path"] == "concrete_line_id"));
    assert!(fields.iter().any(|field| field["path"] == "hole_cards"));

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
    assert_eq!(error["code"], "UNKNOWN_HAND");

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
        "/range/hand-strategy/batch",
        Some(batch),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["results"][0]["strategy"]["exists"], true);
    assert!(result["results"][0]["strategy"].get("hand_code").is_none());
    assert_eq!(result["results"][1]["error"]["code"], "UNKNOWN_HAND");

    let prewarm = json!({
        "dimensions": [
            { "strategy": "default", "player_count": 6, "depth_bb": 100 }
        ]
    });
    let (status, result) = call_json(&app, Method::POST, "/range/prewarm", Some(prewarm)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["prewarmed"], 1);
    assert_eq!(result["total_open"], 1);

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
    assert_eq!(result["lines"][0]["concrete_line_id"], 1);

    let drill_lines = json!({
        "strategy": "default",
        "drill_name": "UTG",
        "player_count": 6,
        "drill_depth": 0
    });
    let (status, result) = call_json(
        &app,
        Method::POST,
        "/range/drill-scenarios",
        Some(drill_lines),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["abstract_lines"], json!(["F-F-F"]));

    // hands-by-actions: query all hands grouped by action
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
    assert_eq!(result["concrete_line_id"], 1);
    let actions = result["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 2);
    // Each action should have a hands array with "AA"
    for action in actions {
        assert!(action["action_name"].is_string());
        assert!(action["hands"].as_array().unwrap().contains(&json!("AA")));
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

fn build_test_store(root: &Path) -> std::path::PathBuf {
    let source_path = root.join("source.db");
    let output_path = root.join("output");
    let source = Connection::open(&source_path, false).unwrap();
    source
        .exec(
            "CREATE TABLE range_data_default_6max_100BB (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               concrete_line_id INTEGER NOT NULL,
               hole_cards TEXT NOT NULL,
               action_name TEXT NOT NULL,
               action_size REAL NOT NULL,
               amount_bb REAL NOT NULL,
               frequency REAL NOT NULL,
               hand_ev REAL NULL
             );
             CREATE TABLE concrete_lines_default_6max_100BB (
               id INTEGER PRIMARY KEY,
               abstract_line TEXT NOT NULL,
               concrete_line TEXT NOT NULL
             );
             CREATE TABLE drill_scenario_lines_default (
               id INTEGER PRIMARY KEY,
               drill_name TEXT NOT NULL,
               abstract_line TEXT NOT NULL,
               player_count INTEGER NOT NULL,
               depth INTEGER NOT NULL
             );
             INSERT INTO concrete_lines_default_6max_100BB
               VALUES (1, 'F-F-F', 'F-F-F');
             INSERT INTO drill_scenario_lines_default
               VALUES (1, 'UTG', 'F-F-F', 6, 0);
             INSERT INTO range_data_default_6max_100BB(
               concrete_line_id, hole_cards, action_name, action_size,
               amount_bb, frequency, hand_ev
             ) VALUES
               (1, 'AA', 'fold', 0, 0, 0.25, NULL),
               (1, 'AA', 'raise', 2.5, 2.5, 0.75, 1.0);",
        )
        .unwrap();
    drop(source);

    build_store(&BuildOptions {
        source_db: source_path,
        out_dir: output_path.clone(),
        dimensions: vec![DimensionSpec {
            strategy: "default".to_owned(),
            player_count: 6,
            depth_bb: 100,
        }],
        max_concrete_lines_per_dimension: None,
        overwrite: false,
    })
    .unwrap();
    output_path
}
