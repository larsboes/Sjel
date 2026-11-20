//! Health's owner socket. The server is the only process that opens the owner file: CLI imports
//! and reads arrive as jobs, and backups request snapshots on the same socket.

use std::{path::PathBuf, sync::Arc, time::Duration};

use axum::{body::Bytes, http::StatusCode, routing::post, Json, Router};
use health::{export, HealthStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sjel_server::{owner_snapshot, InboundAuth};

const JOBS: &str = "http://127.0.0.1/health-jobs";

/// One folded value as it travels to the owner. The owner maps `metric` back to the catalogue.
#[derive(Serialize, Deserialize)]
pub struct WireValue {
    pub metric: String,
    pub day: String,
    pub value: f64,
    pub source: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "job", rename_all = "snake_case")]
pub enum Job {
    Import {
        file_name: String,
        records: u64,
        values: Vec<WireValue>,
    },
    Metrics,
    Imports,
    Window {
        from: String,
        to: String,
    },
}

impl Job {
    async fn run(self, store: &HealthStore) -> Result<Value, String> {
        match self {
            Self::Import {
                file_name,
                records,
                values,
            } => {
                let values = values
                    .into_iter()
                    .map(|value| {
                        let (metric, unit) = export::canonical(&value.metric)
                            .ok_or_else(|| format!("unknown metric {}", value.metric))?;
                        Ok(export::DayValue {
                            metric,
                            day: value.day,
                            value: value.value,
                            unit,
                            source: value.source,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                store
                    .replace(&file_name, records, &values)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(json!({ "stored": values.len() }))
            }
            Self::Metrics => store.metrics().await.map(|m| json!({ "metrics": m })),
            Self::Imports => store.imports().await.map(|i| json!({ "imports": i })),
            Self::Window { from, to } => store
                .window(&from, &to)
                .await
                .map(|m| json!({ "from": from, "to": to, "metrics": m })),
        }
        .map_err(|error| error.to_string())
    }
}

/// Serve jobs and snapshots on the owner socket until the process ends.
pub async fn serve(store: HealthStore, database: Arc<PathBuf>) {
    let jobs_store = store.clone();
    let jobs = Router::new().route(
        "/health-jobs",
        post(move |body: Bytes| {
            let store = jobs_store.clone();
            async move {
                let job: Job = serde_json::from_slice(&body)
                    .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
                job.run(&store)
                    .await
                    .map(Json)
                    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))
            }
        }),
    );
    owner_snapshot::serve_with_jobs(
        "health",
        database,
        move |path, target| {
            store
                .snapshot(path, target)
                .map_err(|error| error.to_string())
        },
        jobs,
    )
    .await;
}

/// Send `job` to the running owner, starting it once through sjel-status if it is down.
///
/// # Errors
/// The owner cannot be reached or refuses the job.
pub fn send(job: &Job) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .unix_socket(owner_snapshot::socket_path("health").map_err(|error| error.to_string())?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|error| error.to_string())?;
    let send = || InboundAuth::with_loopback_auth(client.post(JOBS).json(job), JOBS).send();
    let mut response = send();
    if response.as_ref().is_err_and(reqwest::Error::is_connect) {
        start_owner()?;
        response = send();
    }
    let response = response.map_err(|error| error.to_string())?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(format!("the owner refused the job ({status}): {body}"));
    }
    response.json().map_err(|error| error.to_string())
}

fn start_owner() -> Result<(), String> {
    let port = sjel_config::env_var("SJEL_STATUS_PORT")
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
        .unwrap_or(8082);
    let url = format!("http://127.0.0.1:{port}/api/sjel-status/capabilities/health/start");
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    InboundAuth::with_loopback_auth(client.post(&url), &url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map(drop)
        .map_err(|error| format!("health owner did not start: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store(name: &str) -> (HealthStore, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("sjel-health-owner-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (
            HealthStore::open(&dir.join("health.db")).await.unwrap(),
            dir,
        )
    }

    fn import(metric: &str) -> Job {
        Job::Import {
            file_name: "export.zip".into(),
            records: 3,
            values: vec![WireValue {
                metric: metric.into(),
                day: "2026-11-20".into(),
                value: 4321.0,
                source: "Phone".into(),
            }],
        }
    }

    #[tokio::test]
    async fn an_import_job_stores_catalogue_values_and_reads_them_back() {
        let (store, dir) = store("import").await;
        let stored = import("steps").run(&store).await.unwrap();
        assert_eq!(stored["stored"], 1);
        let metrics = Job::Metrics.run(&store).await.unwrap();
        assert_eq!(metrics["metrics"][0]["metric"], "steps");
        assert_eq!(metrics["metrics"][0]["unit"], "count");
        let imports = Job::Imports.run(&store).await.unwrap();
        assert_eq!(imports["imports"][0]["records"], 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_import_job_refuses_a_metric_outside_the_catalogue() {
        let (store, dir) = store("unknown").await;
        let error = import("not_a_metric").run(&store).await.unwrap_err();
        assert!(error.contains("unknown metric not_a_metric"), "{error}");
        assert!(Job::Metrics.run(&store).await.unwrap()["metrics"]
            .as_array()
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
