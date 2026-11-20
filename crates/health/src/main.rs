use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{Days, Local, NaiveDate};
use serde::Deserialize;
use serde_json::{json, Value};

use health::{export, source, HealthStore};

#[cfg(feature = "turso-wal")]
mod owner;
pub use health_api::{DaysUpload, SleepSample, UploadValue};

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get("GET", "/ready", "Liveness plus database readiness."),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::get(
        "GET",
        "/api/metrics",
        "Every stored health metric (sleep, activity, heart, body) with its group, unit, number of days, first and last day, and newest value.",
    ),
    route_manifest::get(
        "GET",
        "/api/days",
        "Daily values, oldest first. ?metric= one metric key (optional), ?from= and ?to= as YYYY-MM-DD (default: the last 30 days). At most 3,660 days per request.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/days",
        summary: "Store daily values from one device, for example the iOS app's HealthKit sync. Body: {source, values: [{metric, day, value}]}. Each value replaces the stored value of the same metric and day. 400 for an unknown metric, a malformed day or a non-finite value; nothing is stored then.",
        request_schema: Some(route_manifest::schema_of::<DaysUpload>),
    },
    route_manifest::get(
        "GET",
        "/api/summary",
        "Mean, min and max per metric over the last ?days= days (default 30, max 3,660), with the number of days that have data.",
    ),
    route_manifest::get(
        "GET",
        "/api/imports",
        "The 50 newest import runs: file name, records read, values stored, day range.",
    ),
];

const MAX_DAYS: u64 = 3_660;

struct AppState {
    store: HealthStore,
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn internal(error: impl std::fmt::Display) -> ApiError {
    eprintln!("health: {error}");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "health operation failed".into(),
    )
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.into())
}

async fn liveness() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "health" }))
}

async fn ready(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    state.store.ping().await.map_err(internal)?;
    Ok(Json(json!({ "status": "ready", "service": "health" })))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("health", ROUTES))
}

async fn metrics(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let metrics = state.store.metrics().await.map_err(internal)?;
    Ok(Json(json!({ "metrics": metrics })))
}

#[derive(Deserialize)]
struct DaysQuery {
    metric: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

fn date(raw: Option<&str>, name: &str, default: NaiveDate) -> Result<NaiveDate, ApiError> {
    raw.map_or(Ok(default), |raw| {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|_| bad(format!("{name} must be YYYY-MM-DD")))
    })
}

async fn days(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DaysQuery>,
) -> Result<Json<Value>, ApiError> {
    let today = Local::now().date_naive();
    let to = date(query.to.as_deref(), "to", today)?;
    let from = date(query.from.as_deref(), "from", to - Days::new(29))?;
    if from > to {
        return Err(bad("from must not be after to"));
    }
    if (to - from).num_days() >= MAX_DAYS as i64 {
        return Err(bad(format!("at most {MAX_DAYS} days per request")));
    }
    if let Some(metric) = query.metric.as_deref() {
        if export::describe(metric).is_none() {
            return Err(bad(format!(
                "unknown metric '{metric}'; GET /api/metrics lists them"
            )));
        }
    }
    let days = state
        .store
        .days(query.metric.as_deref(), &from.to_string(), &to.to_string())
        .await
        .map_err(internal)?;
    Ok(Json(
        json!({ "from": from.to_string(), "to": to.to_string(), "days": days }),
    ))
}

#[derive(Deserialize)]
struct SummaryQuery {
    days: Option<u64>,
}

async fn summary(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<Value>, ApiError> {
    let span = query.days.unwrap_or(30);
    if span == 0 || span > MAX_DAYS {
        return Err(bad(format!("days must be 1..={MAX_DAYS}")));
    }
    let to = Local::now().date_naive();
    let from = to - Days::new(span - 1);
    let stats = state
        .store
        .window(&from.to_string(), &to.to_string())
        .await
        .map_err(internal)?;
    Ok(Json(
        json!({ "from": from.to_string(), "to": to.to_string(), "metrics": stats }),
    ))
}

/// A source name is a device label, not a document.
const MAX_SOURCE_CHARS: usize = 100;

async fn upload(
    State(state): State<Arc<AppState>>,
    Json(upload): Json<DaysUpload>,
) -> Result<Json<Value>, ApiError> {
    let source = upload.source.trim();
    if source.is_empty() || source.chars().count() > MAX_SOURCE_CHARS {
        return Err(bad(format!(
            "source must be 1..={MAX_SOURCE_CHARS} characters"
        )));
    }
    if upload.values.len() > health_api::MAX_UPLOAD_VALUES
        || upload.sleep.len() > health_api::MAX_UPLOAD_SLEEP
        || (upload.values.is_empty() && upload.sleep.is_empty())
    {
        return Err(bad(format!(
            "an upload holds 1..={} values and at most {} sleep samples",
            health_api::MAX_UPLOAD_VALUES,
            health_api::MAX_UPLOAD_SLEEP
        )));
    }
    let from = match upload.from.as_deref() {
        Some(raw) => Some(
            NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                .map_err(|_| bad("from must be YYYY-MM-DD"))?
                .to_string(),
        ),
        None => None,
    };
    let mut values = Vec::with_capacity(upload.values.len());
    for (index, value) in upload.values.iter().enumerate() {
        let Some((metric, unit)) = export::canonical(&value.metric) else {
            return Err(bad(format!(
                "values[{index}]: unknown metric '{}'",
                value.metric
            )));
        };
        if NaiveDate::parse_from_str(&value.day, "%Y-%m-%d").is_err() {
            return Err(bad(format!("values[{index}]: day must be YYYY-MM-DD")));
        }
        if !value.value.is_finite() {
            return Err(bad(format!(
                "values[{index}]: value must be a finite number"
            )));
        }
        values.push(export::DayValue {
            metric,
            day: value.day.clone(),
            value: value.value,
            unit,
            source: source.to_owned(),
        });
    }
    let nights = export::fold_sleep(upload.sleep.iter().map(|sample| {
        (
            sample.source.as_str(),
            sample.start.as_str(),
            sample.end.as_str(),
            sample.stage.as_str(),
        )
    }));
    values.extend(nights.into_iter().filter(|night| {
        from.as_deref()
            .is_none_or(|from| night.day.as_str() >= from)
    }));
    if values.is_empty() {
        return Ok(Json(json!({ "stored": 0 })));
    }
    let stored = values.len();
    state
        .store
        .record(&format!("upload: {source}"), stored as u64, &values)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "stored": stored })))
}

async fn imports(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let imports = state.store.imports().await.map_err(internal)?;
    Ok(Json(json!({ "imports": imports })))
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(liveness))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/metrics", get(metrics))
        .route("/api/days", get(days).post(upload))
        .route("/api/summary", get(summary))
        .route("/api/imports", get(imports))
        .layer(middleware::from_fn_with_state(
            "health",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(state)
}

/// The owner file under `turso-wal`, otherwise the shared database.
fn database() -> PathBuf {
    #[cfg(feature = "turso-wal")]
    {
        sjel_config::env_var("SJEL_HEALTH_DB_PATH")
            .ok()
            .filter(|path| !path.trim().is_empty())
            .map(|path| sjel_config::expand_tilde(&path))
            .unwrap_or_else(|| {
                eprintln!("health: SJEL_HEALTH_DB_PATH must name the separate owner file");
                std::process::exit(1);
            })
    }
    #[cfg(not(feature = "turso-wal"))]
    {
        sjel_config::database_path()
    }
}

async fn open() -> Arc<AppState> {
    match HealthStore::open(&database()).await {
        Ok(store) => Arc::new(AppState { store }),
        Err(error) => {
            eprintln!("health: cannot open the store: {error}");
            std::process::exit(1);
        }
    }
}

async fn import(path: PathBuf) -> i32 {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let folded = tokio::task::spawn_blocking(move || -> Result<export::Summary, String> {
        let mut opened = source::open(&path)?;
        let summary = export::fold(&mut opened.reader).map_err(|e| format!("read failed: {e}"))?;
        opened.finish()?;
        Ok(summary)
    })
    .await;
    let summary = match folded {
        Ok(Ok(summary)) => summary,
        Ok(Err(error)) => {
            eprintln!("health: {error}");
            return 1;
        }
        Err(error) => {
            eprintln!("health: import task failed: {error}");
            return 1;
        }
    };
    if summary.values.is_empty() {
        eprintln!(
            "health: {name} held {} records and none this capability keeps",
            summary.records
        );
        return 1;
    }
    if let Err(error) = store_import(&name, &summary).await {
        eprintln!("health: cannot store the import: {error}");
        return 1;
    }
    let first = summary
        .values
        .iter()
        .map(|v| v.day.as_str())
        .min()
        .unwrap_or("");
    let last = summary
        .values
        .iter()
        .map(|v| v.day.as_str())
        .max()
        .unwrap_or("");
    println!(
        "imported {name}: {} records read, {} used, {} daily values from {first} to {last}",
        summary.records,
        summary.used,
        summary.values.len()
    );
    0
}

const USAGE: &str = "usage:
  health serve           HTTP API
  health import <path>   fold an Apple Health export (.zip, its folder, or export.xml)
  health metrics         each stored metric with its coverage, as JSON
  health imports         the 50 newest imports and phone uploads, as JSON
  health summary [days]  mean, min and max per metric over the last days (default 30), as JSON
  health snapshot        ask the running owner for a backup snapshot (turso-wal builds)";

#[cfg(not(feature = "turso-wal"))]
async fn store_import(name: &str, summary: &export::Summary) -> Result<(), String> {
    open()
        .await
        .store
        .replace(name, summary.records, &summary.values)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(feature = "turso-wal")]
async fn store_import(name: &str, summary: &export::Summary) -> Result<(), String> {
    let job = owner::Job::Import {
        file_name: name.to_owned(),
        records: summary.records,
        values: summary
            .values
            .iter()
            .map(|value| owner::WireValue {
                metric: value.metric.to_owned(),
                day: value.day.clone(),
                value: value.value,
                source: value.source.clone(),
            })
            .collect(),
    };
    tokio::task::spawn_blocking(move || owner::send(&job))
        .await
        .map_err(|error| error.to_string())?
        .map(drop)
}

/// Run one read: through the owner under `turso-wal`, otherwise on the shared database.
#[cfg(not(feature = "turso-wal"))]
async fn read(command: &str, window: Option<(String, String)>) -> Result<Value, String> {
    let store = &open().await.store;
    match (command, window) {
        ("metrics", _) => store.metrics().await.map(|m| json!({ "metrics": m })),
        ("imports", _) => store.imports().await.map(|i| json!({ "imports": i })),
        (_, Some((from, to))) => store
            .window(&from, &to)
            .await
            .map(|m| json!({ "from": from, "to": to, "metrics": m })),
        (_, None) => return Err("summary needs a window".to_owned()),
    }
    .map_err(|error| error.to_string())
}

#[cfg(feature = "turso-wal")]
async fn read(command: &str, window: Option<(String, String)>) -> Result<Value, String> {
    let job = match (command, window) {
        ("metrics", _) => owner::Job::Metrics,
        ("imports", _) => owner::Job::Imports,
        (_, Some((from, to))) => owner::Job::Window { from, to },
        (_, None) => return Err("summary needs a window".to_owned()),
    };
    tokio::task::spawn_blocking(move || owner::send(&job))
        .await
        .map_err(|error| error.to_string())?
}

async fn print_json(command: &str, days: Option<String>) -> i32 {
    let result = match command {
        "metrics" | "imports" => read(command, None).await,
        _ => {
            let span = match days.as_deref().map(str::parse::<u64>) {
                None => 30,
                Some(Ok(n)) if (1..=MAX_DAYS).contains(&n) => n,
                _ => {
                    eprintln!("health: days must be 1..={MAX_DAYS}");
                    return 2;
                }
            };
            let to = Local::now().date_naive();
            let from = to - Days::new(span - 1);
            read(command, Some((from.to_string(), to.to_string()))).await
        }
    };
    match result {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
            0
        }
        Err(error) => {
            eprintln!("health: {error}");
            1
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    match (command.as_str(), args.next()) {
        ("serve", None) => runtime.block_on(async {
            let port = sjel_config::resolve_port(None, None, 8109);
            let state = open().await;
            #[cfg(feature = "turso-wal")]
            {
                let store = state.store.clone();
                tokio::join!(
                    sjel_server::serve_local("health", port, router(state)),
                    owner::serve(store, Arc::new(database())),
                );
            }
            #[cfg(not(feature = "turso-wal"))]
            sjel_server::serve_local("health", port, router(state)).await;
        }),
        ("import", Some(path)) => std::process::exit(runtime.block_on(import(PathBuf::from(path)))),
        ("metrics", None) => std::process::exit(runtime.block_on(print_json("metrics", None))),
        ("imports", None) => std::process::exit(runtime.block_on(print_json("imports", None))),
        ("summary", days) => std::process::exit(runtime.block_on(print_json("summary", days))),
        #[cfg(feature = "turso-wal")]
        ("snapshot", None) => {
            if let Err(error) = runtime.block_on(sjel_server::owner_snapshot::submit("health")) {
                eprintln!("health: snapshot failed: {error}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    async fn app(name: &str) -> (Router, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("sjel-health-api-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = HealthStore::open(&dir.join("sjel.db")).await.unwrap();
        let values = vec![export::DayValue {
            metric: "steps",
            day: Local::now().date_naive().to_string(),
            value: 4321.0,
            unit: "count",
            source: "Phone".into(),
        }];
        store.record("export.zip", 10, &values).await.unwrap();
        store.record("export.zip", 10, &values).await.unwrap();
        (router(Arc::new(AppState { store })), dir)
    }

    async fn get_json(app: &Router, uri: &str) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn reads_answer_from_the_store_and_reimports_replace() {
        let (app, dir) = app("reads").await;
        let (status, metrics) = get_json(&app, "/api/metrics").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(metrics["metrics"][0]["metric"], "steps");
        assert_eq!(metrics["metrics"][0]["days"], 1);
        assert_eq!(metrics["metrics"][0]["group"], "activity");
        let (_, days) = get_json(&app, "/api/days?metric=steps").await;
        assert_eq!(days["days"].as_array().unwrap().len(), 1);
        let (_, summary) = get_json(&app, "/api/summary?days=7").await;
        assert_eq!(summary["metrics"][0]["mean"], 4321.0);
        let (_, imports) = get_json(&app, "/api/imports").await;
        assert_eq!(imports["imports"].as_array().unwrap().len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    async fn post_json(app: &Router, uri: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::post(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn an_upload_replaces_the_day_and_refuses_bad_values_whole() {
        let (app, dir) = app("upload").await;
        let today = Local::now().date_naive().to_string();
        let (status, answer) = post_json(
            &app,
            "/api/days",
            json!({ "source": "HealthKit", "values": [
                { "metric": "steps", "day": today, "value": 9000.0 },
                { "metric": "sleep_asleep_h", "day": today, "value": 7.25 }
            ]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(answer["stored"], 2);
        let (_, days) = get_json(&app, "/api/days?metric=steps").await;
        assert_eq!(days["days"][0]["value"], 9000.0);
        assert_eq!(days["days"][0]["source"], "HealthKit");
        assert_eq!(days["days"].as_array().unwrap().len(), 1);
        let (status, answer) = post_json(
            &app,
            "/api/days",
            json!({ "source": "HealthKit", "from": "2026-11-18", "sleep": [
                { "source": "Watch", "start": "2026-11-16 23:00:00 +0100", "end": "2026-11-17 06:00:00 +0100", "stage": "AsleepCore" },
                { "source": "Watch", "start": "2026-11-17 23:00:00 +0100", "end": "2026-11-18 06:30:00 +0100", "stage": "AsleepCore" }
            ]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, sleep) = get_json(
            &app,
            "/api/days?metric=sleep_asleep_h&from=2026-11-01&to=2026-11-18",
        )
        .await;
        let nights = sleep["days"].as_array().unwrap();
        assert_eq!(
            nights.len(),
            1,
            "the night before `from` is dropped: {nights:?}"
        );
        assert_eq!(nights[0]["day"], "2026-11-18");
        assert_eq!(nights[0]["value"], 7.5);
        assert!(answer["stored"].as_u64().unwrap() >= 3);
        for bad in [
            json!({ "source": "HealthKit", "values": [{ "metric": "water", "day": today, "value": 1.0 }] }),
            json!({ "source": "HealthKit", "values": [{ "metric": "steps", "day": "18.11.2026", "value": 1.0 }] }),
            json!({ "source": "", "values": [{ "metric": "steps", "day": today, "value": 1.0 }] }),
            json!({ "source": "HealthKit", "values": [] }),
        ] {
            assert_eq!(
                post_json(&app, "/api/days", bad).await.0,
                StatusCode::BAD_REQUEST
            );
        }
        let (_, days) = get_json(&app, "/api/days?metric=steps").await;
        assert_eq!(
            days["days"][0]["value"], 9000.0,
            "a refused upload stores nothing"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_contract_route_is_in_the_manifest() {
        for route in health_api::ALL_ROUTES {
            assert!(ROUTES.iter().any(|r| r.path == route), "{route}");
        }
    }

    #[tokio::test]
    async fn bad_queries_are_refused() {
        let (app, dir) = app("bad").await;
        assert_eq!(
            get_json(&app, "/api/days?metric=water").await.0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            get_json(&app, "/api/days?from=2026-02-01&to=2026-01-01")
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            get_json(&app, "/api/days?from=2000-01-01&to=2026-01-01")
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            get_json(&app, "/api/summary?days=0").await.0,
            StatusCode::BAD_REQUEST
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
