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
        "One record, content included. ?path= names it.",
    ),
    route_manifest::get(
        "GET",
        "/api/project",
        "One project's records. ?name= selects it; ?content=false leaves every content out, for a list that only draws titles.",
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

/// Run blocking store work; a refused write is a 400, anything else a 500.
async fn blocking<T: Send + 'static>(
    projects: Shared,
    refused_is_bad: bool,
    work: impl FnOnce(&Projects) -> Result<T, Box<dyn std::error::Error>> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(move || work(&projects).map_err(|e| e.to_string()))
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "projects operation failed".into(),
            )
        })?
        .map_err(|message| {
            let status = if refused_is_bad {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            ApiError(status, message)
        })
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "projects" }))
}

async fn ready(State(projects): State<Shared>) -> Result<Json<Value>, ApiError> {
    blocking(projects, false, |p| p.ping()).await?;
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
    let tasks = blocking(projects, false, move |p| {
        ops::tasks(p, query.status.as_deref())
    })
    .await?;
    Ok(Json(json!({ "tasks": tasks })))
}

async fn list_projects(State(projects): State<Shared>) -> Result<Json<Value>, ApiError> {
    let records = blocking(projects, false, |p| p.all()).await?;
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
    let records = blocking(projects, false, move |p| ops::records(p, query.kind)).await?;
    Ok(Json(json!({ "records": records })))
}

async fn list_files(State(projects): State<Shared>) -> Result<Json<Value>, ApiError> {
    let files = blocking(projects, false, |p| p.attachments()).await?;
    Ok(Json(json!({ "files": files })))
}

/// Images and PDFs open in the page; anything else downloads. Every answer is sandboxed and
/// unsniffable, so an SVG or HTML file from the vault cannot run script in the dashboard origin.
async fn get_file(State(projects): State<Shared>, Query(query): Query<FileQuery>) -> Response {
    let path = query.path.clone();
    match blocking(projects, true, move |p| p.attachment(&path)).await {
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
fn record_json(record: &projects::Record, with_content: bool) -> Value {
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
    value
}

async fn get_project(
    State(projects): State<Shared>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<Value>, ApiError> {
    let name = query.name;
    let with_content = query.content.unwrap_or(true);
    let records = blocking(projects, false, move |p| p.by_project(&name)).await?;
    if records.is_empty() {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such project".into()));
    }
    let records: Vec<Value> = records
        .iter()
        .map(|record| record_json(record, with_content))
        .collect();
    Ok(Json(json!({ "records": records })))
}

async fn get_record(
    State(projects): State<Shared>,
    Query(query): Query<RecordQuery>,
) -> Result<Json<Value>, ApiError> {
    let path = query.path;
    let record = blocking(projects, false, move |p| p.get(&path))
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such record".into()))?;
    Ok(Json(json!({ "record": record_json(&record, true) })))
}

async fn add_task(
    State(projects): State<Shared>,
    Json(body): Json<AddTask>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let record = blocking(projects, true, move |p| {
        ops::add_task(
            p,
            &body.project,
            &body.title,
            &body.summary,
            body.due.as_deref().filter(|d| !d.is_empty()),
            body.priority.unwrap_or(2),
        )
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "task": ops::Task::from_record(&record) })),
    ))
}

async fn set_done(
    State(projects): State<Shared>,
    Json(body): Json<SetDone>,
) -> Result<Json<Value>, ApiError> {
    let record = blocking(projects, true, move |p| {
        ops::set_done(p, &body.id, body.done)
    })
    .await?;
    Ok(Json(json!({ "task": ops::Task::from_record(&record) })))
}

async fn add_project(
    State(projects): State<Shared>,
    Json(body): Json<AddProject>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let record = blocking(projects, true, move |p| {
        ops::add_project(p, &body.name, &body.summary)
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "project": record.project })),
    ))
}

async fn save_record(State(projects): State<Shared>, Json(body): Json<SaveRecord>) -> Response {
    let outcome = tokio::task::spawn_blocking(move || {
        ops::save(&projects, &body.path, &body.content, Some(&body.base)).map_err(|e| {
            match e.downcast::<ops::SaveError>() {
                Ok(save) => *save,
                Err(other) => ops::SaveError::Refused(other.to_string()),
            }
        })
    })
    .await;
    match outcome {
        Ok(Ok(record)) => Json(json!({ "path": record.path, "kind": record.kind })).into_response(),
        Ok(Err(ops::SaveError::Stale(current))) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "the note changed since it was read; nothing was saved", "code": "stale", "current": current })),
        )
            .into_response(),
        Ok(Err(ops::SaveError::Refused(why))) => ApiError(StatusCode::BAD_REQUEST, why).into_response(),
        Err(_) => ApiError(StatusCode::INTERNAL_SERVER_ERROR, "projects operation failed".into()).into_response(),
    }
}

async fn set_property(
    State(projects): State<Shared>,
    Json(body): Json<SetProperty>,
) -> Result<Json<Value>, ApiError> {
    let record = blocking(projects, true, move |p| {
        ops::set_property(p, &body.path, &body.key, &body.value)
    })
    .await?;
    Ok(Json(json!({ "task": ops::Task::from_record(&record) })))
}

async fn add_note(
    State(projects): State<Shared>,
    Json(body): Json<AddNote>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let record = blocking(projects, true, move |p| {
        let (path, content) = ops::new_note(&body.project, &body.title, &body.summary)?;
        ops::save(p, &path, &content, None)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "path": record.path }))))
}

async fn remove_record(
    State(projects): State<Shared>,
    Json(body): Json<RemoveRecord>,
) -> Result<Json<Value>, ApiError> {
    let removed = blocking(projects, true, move |p| p.remove(&body.path)).await?;
    Ok(Json(json!({ "removed": removed })))
}

async fn attach_file(
    State(projects): State<Shared>,
    Json(body): Json<AttachFile>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let bytes =
        ops::decode_base64(&body.base64).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    let path = body.path.clone();
    let outcome = blocking(projects, true, move |p| ops::attach(p, &path, &bytes)).await?;
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
    let projects = match projects::open_default() {
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

    #[test]
    fn manifest_covers_routes_and_write_schemas() {
        let source = include_str!("server.rs");
        assert!(route_manifest::undeclared_routes(source, ROUTES).is_empty());
        assert!(route_manifest::bodies_without_schemas(ROUTES).is_empty());
    }
}
