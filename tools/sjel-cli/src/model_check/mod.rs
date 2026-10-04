//! `tools/model-check` — is every model this machine names still real, still served, and still the
//! newest of its line?
//!
//! Ported from `tools/model-check.ts` on 2026-10-04, so `tools/doctor` stops starting an
//! interpreter: it ran `bun tools/model-check.ts --local --json` unconditionally and that leg
//! measured 6.9 s of the doctor's 20.6 s. The doctor now calls [`report`] in-process and reads the
//! same payload it read before.
//!
//! ## Why the tool exists
//!
//! Dependabot watches package registries; a model id is not a package, so nothing watched these.
//! The cost of that was measured on 2026-08-30: `summarization_light` named `apple-on-device`
//! months after apfel replaced the server that answered to it, and every unattended request 404'd
//! with `model_not_found` — at request time, so it read as "the light rung never summarizes
//! anything" rather than as a typo.
//!
//! Three questions, deliberately separate, because each has a different answer and a different
//! cost:
//!
//!   1. Is the declared model in the provider's catalogue?  Absent = broken now.
//!   2. Does it actually answer?  (`--probe`)  Listed is not served: `gemini-3.7-flash` is in the
//!      catalogue and returns 503 "high demand" on the free tier, three attempts running.
//!   3. Is something newer available in the same family?  Advisory, never a failure — newer is a
//!      decision about quality, cost and capacity, and question 2 is why it cannot be automatic.
//!
//! The full sweep is not wired into `tools/doctor`: it dials third-party APIs and spends quota,
//! which no check that runs on every invocation should do. `--local` is, because it restricts the
//! sweep to loopback backends and so spends nothing and leaves the machine for nothing — and a
//! stopped local server is precisely the condition nothing else reported.
//!
//! ## Exit codes
//!
//! 0 = checked, every model listed (and probed, if asked) · 1 = at least one role names a model its
//! backend does not list, or declares an incomplete role · 2 = no overlay, or a config that is not
//! readable as JSON.
//!
//! ## Differences from the TypeScript, all deliberate
//!
//! The two rules it shares with `libs/inference` come from there now rather than from a second
//! copy: [`sjel_inference::is_loopback_url`] decides whether a backend is in scope for `--local`
//! (broader than the TypeScript's three-name list — it also reads `127.*` and `0.0.0.0`, and it
//! strips userinfo, so `http://127.0.0.1:1@evil.example` is not loopback), and
//! [`sjel_inference::resolve_key_file`] plus [`sjel_inference::api_key_from_file`] read the
//! credential. That second pair is a real behaviour change for a backend whose `api_key_file`
//! names a `~/` path or a JSON settings file: the TypeScript joined `~/.omlx/settings.json` onto
//! the config directory, found nothing, and reported the role as `credential unavailable` without
//! ever dialling it. No loopback role on this machine declares one today, so nothing it reports
//! changes — but a declared provider would have been reported as unprovisioned while its
//! credential sat exactly where the file said.
//!
//! A role's declaration is read leniently, as the TypeScript read it, rather than through
//! `InferenceConfig`: that struct requires `backend` and `model` on every role and silently
//! degrades the WHOLE config to empty when one is missing, which would turn a broken declaration
//! into doctor's `ok`. The one thing that must never happen here is a config fault reading as
//! health.
//!
//! Roles are checked in the order the file declares them, which is the order the TypeScript
//! emitted — local rungs first, then cloud rungs by failover priority. `serde_json`'s own `Map`
//! cannot give that (it is a `BTreeMap` here, and this crate deliberately does not enable
//! `preserve_order`), so the config is read through `pure::OrderedMap`.

mod pure;

use crate::paths::Paths;
use pure::{
    answered, asks_for_a_vector, catalogue_ids, declared_as, family, js_number, newer,
    refusal_detail, status_for, OrderedMap,
};
use serde_json::{json, Value};
use sjel_http::Purpose;
use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

const HELP: &str = "\
tools/model-check — is every model this machine names real, served, and the newest of its line?

  tools/model-check            check the catalogue each role's model is declared in
  tools/model-check --probe    also ask every model to answer (spends provider quota)
  tools/model-check --local    only loopback backends, each probed (free; what doctor runs)
  tools/model-check --json     machine-readable report
  tools/model-check -h         this help

Config: <overlay>/config/inference.json
";

/// The catalogue read is a list; the probe is a completion. Two budgets, the two the TypeScript's
/// `AbortSignal` calls used.
const CATALOGUE_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub probe: bool,
    pub local_only: bool,
    pub json: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Entry {
    pub role: String,
    pub backend: String,
    pub model: String,
    pub status: &'static str,
    pub detail: String,
}

/// Everything one pass found, in the shape the `--json` payload and the doctor both read.
pub struct Outcome {
    pub scope: &'static str,
    pub entries: Vec<Entry>,
    pub failures: usize,
    pub stated_quota_breaches: usize,
    pub role_count: usize,
}

impl Outcome {
    /// The `--json` payload. Key order is the object literal's from the TypeScript.
    pub fn payload(&self) -> Value {
        let count = |status: &str| self.entries.iter().filter(|e| e.status == status).count();
        json!({
            "scope": self.scope,
            "entries": self.entries,
            "stated_quota_breaches": self.stated_quota_breaches,
            "totals": {
                "count": self.entries.len(),
                "ok": count("ok"),
                "missing": count("missing"),
                "unreachable": count("unreachable"),
                "incomplete": count("incomplete"),
            },
        })
    }
}

fn fail(message: &str, code: u8) -> ExitCode {
    eprintln!("model-check: {message}");
    ExitCode::from(code)
}

pub fn run(argv: &[String]) -> ExitCode {
    if argv.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let opts = Options {
        probe: argv.iter().any(|a| a == "--probe"),
        local_only: argv.iter().any(|a| a == "--local"),
        json: argv.iter().any(|a| a == "--json"),
    };
    let paths = match Paths::from_env() {
        Ok(p) => p,
        Err(e) => return fail(&e, 2),
    };
    let Some(overlay) = paths.overlay_root.as_deref() else {
        return fail("no overlay — run tools/install.sh", 2);
    };
    match report(&opts, overlay) {
        Ok(outcome) => {
            if opts.json {
                println!(
                    "{}",
                    serde_json::to_string(&outcome.payload()).unwrap_or_else(|_| "{}".to_owned())
                );
            } else {
                println!("{}", footer(&outcome, &opts));
            }
            if outcome.failures > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => fail(&e, 2),
    }
}

/// The closing line, which the TypeScript printed as one string with a leading blank line.
fn footer(outcome: &Outcome, opts: &Options) -> String {
    let probed = opts.probe || opts.local_only;
    format!(
        "\n{} role(s) checked{}{}, {} naming a model its backend does not list{}",
        outcome.role_count,
        if opts.local_only { " on loopback" } else { "" },
        if probed {
            ", each probed".to_owned()
        } else {
            " (pass --probe to ask each one to answer)".to_owned()
        },
        outcome.failures,
        if outcome.stated_quota_breaches > 0 {
            format!(
                ", {} declaring a daily ceiling above the provider's own",
                outcome.stated_quota_breaches
            )
        } else {
            String::new()
        }
    )
}

#[derive(Debug, Clone, Default)]
struct BackendDecl {
    is_ollama: bool,
    base_url: Option<String>,
    api_key_file: Option<String>,
}

#[derive(Debug, Clone)]
struct RoleDecl {
    name: String,
    backend: String,
    model: String,
    max_requests_per_day: Option<f64>,
}

/// `inference.json`, read leniently: a role missing its backend or model is data, not a parse
/// error, because that is a thing the tool has to be able to say out loud. Order is kept — see
/// [`pure::OrderedMap`].
#[derive(Debug, Default, serde::Deserialize)]
struct Doc {
    #[serde(default)]
    backends: OrderedMap,
    #[serde(default)]
    roles: OrderedMap,
}

fn read_config(overlay: &Path) -> Result<(HashMap<String, BackendDecl>, Vec<RoleDecl>), String> {
    let config_path = overlay.join("config").join("inference.json");
    let text = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("cannot read {}: {e}", config_path.display()))?;
    let doc: Doc = serde_json::from_str(&text)
        .map_err(|e| format!("cannot read {}: {e}", config_path.display()))?;

    let mut backends = HashMap::new();
    for (name, b) in &doc.backends.0 {
        let string = |key: &str| {
            b.get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|s| !s.is_empty())
        };
        backends.insert(
            name.clone(),
            BackendDecl {
                is_ollama: b.get("api").and_then(Value::as_str) == Some("ollama"),
                base_url: string("base_url"),
                api_key_file: string("api_key_file"),
            },
        );
    }

    // A `Vec`, not a map: the file's order is the report's order.
    let roles = doc
        .roles
        .0
        .iter()
        .map(|(name, r)| RoleDecl {
            name: name.clone(),
            backend: r
                .get("backend")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            model: r.get("model").and_then(Value::as_str).unwrap_or("").to_owned(),
            max_requests_per_day: r
                .get("max_requests_per_day")
                .map(js_number)
                .filter(|n| n.is_finite()),
        })
        .collect();
    Ok((backends, roles))
}

pub fn report(opts: &Options, overlay: &Path) -> Result<Outcome, String> {
    let (backends, roles) = read_config(overlay)?;

    let roles: Vec<&RoleDecl> = roles
        .iter()
        .filter(|role| {
            if !opts.local_only {
                return true;
            }
            backends
                .get(&role.backend)
                .is_some_and(|b| sjel_inference::is_loopback_url(b.base_url.as_deref().unwrap_or("")))
        })
        .collect();

    let catalogue_client =
        sjel_http::client(Purpose::new("model-check"), CATALOGUE_TIMEOUT).map_err(|e| e.to_string())?;
    let probe_client =
        sjel_http::client(Purpose::new("model-check"), PROBE_TIMEOUT).map_err(|e| e.to_string())?;

    let mut checker = Checker {
        catalogue_client: &catalogue_client,
        probe_client: &probe_client,
        opts,
        breaches: 0,
    };
    let mut entries = Vec::new();
    let mut failures = 0usize;
    // One catalogue per backend, read on the first role that reaches it.
    let mut cache: HashMap<String, Result<Vec<String>, String>> = HashMap::new();

    for role in &roles {
        let backend_name = role.backend.as_str();
        let model = role.model.as_str();
        let Some(backend) = backends.get(backend_name) else {
            checker.say(&format!(
                "{}: incomplete declaration (backend='{backend_name}' model='{model}')",
                role.name
            ));
            entries.push(Entry {
                role: role.name.clone(),
                backend: backend_name.to_owned(),
                model: model.to_owned(),
                status: "incomplete",
                detail: "no backend or no model declared".to_owned(),
            });
            failures += 1;
            continue;
        };
        if model.is_empty() {
            checker.say(&format!(
                "{}: incomplete declaration (backend='{backend_name}' model='{model}')",
                role.name
            ));
            entries.push(Entry {
                role: role.name.clone(),
                backend: backend_name.to_owned(),
                model: model.to_owned(),
                status: "incomplete",
                detail: "no backend or no model declared".to_owned(),
            });
            failures += 1;
            continue;
        }

        let key = backend_key(overlay, backend);
        // A backend that names a key file it cannot read is not broken, it is not provisioned —
        // the ordinary state of a provider that has been declared and whose Vaultwarden item has
        // not been materialized yet. Dialling it anyway spends a round trip to be told 401, and
        // reports a pending setup step as a fault. `libs/inference::credential_ready` refuses the
        // same way before any request, which is why comms never sends one either.
        if backend.api_key_file.is_some() && key.is_none() {
            checker.say(&format!(
                "{}: {} on {} — credential unavailable, not probed (materialize its key to enable this role)",
                role.name, model, backend_name
            ));
            entries.push(Entry {
                role: role.name.clone(),
                backend: backend_name.to_owned(),
                model: model.to_owned(),
                status: "unreachable",
                detail: "credential unavailable — key file is absent or empty".to_owned(),
            });
            continue;
        }

        let listed = cache
            .entry(backend_name.to_owned())
            .or_insert_with(|| checker.catalogue(backend, key.as_deref()))
            .clone();

        match listed {
            Err(note) => {
                // No catalogue is never a failure by itself: a local backend that is not running
                // and a provider that publishes no model list are both ordinary.
                //
                // But it used to skip the PROBE too, and that is the wrong half to drop. Question
                // 1 asks whether the model is listed; question 2 asks whether it answers, and
                // question 2 is still answerable when the list is missing. Cloudflare's catalogue
                // 405s here, so the provider that wrote 160 of this machine's 298 digests was the
                // one role the tool said nothing about at all.
                let reply = if opts.probe || opts.local_only {
                    checker.ask(role, backend, model, key.as_deref())
                } else {
                    String::new()
                };
                let detail = if reply.is_empty() {
                    note.clone()
                } else {
                    format!("{note}, but {reply}")
                };
                checker.say(&format!("{}: {model} on {backend_name} — {detail}", role.name));
                entries.push(Entry {
                    role: role.name.clone(),
                    backend: backend_name.to_owned(),
                    model: model.to_owned(),
                    // Answering is what the role is for. A model that answers while its provider
                    // publishes no list is working, and calling that unreachable would be this
                    // tool contradicting the request that just succeeded.
                    status: status_for(answered(&reply)),
                    detail,
                });
            }
            Ok(ids) => {
                let present = ids
                    .iter()
                    .any(|id| declared_as(backend.is_ollama, id).iter().any(|d| d == model));
                let mut candidates: Vec<&String> = ids
                    .iter()
                    .filter(|id| family(id) == family(model) && newer(id, model))
                    .collect();
                candidates.sort();
                let suffix = if candidates.is_empty() {
                    String::new()
                } else {
                    format!(
                        "  newer available: {}",
                        candidates
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };

                if !present {
                    failures += 1;
                    let mut near: Vec<&String> = ids
                        .iter()
                        .filter(|id| family(id) == family(model))
                        .collect();
                    near.sort();
                    let near_note = if near.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " — same family: {}",
                            near.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                        )
                    };
                    checker.say(&format!(
                        "{}: ✗ {model} is NOT in {backend_name}'s catalogue{near_note}",
                        role.name
                    ));
                    entries.push(Entry {
                        role: role.name.clone(),
                        backend: backend_name.to_owned(),
                        model: model.to_owned(),
                        status: "missing",
                        detail: format!("not in {backend_name}'s catalogue{near_note}"),
                    });
                    continue;
                }

                let spoke = if opts.probe || opts.local_only {
                    checker.ask(role, backend, model, key.as_deref())
                } else {
                    String::new()
                };
                checker.say(&format!(
                    "{}: {model} on {backend_name} ok{}{suffix}",
                    role.name,
                    if spoke.is_empty() {
                        String::new()
                    } else {
                        format!(" — {spoke}")
                    }
                ));
                entries.push(Entry {
                    role: role.name.clone(),
                    backend: backend_name.to_owned(),
                    model: model.to_owned(),
                    // A probe that came back as an HTTP status or a timeout is not an ok.
                    status: if spoke.is_empty() {
                        "ok"
                    } else {
                        status_for(answered(&spoke))
                    },
                    detail: if spoke.is_empty() {
                        "listed".to_owned()
                    } else {
                        spoke
                    },
                });
            }
        }
    }

    Ok(Outcome {
        scope: if opts.local_only { "loopback" } else { "all" },
        entries,
        failures,
        stated_quota_breaches: checker.breaches,
        role_count: roles.len(),
    })
}

/// The backend's bearer key: the declared file, resolved against the config directory the way
/// `libs/inference` resolves it, then read the way `libs/inference` reads it.
fn backend_key(overlay: &Path, backend: &BackendDecl) -> Option<String> {
    let raw = backend.api_key_file.as_deref()?;
    let resolved = sjel_inference::resolve_key_file(&overlay.join("config"), raw);
    sjel_inference::api_key_from_file(Some(&resolved))
}

struct Checker<'a> {
    catalogue_client: &'a reqwest::blocking::Client,
    probe_client: &'a reqwest::blocking::Client,
    opts: &'a Options,
    breaches: usize,
}

impl Checker<'_> {
    fn say(&self, line: &str) {
        if !self.opts.json {
            println!("{line}");
        }
    }

    /// The model ids a backend lists, or the reason it lists none.
    fn catalogue(&self, backend: &BackendDecl, key: Option<&str>) -> Result<Vec<String>, String> {
        let Some(base) = backend.base_url.as_deref() else {
            return Err("no base_url declared".to_owned());
        };
        let base = base.trim_end_matches('/');
        let url = if backend.is_ollama {
            format!("{base}/api/tags")
        } else {
            format!("{base}/models")
        };
        let mut request = self.catalogue_client.get(&url);
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let Ok(response) = request.send() else {
            return Err("backend not reachable".to_owned());
        };
        if !response.status().is_success() {
            return Err(format!(
                "catalogue unavailable (HTTP {})",
                response.status().as_u16()
            ));
        }
        // A body that is not JSON lands here, as it did on the TypeScript side: the `json()` call
        // was inside the same try as the fetch.
        match response.json::<Value>() {
            Ok(body) => catalogue_ids(&body),
            Err(_) => Err("backend not reachable".to_owned()),
        }
    }

    /// One probe, with the declared-ceiling check that rides back on the refusal.
    fn ask(
        &mut self,
        role: &RoleDecl,
        backend: &BackendDecl,
        model: &str,
        key: Option<&str>,
    ) -> String {
        let (reply, stated) = if asks_for_a_vector(&role.name) {
            self.probe_embedding(backend, model, key)
        } else {
            self.probe(backend, model, key)
        };
        if let Some(stated) = stated {
            if let Some(declared) = role.max_requests_per_day {
                if declared > stated {
                    self.breaches += 1;
                    let line = format!(
                        "  ! {} declares max_requests_per_day {declared}, provider states {stated} — the local guard is above the real ceiling and stops guarding",
                        role.name
                    );
                    self.say(&line);
                }
            }
        }
        reply
    }

    fn probe(
        &self,
        backend: &BackendDecl,
        model: &str,
        key: Option<&str>,
    ) -> (String, Option<f64>) {
        let Some(base) = backend.base_url.as_deref() else {
            return ("no base_url declared".to_owned(), None);
        };
        let base = base.trim_end_matches('/');
        // Mirrors `libs/inference`'s chat endpoint: an OpenAI backend's base_url already carries
        // its version segment, an Ollama one does not. Wrong here and a served model answers 404
        // and reads as absent — the failure this tool reports, arriving from the tool itself.
        let path = if backend.is_ollama {
            "/v1/chat/completions"
        } else {
            "/chat/completions"
        };
        let mut request = self.probe_client.post(format!("{base}{path}")).json(&json!({
            "model": model,
            "messages": [{ "role": "user", "content": "Reply with one word: OK" }],
            // Generous, because a reasoning model spends its budget thinking and a stingy cap
            // comes back as `finish_reason: length` — which would make a healthy model look
            // broken here, the mirror of the bug that put 15 chains of thought in the Feed.
            "max_tokens": 2000,
        }));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = match request.send() {
            Ok(r) => r,
            Err(_) => return ("no answer within 60s".to_owned(), None),
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().unwrap_or_default();
            let (detail, stated) = refusal_detail(&text);
            let reply = format!(
                "answers HTTP {status}{}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(" — {detail}")
                }
            );
            return (reply, stated);
        }
        match response.json::<Value>() {
            Ok(body) => {
                let spoke = body
                    .get("choices")
                    .and_then(Value::as_array)
                    .and_then(|c| c.first())
                    .and_then(|c| c.get("message"));
                let reply = if spoke.is_some() {
                    "answers"
                } else {
                    "answered without a message"
                };
                (reply.to_owned(), None)
            }
            Err(_) => ("no answer within 60s".to_owned(), None),
        }
    }

    /// Ask an embedding model for a vector. Question 2 is "does it actually answer", and for a
    /// retrieval model the answer is a vector.
    fn probe_embedding(
        &self,
        backend: &BackendDecl,
        model: &str,
        key: Option<&str>,
    ) -> (String, Option<f64>) {
        let Some(base) = backend.base_url.as_deref() else {
            return ("no base_url declared".to_owned(), None);
        };
        let base = base.trim_end_matches('/');
        // Two shapes, matching `libs/inference`'s embedding endpoint: a second opinion about which
        // URL is correct is how this tool starts disagreeing with the thing it checks.
        let body = if backend.is_ollama {
            json!({ "model": model, "input": ["probe"] })
        } else {
            json!({ "model": model, "input": "probe" })
        };
        let mut request = self
            .probe_client
            .post(format!(
                "{base}{}",
                if backend.is_ollama {
                    "/api/embed"
                } else {
                    "/embeddings"
                }
            ))
            .json(&body);
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = match request.send() {
            Ok(r) => r,
            Err(_) => return ("no answer within 60s".to_owned(), None),
        };
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().unwrap_or_default();
            let (detail, stated) = refusal_detail(&text);
            let reply = format!(
                "answers HTTP {status}{}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(" — {detail}")
                }
            );
            return (reply, stated);
        }
        match response.json::<Value>() {
            Ok(body) => {
                let vector = if backend.is_ollama {
                    body.get("embeddings")
                        .and_then(Value::as_array)
                        .and_then(|e| e.first())
                } else {
                    body.get("data")
                        .and_then(Value::as_array)
                        .and_then(|d| d.first())
                        .and_then(|d| d.get("embedding"))
                };
                let reply = match vector.and_then(Value::as_array) {
                    Some(v) if !v.is_empty() => format!("answers ({}-dimensional)", v.len()),
                    _ => "answered without a vector".to_owned(),
                };
                (reply, None)
            }
            Err(_) => ("no answer within 60s".to_owned(), None),
        }
    }
}
