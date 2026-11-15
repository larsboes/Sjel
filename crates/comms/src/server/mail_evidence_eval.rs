use super::*;
use comms::mail_evidence_source::{self, NativeReview, NativeRun};

fn refused(code: &'static str) -> HttpResponse {
    let status = match code {
        "source_classification" => StatusCode::FORBIDDEN,
        "source_context_changed" | "project_context_changed" | "source changed since review" => {
            StatusCode::CONFLICT
        }
        "source_too_large" => StatusCode::PAYLOAD_TOO_LARGE,
        "source_missing" => StatusCode::NOT_FOUND,
        "mail_auth"
        | "source_store"
        | "source_transport"
        | "project_context"
        | "runtime_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_REQUEST,
    };
    error_response(status, code)
}

pub(super) async fn freeze(Json(body): Json<NativeReview>) -> HttpResponse {
    match tokio::task::spawn_blocking(move || mail_evidence_source::freeze(&Config::load(), body))
        .await
    {
        Ok(Ok(corpus)) => (StatusCode::OK, Json(json!(corpus))),
        Ok(Err(code)) => refused(code),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "evaluation task failed"),
    }
}

pub(super) async fn run(Json(body): Json<NativeRun>) -> HttpResponse {
    match tokio::task::spawn_blocking(move || {
        let cfg = Config::load();
        let runtime = if body.role == "rules" {
            sjel_runtime::Status::normal("native-rules-evaluation")
        } else {
            sjel_runtime::current().map_err(|_| "runtime_unavailable")?
        };
        mail_evidence_source::run(&cfg, &runtime, body)
    })
    .await
    {
        Ok(Ok(report)) => (StatusCode::OK, Json(json!(report))),
        Ok(Err(code)) => refused(code),
        Err(_) => error_response(StatusCode::INTERNAL_SERVER_ERROR, "evaluation task failed"),
    }
}
