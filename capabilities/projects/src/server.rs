use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use projects::{ops, Kind, Projects};

mod inventory_links;

const AGENT_ROUTES: sjel_server::agent_policy::AgentRoutes =
    sjel_server::agent_policy::AgentRoutes {
        confirm: &[],
        get_writes: &[],
        closed: &[
            ("GET", "/api/mail-proposals"),
            ("POST", "/api/mail-proposals"),
            ("POST", "/api/mail-proposals/withdraw"),
            ("POST", "/api/mail-proposals/{id}/decision"),
            ("GET", "/api/inventory-links"),
            ("POST", "/api/inventory-links"),
            ("POST", "/api/inventory-links/remove"),
            ("GET", "/api/links"),
        ],
    };

const ROUTES: &[route_manifest::Route] = &[
    route_manifest::get("GET", "/health", "Liveness."),
    route_manifest::get("GET", "/ready", "Liveness plus database readiness."),
    route_manifest::get("GET", "/routes", "This manifest."),
    route_manifest::get("GET", "/api/links", "Backreferences to an Inventory typed ID; stored references do not guarantee current existence."),
    route_manifest::get("GET", "/api/inventory-links", "Reviewed stored Inventory references for a project record. ?path= selects the record."),
    route_manifest::Route { method: "POST", path: "/api/inventory-links", summary: "Add a reviewed Inventory reference after an owner lookup and record revision check.", request_schema: Some(route_manifest::schema_of::<InventoryLinkDecision>) },
    route_manifest::Route { method: "POST", path: "/api/inventory-links/remove", summary: "Remove a reviewed Inventory reference with a record revision check.", request_schema: Some(route_manifest::schema_of::<InventoryLinkDecision>) },
    route_manifest::get("GET", "/api/mail-proposals", "List up to 500 mail proposals, newest first; ?before= is an exclusive id cursor."),
    route_manifest::Route { method: "POST", path: "/api/mail-proposals", summary: "Stage project mail evidence without changing a project.", request_schema: Some(route_manifest::schema_of::<projects::store::MailProposalInput>) },
    route_manifest::Route { method: "POST", path: "/api/mail-proposals/withdraw", summary: "Delete pending and rejected proposals of one mail thread.", request_schema: Some(route_manifest::schema_of::<MailWithdraw>) },
    route_manifest::Route { method: "POST", path: "/api/mail-proposals/{id}/decision", summary: "Reject a pending version, or approve it against a selected record and exact base content.", request_schema: Some(route_manifest::schema_of::<MailDecision>) },
    route_manifest::get(
        "GET",
        "/api/tasks",
        "Tasks of live projects in the vault reader's shape. ?status=open|done filters.",
    ),
    route_manifest::get(
        "GET",
        "/api/projects",
        "Every project with its open and done task counts, its note count and its strictest record class.",
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
        summary: "Replace a record's whole content. `base` is the content the edit started from; a stale base returns 409 with the current content and writes nothing. Without `base` it creates the record at that path, and refuses (400) when the path exists.",
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
#[serde(deny_unknown_fields)]
struct InventoryLinkDecision {
    project: String,
    path: String,
    base: String,
    updated_at: String,
    item_ref: String,
    reviewed: bool,
}

#[derive(Deserialize)]
struct InventoryLinkQuery {
    path: String,
}

#[derive(Deserialize)]
struct LinksQuery {
    to: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct MailDecision {
    version: i64,
    decision: String,
    target_path: Option<String>,
    base: Option<String>,
}

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
    /// The content the edit started from. Absent creates the record, and is refused when the path
    /// already exists, so a create can never overwrite.
    #[serde(default)]
    base: Option<String>,
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
struct MailQuery {
    before: Option<i64>,
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
    /// The strictest class of the project's records, so a reader can keep the name out of a
    /// context that class may not enter.
    data_class: String,
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

async fn inventory_link_list(
    State(projects): State<Shared>,
    Query(query): Query<InventoryLinkQuery>,
) -> Result<Json<Value>, ApiError> {
    let links = projects
        .inventory_links(None, Some(&query.path))
        .await
        .map_err(|e| api_error(false, e))?;
    let links: Vec<_> = links.into_iter().map(|(link, _)| link).collect();
    Ok(Json(
        json!({"links": links, "target_status": "not_checked"}),
    ))
}

async fn inventory_backrefs(
    State(projects): State<Shared>,
    Query(query): Query<LinksQuery>,
) -> Result<Json<sjel_links::Links>, ApiError> {
    let target = sjel_links::target(query.to.as_deref())
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;
    inventory_links::decode_item_ref(target.id)
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.into()))?;
    let rows = projects
        .inventory_links(Some(target.id), None)
        .await
        .map_err(|e| api_error(false, e))?;
    let links = rows
        .into_iter()
        .filter(|(_, r)| !r.archived && !r.domain && r.kind == Kind::Project)
        .map(|(_, r)| sjel_links::Link {
            id: inventory_links::project_ref(&r.path),
            kind: "project".into(),
            title: r.title,
            at: None,
            meta: Some("Stored reference; Inventory target not checked".into()),
            via: "projects_inventory_links",
        })
        .collect();
    Ok(Json(sjel_links::Links {
        to: target.id.into(),
        links,
        unlinkable: 0,
    }))
}

async fn inventory_link_add(
    State(projects): State<Shared>,
    Json(body): Json<InventoryLinkDecision>,
) -> Result<Json<Value>, ApiError> {
    inventory_link_decide(projects, body, true).await
}

async fn inventory_link_remove(
    State(projects): State<Shared>,
    Json(body): Json<InventoryLinkDecision>,
) -> Result<Json<Value>, ApiError> {
    inventory_link_decide(projects, body, false).await
}

async fn inventory_link_decide(
    projects: Shared,
    body: InventoryLinkDecision,
    add: bool,
) -> Result<Json<Value>, ApiError> {
    let base = sjel_config::env_var("SJEL_INVENTORY_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8101".into());
    inventory_link_decide_at(
        projects,
        body,
        add,
        base,
        sjel_server::InboundAuth::from_deployment(),
    )
    .await
}

async fn inventory_link_decide_at(
    projects: Shared,
    body: InventoryLinkDecision,
    add: bool,
    base: String,
    auth: sjel_server::InboundAuth,
) -> Result<Json<Value>, ApiError> {
    if !body.reviewed
        || body.path.is_empty()
        || body.path.len() > 4096
        || body.project.is_empty()
        || body.project.len() > 255
        || body.updated_at.is_empty()
        || body.updated_at.len() > 128
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "explicit review, project, path and revision are required".into(),
        ));
    }
    inventory_links::decode_item_ref(&body.item_ref)
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.into()))?;
    let verified = if add {
        let reference = body.item_ref.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                inventory_links::verify_with_auth(&base, &reference, auth)
            })
            .await
            .map_err(|_| {
                ApiError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Inventory verification failed".into(),
                )
            })?
            .map_err(|error| match error {
                inventory_links::VerifyError::Invalid => ApiError(
                    StatusCode::BAD_REQUEST,
                    "invalid Inventory reference".into(),
                ),
                inventory_links::VerifyError::Missing => {
                    ApiError(StatusCode::NOT_FOUND, "Inventory item not found".into())
                }
                inventory_links::VerifyError::NotOwned => {
                    ApiError(StatusCode::CONFLICT, "Inventory item is not owned".into())
                }
                inventory_links::VerifyError::Unavailable => ApiError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Inventory verification unavailable or inconsistent".into(),
                ),
            })?,
        )
    } else {
        None
    };
    let changed = projects
        .decide_inventory_link(
            projects::store::InventoryLinkTarget {
                path: &body.path,
                project: &body.project,
                base: &body.base,
                revision: &body.updated_at,
                add,
            },
            &body.item_ref,
            verified,
        )
        .await
        .map_err(|e| api_error(false, e))?
        .ok_or_else(|| {
            ApiError(
                StatusCode::CONFLICT,
                "project record missing, not live, or changed since review".into(),
            )
        })?;
    Ok(Json(
        json!({"changed": changed, "item_ref": body.item_ref, "target_status": if add { "checked_at_review" } else { "not_checked" }, "verified_revision": verified}),
    ))
}

async fn stage_mail_proposal(
    State(projects): State<Shared>,
    Json(body): Json<projects::store::MailProposalInput>,
) -> Result<Json<Value>, ApiError> {
    body.validate()
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.into()))?;
    let proposal = projects
        .stage_mail_proposal(&body)
        .await
        .map_err(|e| api_error(true, e))?
        .ok_or_else(|| {
            ApiError(
                StatusCode::CONFLICT,
                "identity already has different evidence".into(),
            )
        })?;
    Ok(Json(json!({"proposal": proposal})))
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct MailWithdraw {
    source_prefix: String,
}

async fn withdraw_mail_proposals(
    State(projects): State<Shared>,
    Json(body): Json<MailWithdraw>,
) -> Result<Json<Value>, ApiError> {
    if !projects::store::mail_thread_prefix(&body.source_prefix) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "source_prefix must be mail:v1:<thread>:".into(),
        ));
    }
    let withdrawn = projects
        .withdraw_mail_thread(&body.source_prefix)
        .await
        .map_err(|e| api_error(false, e))?;
    Ok(Json(json!({ "withdrawn": withdrawn })))
}

async fn list_mail_proposals(
    State(projects): State<Shared>,
    Query(query): Query<MailQuery>,
) -> Result<Json<Value>, ApiError> {
    let proposals = projects
        .mail_proposals(query.before.unwrap_or(i64::MAX))
        .await
        .map_err(|e| api_error(false, e))?;
    Ok(Json(json!({"proposals": proposals, "limit": 500})))
}

async fn decide_mail_proposal(
    State(projects): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<MailDecision>,
) -> Result<Json<Value>, ApiError> {
    if !["approve", "reject"].contains(&body.decision.as_str()) || body.version < 1 || id < 1 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "invalid decision or version".into(),
        ));
    }
    let target = match (
        body.decision.as_str(),
        body.target_path.as_deref(),
        body.base.as_deref(),
    ) {
        ("approve", Some(path), Some(base)) if !path.is_empty() && path.len() <= 4096 => {
            Some((path, base))
        }
        ("reject", None, None) => None,
        _ => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "approval requires target_path and base; rejection accepts neither".into(),
            ))
        }
    };
    let outcome = projects
        .decide_mail_proposal(id, body.version, &body.decision, target)
        .await
        .map_err(|e| api_error(false, e))?
        .ok_or_else(|| ApiError(StatusCode::CONFLICT, "proposal or target is stale, missing, already reviewed, outside the live project, or has a lower data class".into()))?;
    let applied = outcome.target_path.is_some();
    let rendered = applied && outcome.render_error.is_none();
    Ok(Json(
        json!({"proposal": outcome.proposal, "applied": applied, "target_path": outcome.target_path, "rendered": rendered, "render_error": outcome.render_error}),
    ))
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
                    data_class: "c0".into(),
                    ..Default::default()
                });
                summaries.len() - 1
            }
        };
        let s = &mut summaries[index];
        // Class literals order by strictness: c0 < c1 < c2 < c3.
        s.data_class = s.data_class.clone().max(ops::classify_record(&r).value);
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
    let outcome = ops::save(&projects, &body.path, &body.content, body.base.as_deref())
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
        .route(
            "/api/mail-proposals",
            get(list_mail_proposals).post(stage_mail_proposal),
        )
        .route(
            "/api/mail-proposals/withdraw",
            post(withdraw_mail_proposals),
        )
        .route(
            "/api/mail-proposals/{id}/decision",
            post(decide_mail_proposal),
        )
        .route("/api/links", get(inventory_backrefs))
        .route(
            "/api/inventory-links",
            get(inventory_link_list).post(inventory_link_add),
        )
        .route("/api/inventory-links/remove", post(inventory_link_remove))
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
    sjel_server::serve_local_with("projects", port, router(Arc::new(projects)), AGENT_ROUTES).await;
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
        assert!(AGENT_ROUTES.closes("GET", "/api/mail-proposals"));
        assert!(AGENT_ROUTES.closes("POST", "/api/mail-proposals"));
        assert!(AGENT_ROUTES.closes("POST", "/api/mail-proposals/7/decision"));
        assert!(AGENT_ROUTES.closes("POST", "/api/mail-proposals/withdraw"));
        for (method, path) in [
            ("GET", "/api/links"),
            ("GET", "/api/inventory-links"),
            ("POST", "/api/inventory-links"),
            ("POST", "/api/inventory-links/remove"),
        ] {
            assert!(AGENT_ROUTES.closes(method, path));
        }
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

    #[tokio::test]
    async fn reviewed_add_verifies_owner_before_writing() {
        use std::io::{Read, Write};
        let dir =
            std::env::temp_dir().join(format!("sjel-projects-link-verify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let projects = Arc::new(
            Projects::open(&dir.join("axon.db"), dir.join("rendered"))
                .await
                .unwrap(),
        );
        let path = "W/W.md";
        let content = "---\ntype: project\n---\n";
        projects
            .put(&projects::Record::from_content(path, content.into()).unwrap())
            .await
            .unwrap();
        let (_, revision) = projects.get_timed(path).await.unwrap().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let size = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..size]).to_ascii_lowercase();
            assert!(request.contains("authorization: bearer synthetic-token"));
            let body = r#"{"typed_ref":"inv:61","raw_id":"a","revision":4,"state":"owned"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let body = InventoryLinkDecision {
            project: "W".into(),
            path: path.into(),
            base: content.into(),
            updated_at: revision,
            item_ref: "inv:61".into(),
            reviewed: true,
        };
        let result = inventory_link_decide_at(
            projects.clone(),
            body,
            true,
            format!("http://{address}"),
            sjel_server::InboundAuth::with_token(Some("synthetic-token".into())),
        )
        .await
        .ok()
        .unwrap();
        assert_eq!(result.0["changed"], true);
        assert_eq!(result.0["verified_revision"], 4);
        server.join().unwrap();
        assert_eq!(
            projects
                .inventory_links(Some("inv:61"), None)
                .await
                .unwrap()
                .len(),
            1
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn reviewed_inventory_links_are_guarded_and_report_backreferences() {
        let dir = std::env::temp_dir().join(format!("sjel-projects-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let projects = Arc::new(
            Projects::open(&dir.join("axon.db"), dir.join("rendered"))
                .await
                .unwrap(),
        );
        let path = "Ré:pair/Hub.md";
        let content = "---\ntype: project\n---\n\nRepair\n";
        projects
            .put(&projects::Record::from_content(path, content.into()).unwrap())
            .await
            .unwrap();
        let (_, revision) = projects.get_timed(path).await.unwrap().unwrap();
        macro_rules! decide {
            ($base:expr, $rev:expr, $add:expr) => {
                projects
                    .decide_inventory_link(
                        projects::store::InventoryLinkTarget {
                            path,
                            project: "Ré:pair",
                            base: $base,
                            revision: $rev,
                            add: $add,
                        },
                        "inv:61",
                        Some(4),
                    )
                    .await
                    .unwrap()
            };
        }
        assert_eq!(decide!("stale", &revision, true), None);
        assert_eq!(decide!(content, "stale", true), None);
        assert_eq!(decide!(content, &revision, true), Some(true));
        assert_eq!(decide!(content, &revision, true), Some(false));
        let router = router(projects.clone());
        let (status, body) = call(&router, get("/api/links?to=inv:61")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["to"], "inv:61");
        assert_eq!(body["unlinkable"], 0);
        assert_eq!(body["links"][0]["id"], inventory_links::project_ref(path));
        assert_eq!(body["links"][0]["kind"], "project");
        assert_eq!(body["links"][0]["via"], "projects_inventory_links");
        assert_eq!(
            call(&router, get("/api/links?to=inv:6A")).await.0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(&router, get("/api/links")).await.0,
            StatusCode::BAD_REQUEST
        );
        let remove = json!({"project":"Ré:pair", "path":path, "base":content, "updated_at":revision, "item_ref":"inv:61", "reviewed":true});
        let mut stale_remove = remove.clone();
        stale_remove["base"] = json!("old");
        assert_eq!(
            call(&router, post("/api/inventory-links/remove", stale_remove))
                .await
                .0,
            StatusCode::CONFLICT
        );
        let mut unreviewed = remove.clone();
        unreviewed["reviewed"] = json!(false);
        assert_eq!(
            call(&router, post("/api/inventory-links/remove", unreviewed))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(&router, post("/api/inventory-links/remove", remove.clone()))
                .await
                .1["changed"],
            true
        );
        assert_eq!(
            call(&router, post("/api/inventory-links/remove", remove))
                .await
                .1["changed"],
            false
        );
        assert_eq!(
            call(&router, get("/api/links?to=inv:61")).await.1["links"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(projects.get(path).await.unwrap().unwrap().content, content);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn mail_proposals_apply_once_against_the_reviewed_record() {
        let dir = std::env::temp_dir().join(format!("sjel-projects-mail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let projects = Projects::open(&dir.join("axon.db"), dir.join("rendered"))
            .await
            .unwrap();
        let router = router(Arc::new(projects));
        let proposal = json!({"source_id":"message", "line_id":"line-1", "event":"project_evidence", "project":"W", "title":"Delivery", "evidence":"Delivered today", "data_class":"c2", "task_suggestion":"Inspect part"});
        assert_eq!(
            call(&router, post("/api/mail-proposals", proposal.clone()))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(&router, post("/api/projects", json!({"name":"W"})))
                .await
                .0,
            StatusCode::CREATED
        );
        let (status, body) = call(&router, post("/api/mail-proposals", proposal.clone())).await;
        assert_eq!(status, StatusCode::OK);
        let id = body["proposal"]["id"].as_i64().unwrap();
        assert_eq!(body["proposal"]["version"], 1);
        assert_eq!(
            call(&router, post("/api/mail-proposals", proposal.clone()))
                .await
                .1["proposal"]["id"],
            id
        );
        let mut changed = proposal.clone();
        changed["evidence"] = json!("different");
        assert_eq!(
            call(&router, post("/api/mail-proposals", changed)).await.0,
            StatusCode::CONFLICT
        );
        let mut forbidden = proposal.clone();
        forbidden["data_class"] = json!("c3");
        assert_eq!(
            call(&router, post("/api/mail-proposals", forbidden))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        let base = "---\nclass: c2\n---\n\nExisting note\n";
        assert_eq!(
            call(
                &router,
                post(
                    "/api/record",
                    json!({"path":"W/Notes/evidence.md", "content":base})
                )
            )
            .await
            .0,
            StatusCode::OK
        );
        let mut next = proposal.clone();
        next["line_id"] = json!("line-2");
        let (status, second) = call(&router, post("/api/mail-proposals", next.clone())).await;
        assert_eq!(status, StatusCode::OK);
        let second_id = second["proposal"]["id"].as_i64().unwrap();
        let (_, rejected) = call(
            &router,
            post(
                &format!("/api/mail-proposals/{second_id}/decision"),
                json!({"version":1,"decision":"reject"}),
            ),
        )
        .await;
        assert_eq!(rejected["proposal"]["status"], "rejected");
        assert_eq!(
            call(&router, post("/api/mail-proposals", next)).await.1["proposal"]["status"],
            "rejected"
        );
        assert_eq!(
            call(&router, get("/api/record?path=W/Notes/evidence.md"))
                .await
                .1["record"]["content"],
            base
        );
        let decision = format!("/api/mail-proposals/{id}/decision");
        let approval = json!({"version":1,"decision":"approve","target_path":"W/Notes/evidence.md","base":base});
        let mut stale = approval.clone();
        stale["version"] = json!(2);
        assert_eq!(
            call(&router, post(&decision, stale)).await.0,
            StatusCode::CONFLICT
        );
        let mut stale = approval.clone();
        stale["base"] = json!("old content");
        assert_eq!(
            call(&router, post(&decision, stale)).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            call(&router, post("/api/projects", json!({"name":"Other"})))
                .await
                .0,
            StatusCode::CREATED
        );
        let other_base = call(&router, get("/api/record?path=Other/Other.md"))
            .await
            .1["record"]["content"]
            .clone();
        let mut wrong_project = approval.clone();
        wrong_project["target_path"] = json!("Other/Other.md");
        wrong_project["base"] = other_base;
        assert_eq!(
            call(&router, post(&decision, wrong_project)).await.0,
            StatusCode::CONFLICT
        );
        let mut wrong_class = approval.clone();
        wrong_class["target_path"] = json!("W/W.md");
        wrong_class["base"] =
            call(&router, get("/api/record?path=W/W.md")).await.1["record"]["content"].clone();
        let before_c1 = wrong_class["base"].clone();
        assert_eq!(
            call(&router, post(&decision, wrong_class)).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            call(&router, get("/api/record?path=W/W.md")).await.1["record"]["content"],
            before_c1
        );
        let (_, pending) = call(&router, get("/api/mail-proposals")).await;
        assert!(pending["proposals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == id && p["status"] == "pending" && p["version"] == 1));
        let (status, body) = call(&router, post(&decision, approval.clone())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["proposal"]["status"], "approved_applied");
        assert_eq!(body["applied"], true);
        assert_eq!(body["rendered"], true);
        let content = call(&router, get("/api/record?path=W/Notes/evidence.md"))
            .await
            .1["record"]["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            content.contains("Delivered today")
                && content.contains("message")
                && content.contains("line-1")
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("rendered/W/Notes/evidence.md")).unwrap(),
            content
        );
        assert_eq!(
            call(&router, post(&decision, approval)).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(content.matches("Delivered today").count(), 1);
        assert_eq!(
            call(
                &router,
                post(&decision, json!({"version":1,"decision":"reject"}))
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            call(&router, post("/api/mail-proposals", proposal)).await.1["proposal"]["status"],
            "approved_applied"
        );
        assert_eq!(
            call(&router, get("/api/mail-proposals")).await.1["proposals"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            call(&router, get("/api/tasks")).await.1["tasks"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn c2_evidence_is_refused_for_a_c1_record() {
        let dir =
            std::env::temp_dir().join(format!("sjel-projects-mail-class-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let projects = Projects::open(&dir.join("axon.db"), dir.join("rendered"))
            .await
            .unwrap();
        let router = router(Arc::new(projects));
        assert_eq!(
            call(&router, post("/api/projects", json!({"name":"W"})))
                .await
                .0,
            StatusCode::CREATED
        );
        let before = call(&router, get("/api/record?path=W/W.md")).await.1["record"].clone();
        assert_eq!(before["data_class"], "c1");
        let proposal = json!({"source_id":"m","line_id":"1","event":"project_evidence","project":"W","title":"t","evidence":"private excerpt","data_class":"c2"});
        let (_, staged) = call(&router, post("/api/mail-proposals", proposal)).await;
        let id = staged["proposal"]["id"].as_i64().unwrap();
        let (status, _) = call(&router, post(&format!("/api/mail-proposals/{id}/decision"), json!({"version":1,"decision":"approve","target_path":"W/W.md","base":before["content"]}))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            call(&router, get("/api/record?path=W/W.md")).await.1["record"],
            before
        );
        let (_, listed) = call(&router, get("/api/mail-proposals")).await;
        assert_eq!(listed["proposals"][0]["status"], "pending");
        assert_eq!(listed["proposals"][0]["version"], 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn project_list_carries_class_and_withdraw_purges_one_thread() {
        let dir = std::env::temp_dir().join(format!(
            "sjel-projects-mail-withdraw-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let projects = Projects::open(&dir.join("axon.db"), dir.join("rendered"))
            .await
            .unwrap();
        let router = router(Arc::new(projects));
        call(&router, post("/api/projects", json!({"name":"W"}))).await;
        let class = |listed: Value| listed["projects"][0]["data_class"].clone();
        assert_eq!(class(call(&router, get("/api/projects")).await.1), "c1");
        let (status, _) = call(
            &router,
            post(
                "/api/record/property",
                json!({"path":"W/W.md","key":"class","value":"c3"}),
            ),
        )
        .await;
        assert!(status.is_success());
        assert_eq!(class(call(&router, get("/api/projects")).await.1), "c3");

        let stage = |source: &str, line: &str| json!({"source_id":source,"line_id":line,"event":"project_evidence","project":"W","title":"t","evidence":"e","data_class":"c1"});
        let mut ids = Vec::new();
        for (source, line) in [
            ("mail:v1:t_1:m", "a"),
            ("mail:v1:t_1:m", "b"),
            ("mail:v1:tx1:m", "c"),
        ] {
            let (_, staged) = call(&router, post("/api/mail-proposals", stage(source, line))).await;
            ids.push(staged["proposal"]["id"].as_i64().unwrap());
        }
        call(
            &router,
            post(
                &format!("/api/mail-proposals/{}/decision", ids[1]),
                json!({"version":1,"decision":"reject"}),
            ),
        )
        .await;
        for bad in ["mail:v1:", "mail:v1:t_1", "other:t_1:"] {
            let (status, _) = call(
                &router,
                post("/api/mail-proposals/withdraw", json!({"source_prefix":bad})),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        }
        let (status, body) = call(
            &router,
            post(
                "/api/mail-proposals/withdraw",
                json!({"source_prefix":"mail:v1:t_1:"}),
            ),
        )
        .await;
        assert_eq!(
            (status, body["withdrawn"].clone()),
            (StatusCode::OK, json!(2))
        );
        let (_, listed) = call(&router, get("/api/mail-proposals")).await;
        let left: Vec<_> = listed["proposals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_i64().unwrap())
            .collect();
        assert_eq!(left, vec![ids[2]]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn approval_reports_a_committed_write_when_render_fails() {
        let dir =
            std::env::temp_dir().join(format!("sjel-projects-mail-render-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let projects = Projects::open(&dir.join("axon.db"), dir.join("rendered"))
            .await
            .unwrap();
        let router = router(Arc::new(projects));
        assert_eq!(
            call(&router, post("/api/projects", json!({"name":"W"})))
                .await
                .0,
            StatusCode::CREATED
        );
        let base = "# Base\n";
        assert_eq!(
            call(
                &router,
                post("/api/record", json!({"path":"W/Notes/a.md","content":base}))
            )
            .await
            .0,
            StatusCode::OK
        );
        let (_, staged) = call(&router, post("/api/mail-proposals", json!({"source_id":"m","line_id":"1","event":"project_evidence","project":"W","title":"t","evidence":"quoted","data_class":"c1"}))).await;
        let id = staged["proposal"]["id"].as_i64().unwrap();
        std::fs::remove_dir_all(dir.join("rendered/W/Notes")).unwrap();
        std::fs::write(dir.join("rendered/W/Notes"), "blocker").unwrap();
        let request =
            json!({"version":1,"decision":"approve","target_path":"W/Notes/a.md","base":base});
        let (status, result) = call(
            &router,
            post(
                &format!("/api/mail-proposals/{id}/decision"),
                request.clone(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["applied"], true);
        assert_eq!(result["rendered"], false);
        assert!(result["render_error"].is_string());
        assert!(
            call(&router, get("/api/record?path=W/Notes/a.md")).await.1["record"]["content"]
                .as_str()
                .unwrap()
                .contains("quoted")
        );
        assert_eq!(
            call(
                &router,
                post(&format!("/api/mail-proposals/{id}/decision"), request)
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
        std::fs::remove_dir_all(dir).unwrap();
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

        // No base creates, and only where nothing is yet.
        let created = json!({ "path": "W/Notes/new.md", "content": "# New\n" });
        let (status, _) = call(&router, post("/api/record", created.clone())).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&router, post("/api/record", created)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

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
