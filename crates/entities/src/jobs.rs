//! Local-only jobs served by the Entities owner.

use super::*;

fn job_socket() -> Result<std::path::PathBuf, String> {
    sjel_config::overlay_root()
        .map(|overlay| overlay.join("secrets/entities-jobs.sock"))
        .ok_or_else(|| "Entities jobs require an overlay".into())
}

pub(super) fn submit_job(job: Job) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .unix_socket(job_socket()?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|error| error.to_string())?;
    let url = "http://127.0.0.1/jobs";
    let response = sjel_server::InboundAuth::with_loopback_auth(client.post(url).json(&job), url)
        .send()
        .map_err(|error| format!("Entities owner job: {error}"))?;
    let status = response.status();
    let body = response.text().map_err(|error| error.to_string())?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!("Entities owner job returned {status}: {body}"))
    }
}

pub(super) async fn serve_jobs(config: Config, state: Arc<AppState>) {
    let socket = job_socket().unwrap_or_else(|error| fatal(error));
    let router = Router::new().route(
        "/jobs",
        post(move |Json(job): Json<Job>| {
            let config = config.clone();
            let state = Arc::clone(&state);
            async move {
                match tokio::task::spawn_blocking(move || run_job(&config, &state.store, job)).await
                {
                    Ok(Ok(output)) => Ok(output),
                    Ok(Err(error)) => Err((StatusCode::BAD_REQUEST, error)),
                    Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())),
                }
            }
        }),
    );
    sjel_server::serve_unix(
        "entities-jobs",
        &socket,
        router,
        sjel_server::InboundAuth::from_deployment().require_credential(),
    )
    .await;
}
