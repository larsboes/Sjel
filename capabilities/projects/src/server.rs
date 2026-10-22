use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use projects::{ops, Kind, Projects};

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get("GET", "/ready", "Liveness plus database readiness."),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::get(
        "GET",
        "/api/tasks",
        "Tasks of live projects in the vault reader's shape. ?status=open|done filters.",
    ),
    route_manifest::get(
        "GET",
        "/api/projects",
        "Every project with its open and done task counts and its note count.",
    ),
    route_manifest::get(
        "GET",
        "/api/records",
        "Every record of one kind in live projects, in the task shape with all its properties. ?kind=project|task|note.",
    ),
    route_manifest::get(
        "GET",
        "/api/files",
        "Every attachment path: the images, PDFs and other files of projects.",
    ),
    route_manifest::get(
        "GET",
        "/api/file",
        "One attachment's bytes. ?path= names it. Served sandboxed and never sniffed.",
    ),
    route_manifest::get(
        "GET",
        "/api/record",
        "One record, content included, with `updated_at`: the moment its row was last written. ?path= names it.",
    ),
    route_manifest::get(
        "GET",
        "/api/project",
        "One project's records, each with `updated_at`. ?name= selects it; ?content=false leaves every content out, for a list that only draws titles or asks a replica which records moved.",
    ),
    route_manifest::Route {
        method: "POST",
        path: "/api/tasks",
        summary: "Create a task in a live project. Renders its file before it answers.",
        request_schema: Some(route_manifest::schema_of::<AddTask>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/tasks/done",
        summary: "Mark a task done, or open again with done=false.",
        request_schema: Some(route_manifest::schema_of::<SetDone>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/projects",
        summary: "Create a project hub.",
        request_schema: Some(route_manifest::schema_of::<AddProject>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/record",
        summary: "Replace a record's whole content. `base` is the content the edit started from; a stale base returns 409 with the current content and writes nothing.",
        request_schema: Some(route_manifest::schema_of::<SaveRecord>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/record/property",
        summary: "Set one frontmatter property of a live record; an empty value clears it. A task's status sets done with it. Answers with the task shape.",
        request_schema: Some(route_manifest::schema_of::<SetProperty>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/notes",
        summary: "Create a note in a live project.",
        request_schema: Some(route_manifest::schema_of::<AddNote>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/record/remove",
        summary: "Remove a record from the projects store and disk.",
        request_schema: Some(route_manifest::schema_of::<RemoveRecord>),
    },
    route_manifest::Route {
        method: "POST",
        path: "/api/file",
        summary: "Store a project attachment (base64 body) under a live project folder; refuses to overwrite different bytes (409).",
        request_schema: Some(route_manifest::schema_of::<AttachFile>),
    },
];

#[derive(Deserialize, schemars::JsonSchema)]
struct AddTask {
    project: String,
    title: String,
    #[serde(default)]
    summary: String,
    due: Option<String>,
    priority: Option<u8>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SetDone {
    id: String,
    done: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct AddProject {
    name: String,
    #[serde(default)]
    summary: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SaveRecord {
    path: String,
    content: String,
    base: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SetProperty {
    path: String,
    key: String,
    #[serde(default)]
    value: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct AddNote {
    project: String,
    title: String,
    #[serde(default)]
    summary: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct RemoveRecord {
    path: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct AttachFile {
    path: String,
    base64: String,
}

#[derive(Deserialize)]
struct TaskQuery {
    status: Option<String>,
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

#[derive(Deserialize)]
struct KindQuery {
    kind: Kind,
}

#[derive(Deserialize)]
struct ProjectQuery {
    name: String,
    /// Absent means included, so a reader written before this flag keeps its answer.
    content: Option<bool>,
}

#[derive(Deserialize)]
struct RecordQuery {
    path: String,
}

#[derive(Serialize, Default)]
struct ProjectSummary {
    name: String,
    archived: bool,
    open: usize,
    done: usize,
    notes: usize,
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type Shared = Arc<Projects>;

/// A store or ops failure as a response: a refused write is a 400, anything else a 500.
///
/// The handlers await the store directly now, so there is no `spawn_blocking` join error to fold
/// into the 500 branch; only the capability's own refusal reaches here.
fn api_error(refused_is_bad: bool, error: Box<dyn std::error::Error + Send + Sync>) -> ApiError {
    let status = if refused_is_bad {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    ApiError(status, error.to_string())
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "projects" }))
}

async fn ready(State(projects): State<Shared>) -> Result<Json<Value>, ApiError> {
    projects.ping().await.map_err(|e| api_error(false, e))?;
    Ok(Json(json!({ "status": "ready", "service": "projects" })))
}

async fn routes() -> Json<Value> {
    Json(route_manifest::manifest("projects", ROUTES))
}

async fn list_tasks(
    State(projects): State<Shared>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<Value>, ApiError> {
    if let Some(status) = query.status.as_deref() {
        if !["open", "done"].contains(&status) {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                format!("invalid status '{status}'"),
            ));
        }
    }
    let tasks = ops::tasks(&projects, query.status.as_deref())
        .await
        .map_err(|e| api_error(false, e))?;
    Ok(Json(json!({ "tasks": tasks })))
}

async fn list_projects(State(projects): State<Shared>) -> Result<Json<Value>, ApiError> {
    let records = projects.all().await.map_err(|e| api_error(false, e))?;
    let mut summaries: Vec<ProjectSummary> = Vec::new();
    for r in records.into_iter().filter(|r| !r.domain) {
        let index = match summaries
            .iter()
            .position(|s| s.name == r.project && s.archived == r.archived)
        {
            Some(i) => i,
            None => {
                summaries.push(ProjectSummary {
                    name: r.project.clone(),
                    archived: r.archived,
                    ..Default::default()
                });
                summaries.len() - 1
            }
        };
        let s = &mut summaries[index];
        match (r.kind, r.done) {
            (Kind::Task, false) => s.open += 1,
            (Kind::Task, true) => s.done += 1,
            _ => s.notes += 1,
        }
    }
    summaries.sort_by(|a, b| (a.archived, &a.name).cmp(&(b.archived, &b.name)));
    Ok(Json(json!({ "projects": summaries })))
}

async fn list_records(
    State(projects): State<Shared>,
    Query(query): Query<KindQuery>,
) -> Result<Json<Value>, ApiError> {
    let records = ops::records(&projects, query.kind)
        .await
        .map_err(|e| api_error(false, e))?;
    Ok(Json(json!({ "records": records })))
}

async fn list_files(State(projects): State<Shared>) -> Result<Json<Value>, ApiError> {
    let files = projects.attachments().map_err(|e| api_error(false, e))?;
    Ok(Json(json!({ "files": files })))
}

/// Images and PDFs open in the page; anything else downloads. Every answer is sandboxed and
/// unsniffable, so an SVG or HTML file from the vault cannot run script in the dashboard origin.
async fn get_file(State(projects): State<Shared>, Query(query): Query<FileQuery>) -> Response {
    let path = query.path.clone();
    match projects.attachment(&path).map_err(|e| api_error(true, e)) {
        Ok(Some(bytes)) => {
            let extension = query
                .path
                .rsplit_once('.')
                .map_or("", |(_, e)| e)
                .to_ascii_lowercase();
            let (kind, inline) = match extension.as_str() {
                "png" => ("image/png", true),
                "jpg" | "jpeg" => ("image/jpeg", true),
                "gif" => ("image/gif", true),
                "webp" => ("image/webp", true),
                "svg" => ("image/svg+xml", true),
                "pdf" => ("application/pdf", true),
                _ => ("application/octet-stream", false),
            };
            let name = query
                .path
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .replace('"', "");
            let disposition = format!(
                "{}; filename=\"{name}\"",
                if inline { "inline" } else { "attachment" }
            );
            (
                [
                    (header::CONTENT_TYPE, kind.to_string()),
                    (header::CONTENT_DISPOSITION, disposition),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                    (header::CONTENT_SECURITY_POLICY, "sandbox".to_string()),
                    (header::CACHE_CONTROL, "private, max-age=3600".to_string()),
                ],
                bytes,
            )
                .into_response()
        }
        Ok(None) => ApiError(StatusCode::NOT_FOUND, "no such attachment".into()).into_response(),
        Err(e) => e.into_response(),
    }
}

/// One record in the shape `GET /api/project` and `GET /api/record` share.
///
/// `updated_at` is left out of the reads that do not carry it, so the shape stays one shape and a
/// reader that never looks at the key never has to.
fn record_json(record: &projects::Record, with_content: bool, updated_at: Option<&str>) -> Value {
    let class = ops::classify_record(record);
    let mut value = json!({
        "path": record.path, "kind": record.kind, "title": record.title,
        "done": record.done, "due": record.due, "priority": record.priority,
        "archived": record.archived,
        "data_class": class.value, "data_class_rationale": class.rationale,
    });
    if with_content {
        value["content"] = json!(record.content);
    }
    if let Some(updated_at) = updated_at {
        value["updated_at"] = json!(updated_at);
    }
    value
}

async fn get_project(
    State(projects): State<Shared>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<Value>, ApiError> {
    let name = query.name;
    let with_content = query.content.unwrap_or(true);
    let records = projects
        .by_project_timed(&name)
        .await
        .map_err(|e| api_error(false, e))?;
    if records.is_empty() {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such project".into()));
    }
    let records: Vec<Value> = records
        .iter()
        .map(|(record, updated_at)| record_json(record, with_content, Some(updated_at)))
        .collect();
    Ok(Json(json!({ "records": records })))
}

async fn get_record(
    State(projects): State<Shared>,
    Query(query): Query<RecordQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = query.path;
    let (record, updated_at) = projects
        .get_timed(&path)
        .await
        .map_err(|e| api_error(false, e))?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such record".into()))?;
    Ok(Json(
        json!({ "record": record_json(&record, true, Some(&updated_at)) }),
    ))
}

async fn add_task(
    State(projects): State<Shared>,
    Json(body): Json<AddTask>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let record = ops::add_task(
        &projects,
        &body.project,
        &body.title,
        &body.summary,
        body.due.as_deref().filter(|d| !d.is_empty()),
        body.priority.unwrap_or(2),
    )
    .await
    .map_err(|e| api_error(true, e))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "task": ops::Task::from_record(&record) })),
    ))
}

async fn set_done(
    State(projects): State<Shared>,
    Json(body): Json<SetDone>,
) -> Result<Json<Value>, ApiError> {
    let record = ops::set_done(&projects, &body.id, body.done)
        .await
        .map_err(|e| api_error(true, e))?;
    Ok(Json(json!({ "task": ops::Task::from_record(&record) })))
}

async fn add_project(
    State(projects): State<Shared>,
    Json(body): Json<AddProject>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let record = ops::add_project(&projects, &body.name, &body.summary)
        .await
        .map_err(|e| api_error(true, e))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "project": record.project })),
    ))
}

async fn save_record(State(projects): State<Shared>, Json(body): Json<SaveRecord>) -> Response {
    let outcome = ops::save(&projects, &body.path, &body.content, Some(&body.base))
        .await
        .map_err(|e| match e.downcast::<ops::SaveError>() {
            Ok(save) => *save,
            Err(other) => ops::SaveError::Refused(other.to_string()),
        });
    match outcome {
        Ok(record) => Json(json!({ "path": record.path, "kind": record.kind })).into_response(),
        Err(ops::SaveError::Stale(current)) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "the note changed since it was read; nothing was saved", "code": "stale", "current": current })),
        )
            .into_response(),
        Err(ops::SaveError::Refused(why)) => ApiError(StatusCode::BAD_REQUEST, why).into_response(),
    }
}

async fn set_property(
    State(projects): State<Shared>,
    Json(body): Json<SetProperty>,
) -> Result<Json<Value>, ApiError> {
    let record = ops::set_property(&projects, &body.path, &body.key, &body.value)
        .await
        .map_err(|e| api_error(true, e))?;
    Ok(Json(json!({ "task": ops::Task::from_record(&record) })))
}

async fn add_note(
    State(projects): State<Shared>,
    Json(body): Json<AddNote>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (path, content) = ops::new_note(&body.project, &body.title, &body.summary)
        .map_err(|why| api_error(true, why.into()))?;
    let record = ops::save(&projects, &path, &content, None)
        .await
        .map_err(|e| api_error(true, e))?;
    Ok((StatusCode::CREATED, Json(json!({ "path": record.path }))))
}

async fn remove_record(
    State(projects): State<Shared>,
    Json(body): Json<RemoveRecord>,
) -> Result<Json<Value>, ApiError> {
    let removed = projects
        .remove(&body.path)
        .await
        .map_err(|e| api_error(true, e))?;
    Ok(Json(json!({ "removed": removed })))
}

async fn attach_file(
    State(projects): State<Shared>,
    Json(body): Json<AttachFile>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let bytes =
        ops::decode_base64(&body.base64).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    let path = body.path.clone();
    let outcome = ops::attach(&projects, &path, &bytes)
        .await
        .map_err(|e| api_error(true, e))?;
    match outcome {
        projects::store::Attached::New => Ok((
            StatusCode::CREATED,
            Json(json!({ "path": body.path, "status": "created" })),
        )),
        projects::store::Attached::Unchanged => Ok((
            StatusCode::OK,
            Json(json!({ "path": body.path, "status": "unchanged" })),
        )),
        projects::store::Attached::Differs => Err(ApiError(
            StatusCode::CONFLICT,
            format!(
                "a different attachment already exists at {:?}; not overwritten",
                body.path
            ),
        )),
    }
}

fn router(projects: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/routes", get(routes))
        .route("/api/tasks", get(list_tasks).post(add_task))
        .route("/api/tasks/done", post(set_done))
        .route("/api/projects", get(list_projects).post(add_project))
        .route("/api/records", get(list_records))
        .route("/api/files", get(list_files))
        .route("/api/file", get(get_file).post(attach_file))
        .route("/api/project", get(get_project))
        .route("/api/record", get(get_record).post(save_record))
        .route("/api/record/property", post(set_property))
        .route("/api/record/remove", post(remove_record))
        .route("/api/notes", post(add_note))
        .layer(middleware::from_fn_with_state(
            "projects",
            sjel_server::origin::refuse_foreign_origins,
        ))
        .with_state(projects)
}

async fn serve() {
    let port = sjel_config::resolve_port(None, None, 8103);
    let projects = match projects::open_default().await {
        Ok(projects) => projects,
        Err(error) => {
            eprintln!("projects: {error}");
            std::process::exit(1);
        }
    };
    sjel_server::serve_local("projects", port, router(Arc::new(projects))).await;
}

fn main() {
    tokio::runtime::Runtime::new()
        .expect("tokio runtime could not start")
        .block_on(serve());
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::util::ServiceExt;

    #[test]
    fn manifest_covers_routes_and_write_schemas() {
        let source = include_str!("server.rs");
        assert!(route_manifest::undeclared_routes(source, ROUTES).is_empty());
        assert!(route_manifest::bodies_without_schemas(ROUTES).is_empty());
    }

    /// Drives the wired router with no listener and no token: `router` carries the handlers, and
    /// inbound auth is the layer `serve_local` adds around them.
    async fn call(
        router: &Router,
        request: axum::http::Request<axum::body::Body>,
    ) -> (StatusCode, Value) {
        let response = router
            .clone()
            .oneshot(request)
            .await
            .expect("router answered");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .expect("body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn post(uri: &str, body: Value) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::to_vec(&body).expect("body encodes"),
            ))
            .expect("request builds")
    }

    fn get(uri: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri(uri)
            .body(axum::body::Body::empty())
            .expect("request builds")
    }

    /// The whole write path answers through the async store: create, read back, save, refuse a
    /// stale save, and remove.
    #[tokio::test]
    async fn project_routes_answer_from_the_async_store() {
        let dir = std::env::temp_dir().join(format!("sjel-projects-router-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let projects = Projects::open(&dir.join("axon.db"), dir.join("rendered"))
            .await
            .expect("store opens");
        let router = router(Arc::new(projects));

        for (uri, expected) in [("/ready", "ready"), ("/health", "ok")] {
            let (status, body) = call(&router, get(uri)).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(body["status"], json!(expected), "{uri}");
        }

        let (status, body) = call(
            &router,
            post("/api/projects", json!({ "name": "W", "summary": "w" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["project"], json!("W"));

        let (status, body) = call(
            &router,
            post(
                "/api/tasks",
                json!({ "project": "W", "title": "x", "due": "2026-10-24", "priority": 1 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["task"]["id"], json!("W/Tasks/x.md"));

        let (status, body) = call(&router, get("/api/tasks?status=open")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["tasks"].as_array().map(Vec::len), Some(1));

        let (status, body) = call(&router, get("/api/projects")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["projects"][0]["name"], json!("W"));
        assert_eq!(body["projects"][0]["open"], json!(1));

        let (status, body) = call(
            &router,
            post(
                "/api/tasks/done",
                json!({ "id": "W/Tasks/x.md", "done": true }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["task"]["done"], json!(true));

        let (status, body) = call(&router, get("/api/record?path=W/Tasks/x.md")).await;
        assert_eq!(status, StatusCode::OK);
        let base = body["record"]["content"]
            .as_str()
            .expect("content")
            .to_string();

        let edited = format!("{base}\nmore\n");
        let (status, _) = call(
            &router,
            post(
                "/api/record",
                json!({ "path": "W/Tasks/x.md", "content": edited, "base": base }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = call(
            &router,
            post(
                "/api/record",
                json!({ "path": "W/Tasks/x.md", "content": "gone", "base": base }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["code"], json!("stale"));

        let (status, body) = call(
            &router,
            post("/api/record/remove", json!({ "path": "W/Tasks/x.md" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["removed"], json!(true));

        let (status, body) = call(&router, get("/api/tasks?status=open")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["tasks"].as_array().map(Vec::len), Some(0));

        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A replica reads the project without contents and asks which records moved, so every record
    /// the two record reads answer with carries the moment its row was last written.
    #[tokio::test]
    async fn record_reads_carry_the_moment_the_row_was_written() {
        let dir = std::env::temp_dir().join(format!("sjel-projects-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let projects = Projects::open(&dir.join("axon.db"), dir.join("rendered"))
            .await
            .expect("store opens");
        let router = router(Arc::new(projects));

        let (status, _) = call(
            &router,
            post("/api/projects", json!({ "name": "R", "summary": "r" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        for title in ["one", "two"] {
            let (status, _) = call(
                &router,
                post("/api/tasks", json!({ "project": "R", "title": title })),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
        }

        let (status, body) = call(&router, get("/api/project?name=R&content=false")).await;
        assert_eq!(status, StatusCode::OK);
        let listed = body["records"].as_array().expect("records");
        assert_eq!(listed.len(), 3, "the hub and both tasks");
        for record in listed {
            assert!(record.get("content").is_none(), "content=false: {record}");
            assert!(
                record["updated_at"].as_str().is_some_and(|s| s.len() > 20),
                "no stamp on {record}"
            );
        }
        let before = listed
            .iter()
            .find(|r| r["path"] == json!("R/Tasks/one.md"))
            .and_then(|r| r["updated_at"].as_str())
            .expect("task stamp")
            .to_string();

        let (status, body) = call(&router, get("/api/record?path=R/Tasks/two.md")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body["record"]["updated_at"].as_str().is_some(),
            "the single read stamps too: {body}"
        );
        let base = body["record"]["content"].as_str().expect("content");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let (status, _) = call(
            &router,
            post(
                "/api/record",
                json!({ "path": "R/Tasks/two.md", "content": format!("{base}\nmore\n"), "base": base }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = call(&router, get("/api/project?name=R&content=false")).await;
        assert_eq!(status, StatusCode::OK);
        let listed = body["records"].as_array().expect("records");
        let after = listed
            .iter()
            .find(|r| r["path"] == json!("R/Tasks/one.md"))
            .and_then(|r| r["updated_at"].as_str())
            .expect("task stamp");
        assert_eq!(
            after, before,
            "a write to one record does not stamp another"
        );
        let moved = listed
            .iter()
            .find(|r| r["path"] == json!("R/Tasks/two.md"))
            .and_then(|r| r["updated_at"].as_str())
            .expect("task stamp");
        assert_ne!(moved, before, "the saved record's stamp moved");

        std::fs::remove_dir_all(dir).unwrap();
    }
}
