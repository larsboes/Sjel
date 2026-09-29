# Error Architecture in Sjel

## 1. Domain Errors with `thiserror`
Define domain errors as enums deriving `thiserror::Error`. Give each variant a descriptive, human-readable format string:

```rust
#[derive(Debug, thiserror::Error)]
pub enum TripError {
    #[error("trip {0} not found")]
    NotFound(String),
    #[error("invalid date format '{0}': expected YYYY-MM-DD")]
    InvalidDate(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}
```

## 2. Converting Domain Errors to HTTP Responses
Capabilities should not manually map errors in every handler to `(StatusCode, String)` or write repetitive `Ok(Ok(...))` match arms.

Implement `axum::response::IntoResponse` for capability error types:

```rust
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

impl IntoResponse for TripError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            Self::InvalidDate(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::Database(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            Self::Join(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
```

## 3. Cleaning Up `tokio::task::spawn_blocking`
Instead of deeply nested matches:
```rust
// AVOID:
match tokio::task::spawn_blocking(move || store.fetch_trip(&id)).await {
    Ok(Ok(trip)) => response(StatusCode::OK, trip),
    Ok(Err(TripError::NotFound(id))) => response(StatusCode::NOT_FOUND, json!({ "error": id })),
    Ok(Err(e)) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })),
    Err(e) => response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": e.to_string() })),
}
```

Use `?` propagation directly in your handler:
```rust
// PREFERRED:
pub async fn get_trip(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Trip>, TripError> {
    let trip = tokio::task::spawn_blocking(move || {
        state.store.fetch_trip(&id)
    }).await??;

    Ok(Json(trip))
}
```
This is enabled because `tokio::task::JoinError` is converted into `TripError` via `#[from]`.
