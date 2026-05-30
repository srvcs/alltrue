use axum::body::Body;
use axum::extract::Json as JsonExtract;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router as AxumRouter};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use srvcs_alltrue::{api::Deps, health, router, telemetry};
use tower::ServiceExt;

const DEAD_URL: &str = "http://127.0.0.1:1";

/// Mock `srvcs-and` that ACTUALLY COMPUTES: it reads `{a, b}` from the request
/// and returns `{"a", "b", "result": a && b}`. This is what makes the fold
/// genuinely testable — the running accumulator is real, not faked.
async fn spawn_mock_and() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|JsonExtract(req): JsonExtract<Value>| async move {
            let a = req["a"].as_bool().unwrap_or(false);
            let b = req["b"].as_bool().unwrap_or(false);
            Json(json!({ "a": a, "b": b, "result": a && b }))
        }),
    );
    serve(app).await
}

/// Mock `srvcs-and` that always answers with a fixed status + body (used to
/// simulate a `422` rejection of a non-boolean element).
async fn spawn_fixed(status: StatusCode, body: Value) -> String {
    let app = AxumRouter::new().route(
        "/",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    serve(app).await
}

async fn serve(app: AxumRouter) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn app(and_url: &str) -> axum::Router {
    router(
        telemetry::metrics_handle_for_tests(),
        Deps {
            and_url: and_url.to_string(),
        },
    )
}

async fn eval(and_url: &str, values: Value) -> (StatusCode, Value) {
    let res = app(and_url)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "values": values }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn status_of(uri: &str) -> StatusCode {
    app(DEAD_URL)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

// --- Standard srvcs service surface ---

#[tokio::test]
async fn index_ok() {
    assert_eq!(status_of("/").await, StatusCode::OK);
}

#[tokio::test]
async fn healthz_ok() {
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reflects_state() {
    health::set_ready(true);
    assert_eq!(status_of("/readyz").await, StatusCode::OK);
}

#[tokio::test]
async fn metrics_ok() {
    assert_eq!(status_of("/metrics").await, StatusCode::OK);
}

#[tokio::test]
async fn openapi_ok() {
    assert_eq!(status_of("/openapi.json").await, StatusCode::OK);
}

// --- Truth-table cases, exercised against a REAL computing and ---

#[tokio::test]
async fn all_true_is_true() {
    let and = spawn_mock_and().await;
    let (status, body) = eval(&and, json!([true, true, true])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], true);
    assert_eq!(body["values"], json!([true, true, true]));
}

#[tokio::test]
async fn one_false_makes_it_false() {
    let and = spawn_mock_and().await;
    let (status, body) = eval(&and, json!([true, false, true])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], false);
}

#[tokio::test]
async fn all_false_is_false() {
    let and = spawn_mock_and().await;
    let (status, body) = eval(&and, json!([false, false])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], false);
}

#[tokio::test]
async fn singleton_true_is_true() {
    let and = spawn_mock_and().await;
    let (status, body) = eval(&and, json!([true])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], true);
}

#[tokio::test]
async fn singleton_false_is_false() {
    let and = spawn_mock_and().await;
    let (status, body) = eval(&and, json!([false])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], false);
}

#[tokio::test]
async fn empty_list_is_true_with_no_calls() {
    // DEAD_URL: if the fold tried to call and at all on an empty list, this
    // would degrade to 503. It must short-circuit to true with no calls.
    let (status, body) = eval(DEAD_URL, json!([])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], true);
    assert_eq!(body["values"], json!([]));
}

// --- Error / edge cases ---

#[tokio::test]
async fn forwards_422_for_non_boolean_element() {
    let and = spawn_fixed(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "b is not a boolean" }),
    )
    .await;
    let (status, body) = eval(&and, json!([true, "nope", true])).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "b is not a boolean");
}

#[tokio::test]
async fn degrades_when_and_is_unreachable() {
    let (status, body) = eval(DEAD_URL, json!([true, true])).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-and");
}

#[tokio::test]
async fn server_error_when_and_returns_no_boolean() {
    let and = spawn_fixed(StatusCode::OK, json!({ "a": true, "b": true })).await;
    let (status, _body) = eval(&and, json!([true])).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn generates_request_id_when_absent() {
    let res = app(DEAD_URL)
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        res.headers().contains_key("x-request-id"),
        "response must carry a generated x-request-id"
    );
}
