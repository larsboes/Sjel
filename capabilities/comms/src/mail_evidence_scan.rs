//! Explicit, bounded mail evidence handoff. No scan state is persisted here.

use std::{
    collections::{BTreeMap, HashSet},
    io::Read,
    time::Duration,
};

use serde::Deserialize;

use crate::{
    config::Config,
    google, intake,
    mail_evidence::{self, Proposal, Stage},
    store::Store,
    summarize::{self},
};

pub const MAX_THREADS: usize = 50;
const REPLY_TOKENS: u32 = 1200;
const MAX_PROJECT_RESPONSE: u64 = 128 * 1024;

#[derive(Default, serde::Serialize)]
pub struct ScanReport {
    pub considered: usize,
    pub completed: usize,
    pub refused: usize,
    pub failed: usize,
    pub inventory_staged: usize,
    pub projects_staged: usize,
    pub already_staged: usize,
    pub failure_stages: BTreeMap<&'static str, usize>,
    pub failed_thread_ids: Vec<String>,
    pub refused_thread_ids: Vec<String>,
}

impl ScanReport {
    fn fail(&mut self, id: &str, stage: &'static str) {
        self.failed += 1;
        *self.failure_stages.entry(stage).or_default() += 1;
        self.failed_thread_ids.push(id.to_owned());
    }
}

// Only literal loopback addresses are accepted; no DNS, credentials, query or path.
fn local_base(base: &str) -> Option<String> {
    let url = reqwest::Url::parse(base).ok()?;
    let host = url.host_str()?;
    let ip: std::net::IpAddr = host.trim_matches(['[', ']']).parse().ok()?;
    if !ip.is_loopback()
        || url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(url.as_str().trim_end_matches('/').to_owned())
}

fn client(timeout_ms: u64) -> Result<reqwest::blocking::Client, ()> {
    sjel_http::builder(
        sjel_http::Purpose::new("comms-mail-evidence"),
        Duration::from_millis(timeout_ms.clamp(1000, 30000)),
    )
    .redirect(reqwest::redirect::Policy::none())
    .no_proxy()
    .build()
    .map_err(|_| ())
}

#[derive(Deserialize)]
struct ProjectList {
    projects: Vec<ProjectName>,
}
#[derive(Deserialize)]
struct ProjectName {
    name: String,
    archived: bool,
}

fn project_names(client: &reqwest::blocking::Client, base: &str) -> Result<Vec<String>, ()> {
    let url = format!("{base}/api/projects");
    let response = sjel_server::InboundAuth::with_loopback_auth(client.get(&url), &url)
        .send()
        .map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_PROJECT_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > MAX_PROJECT_RESPONSE {
        return Err(());
    }
    let list: ProjectList = serde_json::from_slice(&bytes).map_err(|_| ())?;
    let names: Vec<String> = list
        .projects
        .into_iter()
        .filter(|p| !p.archived)
        .map(|p| p.name)
        .collect();
    if names.len() > mail_evidence::MAX_PROJECTS
        || names
            .iter()
            .any(|p| p.trim().is_empty() || p.len() > mail_evidence::MAX_FIELD_BYTES)
        || names.iter().collect::<HashSet<_>>().len() != names.len()
    {
        return Err(());
    }
    Ok(names)
}

fn rank(class: &str) -> Option<u8> {
    match class {
        "c0" => Some(0),
        "c1" => Some(1),
        "c2" => Some(2),
        "c3" => Some(3),
        _ => None,
    }
}

fn message_class(
    thread_class: &str,
    stream: &str,
    message: &google::ThreadMessageBody,
) -> Option<&'static str> {
    let from = message
        .from_addr
        .as_deref()
        .filter(|s| !s.trim().is_empty())?;
    let subject = message
        .subject
        .as_deref()
        .filter(|s| !s.trim().is_empty())?;
    let snippet = message
        .snippet
        .as_deref()
        .filter(|s| !s.trim().is_empty())?;
    // Body text can name a person or carry a credential the metadata does not show.
    // The whole body is classified: a truncated prefix would hide a credential further down.
    let body_classification = intake::classify_mail(stream, from, &message.text, "");
    let classification = intake::classify_mail(stream, from, subject, snippet);
    let rank = rank(thread_class)?
        .max(rank(&classification.value)?)
        .max(rank(&body_classification.value)?);

    match rank {
        0 => Some("c0"),
        1 => Some("c1"),
        2 => Some("c2"),
        3 => Some("c3"),
        _ => None,
    }
}

pub fn valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 120
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub fn valid_batch(ids: &[String]) -> bool {
    !ids.is_empty()
        && ids.len() <= MAX_THREADS
        && ids.iter().all(|id| valid_provider_id(id))
        && ids.iter().collect::<HashSet<_>>().len() == ids.len()
}

fn valid_message_ids(messages: &[google::ThreadMessageBody]) -> bool {
    messages.iter().all(|m| valid_provider_id(&m.id))
        && messages.iter().map(|m| &m.id).collect::<HashSet<_>>().len() == messages.len()
}

fn source_id(thread: &str, message: &str) -> Option<String> {
    if !valid_provider_id(thread) || !valid_provider_id(message) {
        return None;
    }
    Some(format!("mail:v1:{thread}:{message}"))
}

struct Extraction<'a> {
    stage: Stage,
    class: &'a str,
    text: &'a str,
    source: &'a str,
}

fn model(
    runtime: &sjel_runtime::Status,
    cfg: &Config,
    role: &sjel_inference::ResolvedRole,
    extraction: &Extraction<'_>,
    projects: &[String],
) -> Result<Vec<Proposal>, &'static str> {
    let Extraction {
        stage,
        class,
        text,
        source,
    } = *extraction;
    role.runtime_admission_with(runtime)
        .map_err(|_| "runtime_busy")?;
    let request = mail_evidence::prompt(stage, class, text, projects).map_err(|_| "prompt")?;
    crate::quiet::runtime_admission(runtime, role, request.chars().count(), REPLY_TOKENS)
        .map_err(|_| "runtime_busy")?;
    if !summarize::fits_window(
        request.chars().count(),
        REPLY_TOKENS,
        role.max_input_tokens.unwrap_or_default(),
    ) {
        return Err("prompt_too_large");
    }
    let target = crate::digest::to_target(cfg, role, runtime);
    let endpoint = reqwest::Url::parse(&target.endpoint).map_err(|_| "local_model_config")?;
    let host = endpoint.host_str().ok_or("local_model_config")?;
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !loopback
        || !matches!(endpoint.scheme(), "http" | "https")
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err("local_model_config");
    }
    let _admission = match &target.gate {
        Some(gate) => Some(gate.acquire().map_err(|_| "runtime_busy")?),
        None => None,
    };
    let http = sjel_http::builder(
        sjel_http::Purpose::new("comms-mail-evidence-model"),
        Duration::from_secs(120),
    )
    .redirect(reqwest::redirect::Policy::none())
    .no_proxy()
    .build()
    .map_err(|_| "local_model")?;
    let mut call = http.post(endpoint).json(&serde_json::json!({"model": target.model, "messages": [{"role": "user", "content": request}], "max_tokens": REPLY_TOKENS, "stream": false}));
    if let Some(key) = &target.api_key {
        call = call.bearer_auth(key);
    }
    let response = call.send().map_err(|_| "local_model")?;
    if !response.status().is_success() {
        return Err("local_model");
    }
    let mut bytes = Vec::new();
    response
        .take((mail_evidence::MAX_RESPONSE_BYTES * 4 + 4096) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "local_model")?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "local_model")?;
    if summarize::server_error(&json).is_some() {
        return Err("local_model");
    }
    let answer = json
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or("local_model")?;
    mail_evidence::parse(stage, source, class, text, projects, answer.trim())
        .map_err(|_| "extraction")
}

/// A conflicting identity means the owner already holds a different extraction for the same
/// source line. That proposal stays reviewable, so the handoff is settled rather than retried.
fn post(client: &reqwest::blocking::Client, base: &str, proposal: &Proposal) -> Result<bool, ()> {
    let url = format!("{base}/api/mail-proposals");
    let request = client.post(&url).json(proposal);
    let response = sjel_server::InboundAuth::with_loopback_auth(request, &url)
        .send()
        .map_err(|_| ())?;
    match response.status().as_u16() {
        409 => Ok(false),
        status if (200..300).contains(&status) => Ok(true),
        _ => Err(()),
    }
}

/// Explicitly analyze indexed thread IDs, without recording progress. The inventory URL
/// must be a literal HTTP loopback origin. Retry failed IDs; owner writes are idempotent.
/// Runs blocking I/O and must not be called on an async executor thread.
pub fn scan(
    runtime: &sjel_runtime::Status,
    cfg: &Config,
    inventory_base_url: &str,
    ids: &[String],
) -> ScanReport {
    let mut report = ScanReport::default();
    if ids.is_empty() {
        return report;
    }
    // IDs are bounded to the provider's opaque identifier alphabet before use in URLs or reports.
    if !valid_batch(ids) {
        report.failed = ids.len();
        *report.failure_stages.entry("invalid_batch").or_default() = ids.len();
        return report;
    }
    let setup = || -> Result<_, &'static str> {
        let inventory = local_base(inventory_base_url).ok_or("owner_config")?;
        let projects_base = local_base(&cfg.projects_context.base_url).ok_or("owner_config")?;
        let inventory_client =
            client(cfg.inventory_context.timeout_ms).map_err(|_| "owner_config")?;
        let client = client(cfg.projects_context.timeout_ms).map_err(|_| "owner_config")?;
        let role = cfg
            .light_summarization_role()
            .filter(|r| r.is_loopback())
            .ok_or("local_model_config")?;
        let store = Store::open(&cfg.database_path).map_err(|_| "store")?;
        Ok((
            inventory,
            projects_base,
            client,
            inventory_client,
            role,
            store,
        ))
    };
    let (inventory, projects_base, client, inventory_client, role, store) = match setup() {
        Ok(v) => v,
        Err(stage) => {
            for id in ids {
                report.fail(id, stage);
            }
            return report;
        }
    };
    let mut token = None;
    let mut names = None;
    for id in ids {
        report.considered += 1;
        let item = match store.get_triage(id) {
            Ok(Some(item)) => item,
            _ => {
                report.fail(id, "store");
                continue;
            }
        };
        match rank(&item.data_class) {
            Some(3) => {
                report.refused += 1;
                report.refused_thread_ids.push(id.clone());
                continue;
            }
            Some(_) => {}
            None => {
                report.fail(id, "classification");
                continue;
            }
        }
        if token.is_none() {
            token = google::access_token(&cfg.google_env_path).ok();
        }
        let Some(token) = token.as_deref() else {
            report.fail(id, "mail_auth");
            continue;
        };
        let messages = match google::thread_message_bodies(token, id) {
            Ok(Some(messages)) if !messages.is_empty() => messages,
            _ => {
                report.fail(id, "mail_body");
                continue;
            }
        };
        if !valid_message_ids(&messages) {
            report.fail(id, "message_id");
            continue;
        }
        let classes: Option<Vec<_>> = messages
            .iter()
            .map(|m| message_class(&item.data_class, &item.stream, m))
            .collect();
        let Some(classes) = classes else {
            report.fail(id, "classification");
            continue;
        };
        if classes.contains(&"c3") {
            report.refused += 1;
            report.refused_thread_ids.push(id.clone());
            continue;
        }
        if names.is_none() {
            names = project_names(&client, &projects_base).ok();
        }
        let Some(names) = names.as_ref() else {
            report.fail(id, "projects_context");
            continue;
        };
        let mut pending = Vec::new();
        let mut failure = None;
        for (message, class) in messages.iter().zip(classes) {
            let Some(source) = source_id(id, &message.id) else {
                failure = Some("message_id");
                break;
            };
            for stage in [Stage::Inventory, Stage::Projects] {
                let extraction = Extraction {
                    stage,
                    class,
                    text: &message.text,
                    source: &source,
                };
                match model(runtime, cfg, &role, &extraction, names) {
                    Ok(proposals) => pending.extend(proposals.into_iter().map(|p| (stage, p))),
                    Err(code) => {
                        failure = Some(code);
                        break;
                    }
                }
            }
            if failure.is_some() {
                break;
            }
        }
        if let Some(code) = failure {
            report.fail(id, code);
            continue;
        }
        // The indexed class can change while this thread is being analyzed. A stricter class
        // must not hand off evidence extracted under the earlier, weaker one.
        match store.get_triage(id) {
            Ok(Some(current)) if current.data_class == item.data_class => {}
            Ok(Some(_)) => {
                report.fail(id, "stale_class");
                continue;
            }
            _ => {
                report.fail(id, "store");
                continue;
            }
        }
        for (stage, proposal) in pending {
            let base = match stage {
                Stage::Inventory => &inventory,
                Stage::Projects => &projects_base,
            };
            let owner_client = match stage {
                Stage::Inventory => &inventory_client,
                Stage::Projects => &client,
            };
            match post(owner_client, base, &proposal) {
                Ok(true) => match stage {
                    Stage::Inventory => report.inventory_staged += 1,
                    Stage::Projects => report.projects_staged += 1,
                },
                Ok(false) => report.already_staged += 1,
                Err(()) => {
                    failure = Some("handoff");
                    break;
                }
            }
        }
        if let Some(code) = failure {
            report.fail(id, code);
        } else {
            report.completed += 1;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_origins_only() {
        assert!(local_base("http://127.0.0.1:8101").is_some());
        for url in [
            "http://localhost:8101",
            "http://192.0.2.1",
            "http://127.0.0.1/path",
            "http://user@127.0.0.1",
            "https://127.0.0.1",
        ] {
            assert!(local_base(url).is_none());
        }
    }
    #[test]
    fn message_metadata_is_required_and_secret_escalates() {
        let mut msg = google::ThreadMessageBody {
            id: "m".into(),
            from_addr: Some("sender@example.test".into()),
            subject: Some("Your verification code".into()),
            snippet: Some("Use the code to sign in".into()),
            text: "body".into(),
        };
        assert_eq!(message_class("c1", "aktiv", &msg), Some("c3"));
        msg.snippet = None;
        assert_eq!(message_class("c1", "aktiv", &msg), None);
        assert_eq!(message_class("unknown", "aktiv", &msg), None);
    }
    #[test]
    fn duplicate_and_invalid_message_ids_fail_closed() {
        let make = |id: &str| google::ThreadMessageBody {
            id: id.into(),
            from_addr: None,
            subject: None,
            snippet: None,
            text: String::new(),
        };
        assert!(valid_message_ids(&[make("one"), make("two")]));
        assert!(!valid_message_ids(&[make("one"), make("one")]));
        assert!(!valid_message_ids(&[make("one:two")]));
    }
    #[test]
    fn owner_payloads_do_not_include_foreign_fields() {
        let inventory = mail_evidence::parse(
            Stage::Inventory,
            "source",
            "c1",
            "Order: mug",
            &[],
            r#"[{"event":"order","title":"mug","evidence":"Order: mug"}]"#,
        )
        .unwrap();
        let project = mail_evidence::parse(Stage::Projects, "source", "c1", "Order: mug", &["Project".into()], r#"[{"event":"project_evidence","project":"Project","title":"mug","evidence":"Order: mug"}]"#).unwrap();
        let a = serde_json::to_value(&inventory[0]).unwrap();
        let b = serde_json::to_value(&project[0]).unwrap();
        assert!(a.get("project").is_none() && a.get("task_suggestion").is_none());
        assert!(b.get("order_ref").is_none());
    }

    #[test]
    fn owner_redirects_are_not_followed() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let _ = stream.read(&mut buffer);
            stream.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://192.0.2.1/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let proposal = mail_evidence::parse(
            Stage::Inventory,
            "source",
            "c1",
            "Order: mug",
            &[],
            r#"[{"event":"order","title":"mug","evidence":"Order: mug"}]"#,
        )
        .unwrap();
        assert!(post(&client(1000).unwrap(), &base, &proposal[0]).is_err());
        worker.join().unwrap();
    }

    #[test]
    fn source_identity_is_scoped_and_opaque() {
        assert_eq!(
            source_id("thread", "message"),
            source_id("thread", "message")
        );
        assert_ne!(source_id("thread", "message"), source_id("thread", "other"));
        assert_ne!(
            source_id("thread", "message"),
            source_id("other", "message")
        );
        assert_eq!(
            source_id("thread", "message").as_deref(),
            Some("mail:v1:thread:message")
        );
        assert!(source_id("thread:bad", "message").is_none());
        assert!(source_id("thread", &"x".repeat(121)).is_none());
        assert!(
            source_id(&"x".repeat(120), &"y".repeat(120)).unwrap().len()
                <= mail_evidence::MAX_FIELD_BYTES
        );
    }
}
