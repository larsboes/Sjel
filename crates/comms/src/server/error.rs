use axum::{http::StatusCode, response::Json};
use serde_json::{json, Value};

/// The server's stable JSON response shape for handlers that can fail.
pub(super) type HttpResponse = (StatusCode, Json<Value>);

/// Build the shared `{ "error": ... }` response without coupling handlers to
/// a catch-all error enum that would erase their domain-specific statuses.
pub(super) fn error_response(status: StatusCode, message: impl Into<String>) -> HttpResponse {
    (status, Json(json!({ "error": message.into() })))
}
