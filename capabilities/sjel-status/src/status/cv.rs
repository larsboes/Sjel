//! The CV page's backend. Every read and write shells `capabilities/cv/cv`, the same launcher
//! the CLI and the `cv-builder` skill drive, so the page cannot disagree with them.

use super::*;
use axum::{
    extract::Query,
    http::header,
    response::{IntoResponse, Response},
};

const CV: &str = "capabilities/cv/cv";

async fn run_cv(args: &[String]) -> Result<(bool, String, String), (StatusCode, Json<Value>)> {
    run_cv_with_input(args, None).await
}

async fn run_cv_with_input(
    args: &[String],
    input: Option<&str>,
) -> Result<(bool, String, String), (StatusCode, Json<Value>)> {
    use tokio::io::AsyncWriteExt;
    let root = axon_root().map_err(bad_gateway)?;
    let mut child = tokio::process::Command::new(root.join(CV))
        .args(args)
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| bad_gateway(format!("could not run {CV}: {e}")))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    if let Some(text) = input {
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|e| bad_gateway(format!("could not write to {CV}: {e}")))?;
    }
    drop(stdin);
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| bad_gateway(format!("could not run {CV}: {e}")))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}

async fn cv_status() -> Result<Value, (StatusCode, Json<Value>)> {
    let (ok, stdout, stderr) = run_cv(&["status".into(), "--json".into()]).await?;
    if !ok {
        return Err(bad_gateway(format!("{CV} status failed: {stderr}")));
    }
    serde_json::from_str(&stdout).map_err(|e| bad_gateway(format!("{CV} did not emit JSON: {e}")))
}

/// `GET /api/sjel-status/cv` — profiles, applications, built PDFs and their staleness.
pub(crate) async fn cv_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    cv_status().await.map(Json)
}

/// Body of `POST /api/sjel-status/cv`. Fields not used by an action are ignored.
#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct CvRequest {
    pub(crate) action: String,
    pub(crate) profile: String,
    pub(crate) lang: String,
    pub(crate) slug: String,
    pub(crate) company: String,
    pub(crate) role: String,
    pub(crate) url: String,
    pub(crate) status: String,
    pub(crate) note: String,
}

/// Run one `cv` verb. A refusal from the launcher is returned as a 400 with its own words.
pub(crate) async fn cv_write_handler(
    Json(request): Json<CvRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let args = plan_cv(&request).map_err(bad_request)?;
    let (ok, stdout, stderr) = run_cv(&args).await?;
    if !ok {
        return Err(bad_request(if stderr.is_empty() { stdout } else { stderr }));
    }
    Ok(Json(json!({ "action": request.action, "detail": stdout, "warnings": stderr })))
}

/// The launcher argv for a request. Separate from the handler so the mapping is tested
/// without building PDFs or calling LinkedIn.
fn plan_cv(r: &CvRequest) -> Result<Vec<String>, String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let lang = if r.lang.is_empty() { "en" } else { r.lang.as_str() };
    if !matches!(lang, "en" | "de") {
        return Err(format!("lang '{lang}' is not en or de"));
    }
    match r.action.as_str() {
        "build" => {
            plain_name(&r.profile, "profile")?;
            Ok(s(&["build", "--profile", &r.profile, "--lang", lang]))
        }
        "build-all" => Ok(s(&["build", "--all"])),
        "app-new" => {
            plain_name(&r.slug, "slug")?;
            plain_name(&r.profile, "profile")?;
            let mut args = s(&[
                "app", "new", &r.slug, "--company", &r.company, "--role", &r.role,
                "--profile", &r.profile, "--lang", lang,
            ]);
            if !r.url.is_empty() {
                args.extend(s(&["--url", &r.url]));
            }
            Ok(args)
        }
        "app-build" => {
            plain_name(&r.slug, "slug")?;
            Ok(s(&["app", "build", &r.slug]))
        }
        "app-status" => {
            plain_name(&r.slug, "slug")?;
            plain_name(&r.status, "status")?;
            Ok(s(&["app", "status", &r.slug, &r.status, &r.note]))
        }
        "linkedin-pull" => Ok(s(&["linkedin", "pull"])),
        "linkedin-diff" => Ok(s(&["linkedin", "diff"])),
        "linkedin-sheet" => Ok(s(&["linkedin", "sheet", "--lang", lang])),
        other => Err(format!("unknown action '{other}'")),
    }
}

/// Which file the editor reads or writes: the master when `slug` is empty, else one application.
#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct CvFile {
    pub(crate) slug: String,
    pub(crate) content: String,
}

fn edit_args(verb: &str, slug: &str) -> Result<Vec<String>, String> {
    let mut args = vec!["edit".to_string(), verb.to_string()];
    if !slug.is_empty() {
        plain_name(slug, "slug")?;
        args.push(slug.to_string());
    }
    Ok(args)
}

/// `GET /api/sjel-status/cv/file?slug=` — the raw YAML of the master or one application.
pub(crate) async fn cv_file_handler(
    Query(query): Query<CvFile>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let args = edit_args("get", &query.slug).map_err(bad_request)?;
    let (ok, stdout, stderr) = run_cv(&args).await?;
    if !ok {
        return Err(bad_request(stderr));
    }
    Ok(Json(json!({ "slug": query.slug, "content": stdout })))
}

/// `PUT /api/sjel-status/cv/file` — replace the file. `cv edit put` parses and renders it first
/// and refuses with its own words; an accepted write is committed in the overlay's git.
pub(crate) async fn cv_file_write_handler(
    Json(file): Json<CvFile>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let args = edit_args("put", &file.slug).map_err(bad_request)?;
    let (ok, stdout, stderr) = run_cv_with_input(&args, Some(&file.content)).await?;
    if !ok {
        return Err(bad_request(stderr));
    }
    Ok(Json(json!({ "slug": file.slug, "detail": stdout })))
}

#[derive(Deserialize)]
pub(crate) struct PdfQuery {
    name: String,
}

/// `GET /api/sjel-status/cv/pdf?name=<name from GET /cv>` — the PDF bytes, for inline preview.
/// Only a name the launcher itself listed is served, so no request names an arbitrary path.
pub(crate) async fn cv_pdf_handler(Query(query): Query<PdfQuery>) -> Response {
    let status = match cv_status().await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let path = status["pdfs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"] == query.name.as_str())
        .and_then(|p| p["path"].as_str().map(PathBuf::from));
    let Some(path) = path else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "no such PDF" }))).into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/pdf"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => bad_gateway(format!("could not read {}: {e}", path.display())).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(action: &str) -> CvRequest {
        CvRequest { action: action.into(), ..Default::default() }
    }

    #[test]
    fn build_needs_a_plain_profile() {
        let mut r = req("build");
        r.profile = "swe".into();
        assert_eq!(plan_cv(&r).unwrap(), ["build", "--profile", "swe", "--lang", "en"]);
        r.profile = "--all".into();
        assert!(plan_cv(&r).is_err());
    }

    #[test]
    fn lang_is_en_or_de() {
        let mut r = req("linkedin-sheet");
        r.lang = "fr".into();
        assert!(plan_cv(&r).is_err());
    }

    #[test]
    fn app_status_passes_the_note_as_one_argument() {
        let mut r = req("app-status");
        r.slug = "acme".into();
        r.status = "applied".into();
        r.note = "--profile x; rm -rf /".into();
        assert_eq!(plan_cv(&r).unwrap(), ["app", "status", "acme", "applied", "--profile x; rm -rf /"]);
    }

    #[test]
    fn slug_cannot_leave_the_directory() {
        let mut r = req("app-build");
        r.slug = "../x".into();
        assert!(plan_cv(&r).is_err());
    }

    #[test]
    fn edit_targets_the_master_or_a_plain_slug() {
        assert_eq!(edit_args("get", "").unwrap(), ["edit", "get"]);
        assert_eq!(edit_args("put", "acme").unwrap(), ["edit", "put", "acme"]);
        assert!(edit_args("put", "../master").is_err());
    }

    #[test]
    fn unknown_action_is_refused() {
        assert!(plan_cv(&req("shell")).is_err());
    }
}
