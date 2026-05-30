use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-alltrue";
pub const CONCERN: &str = "logic: are all values true";
pub const DEPENDS_ON: &[&str] = &["srvcs-and"];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub and_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    /// The list of booleans to AND together. An empty list is `true`.
    #[schema(value_type = Object)]
    pub values: Vec<Value>,
}

#[derive(Serialize, ToSchema)]
pub struct AllTrueResponse {
    #[schema(value_type = Object)]
    pub values: Vec<Value>,
    pub result: bool,
}

fn ok(values: Vec<Value>, result: bool) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "values": values, "result": result })),
    )
        .into_response()
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// Ask `srvcs-and` to compute `acc && v`, returning the running conjunction.
///
/// Maps the dependency's failures to the response this service should return:
/// `503` if it is unreachable, the forwarded `422` if `and` rejects the element
/// (e.g. a non-boolean), and a generic `500` if `and` returns an unusable body.
async fn ask(url: &str, acc: bool, v: &Value) -> Result<bool, Response> {
    let body = json!({ "a": acc, "b": v });
    match client::call(url, &body).await {
        Err(DepError::Unreachable) => Err(degraded("srvcs-and")),
        Ok((200, body)) => match body.get("result").and_then(Value::as_bool) {
            Some(result) => Ok(result),
            None => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "srvcs-and returned no boolean result" })),
            )
                .into_response()),
        },
        // Bad element (e.g. not a boolean) — and already judged it; forward it.
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded("srvcs-and")),
    }
}

/// `POST /` — are all values in the list `true`?
///
/// This service does no logic of its own. It folds the list through
/// `srvcs-and`, starting from `true`: `acc = and(acc, v)` for each element. The
/// conjunction of the empty list is `true` and makes no dependency calls. If
/// `and` rejects an element the `422` is forwarded; if `and` is unreachable this
/// service reports itself degraded rather than guessing.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = AllTrueResponse),
        (status = 422, description = "an element is not a valid boolean (forwarded from srvcs-and)"),
        (status = 500, description = "srvcs-and returned an unusable response"),
        (status = 503, description = "the srvcs-and dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    let mut acc = true;
    for v in &req.values {
        acc = match ask(&deps.and_url, acc, v).await {
            Ok(result) => result,
            Err(resp) => return resp,
        };
    }
    ok(req.values, acc)
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, AllTrueResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[tokio::test]
    async fn index_reports_dependency() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-alltrue");
        assert_eq!(info.concern, "logic: are all values true");
        assert_eq!(info.depends_on, vec!["srvcs-and"]);
    }
}
