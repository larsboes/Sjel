//! The Posts page's backend. Every read and write shells `crates/posts/posts`, the same
//! launcher the CLI uses, so the page cannot disagree with it.

use super::*;
use axum::extract::Query;

const POSTS: &str = "crates/posts/posts";

async fn run_posts(
    args: &[String],
    input: Option<&str>,
) -> Result<(bool, String, String), (StatusCode, Json<Value>)> {
    use tokio::io::AsyncWriteExt;
    let root = sjel_root().map_err(bad_gateway)?;
    let mut child = tokio::process::Command::new(root.join(POSTS))
        .args(args)
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| bad_gateway(format!("could not run {POSTS}: {e}")))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    if let Some(text) = input {
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|e| bad_gateway(format!("could not write to {POSTS}: {e}")))?;
    }
    drop(stdin);
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| bad_gateway(format!("could not run {POSTS}: {e}")))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    ))
}

async fn posts_json(args: &[String]) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (ok, stdout, stderr) = run_posts(args, None).await?;
    if !ok {
        return Err(bad_request(stderr));
    }
    serde_json::from_str(&stdout)
        .map(Json)
        .map_err(|e| bad_gateway(format!("{POSTS} did not emit JSON: {e}")))
}

/// `GET /api/sjel-status/posts` — every post with its state.
pub(crate) async fn posts_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    posts_json(&["status".into(), "--json".into()]).await
}

#[derive(Deserialize, Default, schemars::JsonSchema)]
#[serde(default)]
pub(crate) struct PostQuery {
    pub(crate) slug: String,
}

/// `GET /api/sjel-status/posts/detail?slug=` — text, ordered media and export files.
pub(crate) async fn post_detail_handler(
    Query(query): Query<PostQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    plain_name(&query.slug, "slug").map_err(bad_request)?;
    posts_json(&["get".into(), query.slug, "--json".into()]).await
}

/// Body of `POST /api/sjel-status/posts`. Fields not used by an action are ignored.
/// `index` counts slots from 0.
#[derive(Deserialize, Default, schemars::JsonSchema)]
#[serde(default)]
pub(crate) struct PostsRequest {
    pub(crate) action: String,
    pub(crate) slug: String,
    pub(crate) title: String,
    pub(crate) event: String,
    pub(crate) state: String,
    pub(crate) url: String,
    pub(crate) text: String,
    pub(crate) index: usize,
    pub(crate) direction: String,
    pub(crate) layout: String,
    pub(crate) accent: String,
}

/// Run one `posts` verb. A refusal from the launcher is returned as a 400 with its own words.
pub(crate) async fn posts_write_handler(
    Json(request): Json<PostsRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let args = plan_posts(&request).map_err(bad_request)?;
    let input = (request.action == "text").then_some(request.text.as_str());
    let (ok, stdout, stderr) = run_posts(&args, input).await?;
    if !ok {
        return Err(bad_request(if stderr.is_empty() { stdout } else { stderr }));
    }
    Ok(Json(
        json!({ "action": request.action, "detail": stdout, "warnings": stderr }),
    ))
}

/// The launcher argv for a request. Separate from the handler so the mapping is tested
/// without running Photos or ImageMagick.
fn plan_posts(r: &PostsRequest) -> Result<Vec<String>, String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    plain_name(&r.slug, "slug")?;
    let slot = (r.index + 1).to_string();
    match r.action.as_str() {
        "new" => {
            if r.title.trim().is_empty() {
                return Err("a new post needs a title".into());
            }
            let mut args = s(&["new", &r.slug, "--title", &r.title]);
            if !r.event.is_empty() {
                args.extend(s(&["--event", &r.event]));
            }
            Ok(args)
        }
        "text" => Ok(s(&["text", "put", &r.slug])),
        "state" => {
            plain_name(&r.state, "state")?;
            let mut args = s(&["state", &r.slug, &r.state]);
            if !r.url.is_empty() {
                args.extend(s(&["--url", &r.url]));
            }
            Ok(args)
        }
        "add-selected" => Ok(s(&["media", "add-selected", &r.slug])),
        "move" => {
            plain_name(&r.direction, "direction")?;
            Ok(s(&["media", "move", &r.slug, &slot, &r.direction]))
        }
        "remove" => Ok(s(&["media", "remove", &r.slug, &slot])),
        "pair" => {
            let layout = if r.layout.is_empty() {
                "pair"
            } else {
                r.layout.as_str()
            };
            let accent = if r.accent.is_empty() {
                "none"
            } else {
                r.accent.as_str()
            };
            plain_name(layout, "layout")?;
            plain_name(accent, "accent")?;
            Ok(s(&[
                "media", "pair", &r.slug, &slot, "--layout", layout, "--accent", accent,
            ]))
        }
        "split" => Ok(s(&["media", "split", &r.slug, &slot])),
        "accent" => {
            plain_name(&r.accent, "accent")?;
            Ok(s(&["media", "accent", &r.slug, &slot, &r.accent]))
        }
        "export" => Ok(s(&["export", &r.slug])),
        other => Err(format!("unknown action '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(action: &str) -> PostsRequest {
        PostsRequest {
            action: action.into(),
            slug: "recap".into(),
            ..Default::default()
        }
    }

    #[test]
    fn slots_count_from_one_on_the_command_line() {
        let r = PostsRequest {
            index: 2,
            direction: "up".into(),
            ..req("move")
        };
        assert_eq!(
            plan_posts(&r).unwrap(),
            ["media", "move", "recap", "3", "up"]
        );
    }

    #[test]
    fn pair_defaults_and_refusals() {
        assert_eq!(
            plan_posts(&req("pair")).unwrap(),
            ["media", "pair", "recap", "1", "--layout", "pair", "--accent", "none"]
        );
        assert!(plan_posts(&PostsRequest {
            slug: "../x".into(),
            ..req("export")
        })
        .is_err());
        assert!(plan_posts(&req("new")).is_err());
        assert!(plan_posts(&req("publish")).is_err());
    }
}
