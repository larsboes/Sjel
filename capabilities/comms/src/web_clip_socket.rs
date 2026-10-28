//! The host-only path from the Sjel Clipper extension to the clip store.
//!
//! The browser starts `comms-clip-host` through native messaging; the host relays each
//! message over a Unix socket that only `comms-server` listens on. The socket is a file
//! with owner-only permissions in the overlay data directory, so there is no TCP route for
//! `sjel-status` to proxy and no bearer token in browser storage. Both hops use the native
//! messaging frame: a little-endian `u32` length, then that many bytes of JSON.

use std::path::Path;

use serde_json::{json, Value};

pub use sjel_clip_socket::{
    read_frame, relay, socket_path, write_frame, HOST_NAME, MAX_FRAME_BYTES,
};

use crate::config::Config;
use crate::store::{ClipError, SettingsError, Store, WebClipInput, WebClipWrite};
use crate::summarize::{self, Outcome, Reach};
use crate::{digest, quiet};

/// The Sjel Clipper's Interpreter, answered by Sjel instead of a provider the extension holds a
/// key for. Behind a trait so the socket can be tested without a model.
pub trait Interpreter {
    /// Which model would answer, for the extension and the app to show.
    fn describe(&self) -> Value;
    /// Answer one prompt, or say why not.
    fn interpret(&self, prompt: &str) -> Value;
}

/// Reply budget for one Interpreter call; upstream asks for short Markdown answers.
const INTERPRET_REPLY_TOKENS: u32 = 1500;

/// The light local role, the same rung unattended comms work uses. Clip content is `c2`, so
/// the request is `LoopbackOnly`: a role that is not on hardware the operator owns is refused,
/// never replaced by a cloud model.
pub struct LocalInterpreter(pub Config);

impl Interpreter for LocalInterpreter {
    fn describe(&self) -> Value {
        match self.0.light_summarization_role() {
            Some(role) => {
                json!({ "available": true, "model": role.model, "backend": role.backend_name })
            }
            None => json!({ "available": false }),
        }
    }

    fn interpret(&self, prompt: &str) -> Value {
        let chars = prompt.chars().count();
        let role = match quiet::rung(&self.0.inference, chars, INTERPRET_REPLY_TOKENS) {
            quiet::Rung::Light(role) => *role,
            quiet::Rung::OverWindow => return failure("too_large"),
            quiet::Rung::Unconfigured => return failure("unconfigured"),
        };
        let runtime = match sjel_runtime::current() {
            Ok(runtime) => runtime,
            Err(deferred) => {
                return json!({ "ok": false, "error": "deferred", "detail": deferred.to_string() })
            }
        };
        if let Err(deferred) =
            quiet::runtime_admission(&runtime, &role, chars, INTERPRET_REPLY_TOKENS)
        {
            return json!({ "ok": false, "error": "deferred", "detail": deferred.to_string() });
        }
        let target = digest::to_target(&self.0, &role, &runtime);
        match summarize::ask(
            Some(&target),
            prompt,
            INTERPRET_REPLY_TOKENS,
            Reach::LoopbackOnly,
        ) {
            Outcome::Ok(content) => json!({ "ok": true, "content": content, "model": role.model }),
            other => failure(other.state()),
        }
    }
}

fn settings_error(error: &SettingsError) -> Value {
    failure(match error {
        SettingsError::Invalid => "invalid",
        SettingsError::TooLarge => "too_large",
        SettingsError::Stale => "stale",
        SettingsError::AlreadySeeded => "already_seeded",
        SettingsError::Storage => "storage",
    })
}

fn failure(error: &str) -> Value {
    json!({ "ok": false, "error": error })
}

fn clip_error(error: &ClipError) -> Value {
    failure(match error {
        ClipError::Invalid => "invalid",
        ClipError::TooLarge => "too_large",
        ClipError::Conflict => "conflict",
        ClipError::Storage => "storage",
    })
}

/// Answer one request against the store at `database`.
///
/// Requests: `ping`; `save` with `clip`; `documents` with `limit`; `document` with `key`;
/// `settings_get`; `settings_seed` with `templates`; `settings_put` with `templates` and
/// `base_revision`; `interpret` with `prompt`.
pub fn handle(database: &Path, interpreter: &dyn Interpreter, request: &Value) -> Value {
    let kind = request.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "ping" => return json!({ "ok": true, "service": "comms", "version": 1 }),
        "interpret" => {
            return match request.get("prompt").and_then(Value::as_str) {
                Some(prompt) if !prompt.trim().is_empty() => interpreter.interpret(prompt),
                _ => failure("invalid"),
            }
        }
        _ => {}
    }
    let store = match Store::open(database) {
        Ok(store) => store,
        Err(_) => return failure("storage"),
    };
    match kind {
        "save" => {
            let Some(input) = request
                .get("clip")
                .cloned()
                .and_then(|clip| serde_json::from_value::<WebClipInput>(clip).ok())
            else {
                return failure("invalid");
            };
            match store.put_web_clip(input) {
                Ok(write) => {
                    let (status, clip) = match write {
                        WebClipWrite::Created(clip) => ("created", clip),
                        WebClipWrite::Replayed(clip) => ("replayed", clip),
                    };
                    json!({
                        "ok": true,
                        "status": status,
                        "request_id": clip.request_id,
                        "document_key": clip.document_key,
                    })
                }
                Err(error) => clip_error(&error),
            }
        }
        "documents" => {
            let limit = request
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(50)
                .clamp(1, 500) as usize;
            match store.web_clip_documents(limit) {
                Ok(documents) => json!({ "ok": true, "documents": documents }),
                Err(error) => clip_error(&error),
            }
        }
        "document" => {
            let Some(key) = request.get("key").and_then(Value::as_str) else {
                return failure("invalid");
            };
            match store.web_clip_document(key) {
                Ok(Some(document)) => json!({ "ok": true, "document": document }),
                Ok(None) => failure("not_found"),
                Err(error) => clip_error(&error),
            }
        }
        "settings_get" => match store.clipper_settings() {
            Ok(settings) => json!({
                "ok": true,
                "revision": settings.revision,
                "templates": settings.templates,
                "interpreter": interpreter.describe(),
            }),
            Err(error) => settings_error(&error),
        },
        "settings_seed" => match request.get("templates") {
            Some(templates) => match store.seed_clipper_templates(templates) {
                Ok(settings) => json!({ "ok": true, "revision": settings.revision }),
                Err(error) => settings_error(&error),
            },
            None => failure("invalid"),
        },
        "settings_put" => {
            let base = request.get("base_revision").and_then(Value::as_i64);
            match (request.get("templates"), base) {
                (Some(templates), Some(base)) => match store.put_clipper_templates(templates, base)
                {
                    Ok(settings) => json!({ "ok": true, "revision": settings.revision }),
                    Err(error) => settings_error(&error),
                },
                _ => failure("invalid"),
            }
        }
        _ => failure("unknown_request"),
    }
}

/// Serve clip requests on the socket until the process exits. Owner-only permissions on the
/// socket and its directory are what authorize a caller. Another live server on the same
/// socket leaves this one without a clip listener rather than taking the path over.
///
/// # Errors
/// The directory or socket cannot be created or restricted.
pub fn serve(socket: &Path, database: &Path, interpreter: &dyn Interpreter) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = socket.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    if std::os::unix::net::UnixStream::connect(socket).is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "another server answers on the clip socket",
        ));
    }
    let _ = std::fs::remove_file(socket);
    let listener = std::os::unix::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    // ponytail: one connection at a time; clips are human-paced. Spawn per connection if
    // a bulk import ever goes through this socket.
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
        let answer = match read_frame(&mut stream) {
            Ok(Some(request)) => handle(database, interpreter, &request),
            _ => failure("invalid"),
        };
        let _ = write_frame(&mut stream, &answer);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers without a model: echoes the prompt length, names a fixed model.
    pub(crate) struct Stub;
    impl Interpreter for Stub {
        fn describe(&self) -> Value {
            json!({ "available": true, "model": "stub" })
        }
        fn interpret(&self, prompt: &str) -> Value {
            json!({ "ok": true, "content": format!("{} chars", prompt.len()), "model": "stub" })
        }
    }

    #[test]
    fn the_socket_is_owner_only_and_relays_to_the_store() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("clip-sock-{}", std::process::id()));
        let socket = dir.join("s").join("clip.sock");
        let db = dir.join("comms.db");
        std::fs::create_dir_all(&dir).unwrap();
        let (s, d) = (socket.clone(), db.clone());
        std::thread::spawn(move || serve(&s, &d, &Stub));
        let started = std::time::Instant::now();
        while !socket.exists() && started.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            (mode(&socket), mode(socket.parent().unwrap())),
            (0o600, 0o700)
        );
        assert_eq!(relay(&socket, &json!({"type": "ping"}))["ok"], true);
        assert!(
            serve(&socket, &db, &Stub).is_err(),
            "a second server must not take the socket over"
        );
        let clip = json!({"type": "save", "clip": {
            "request_id": "r1", "url": "https://example.org/",
            "payload": {"version": 1, "behavior": "create", "markdown": "x"}}});
        assert_eq!(relay(&socket, &clip)["status"], "created");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn requests_are_answered_without_exposing_storage_detail() {
        let dir = std::env::temp_dir().join(format!("clip-handle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("comms.db");
        assert_eq!(handle(&db, &Stub, &json!({"type": "ping"}))["ok"], true);
        assert_eq!(
            handle(&db, &Stub, &json!({"type": "nope"}))["error"],
            "unknown_request"
        );
        assert_eq!(
            handle(&db, &Stub, &json!({"type": "save", "clip": 1}))["error"],
            "invalid"
        );
        let clip = json!({"type": "save", "clip": {
            "request_id": "r1", "url": "https://example.org/",
            "payload": {"version": 1, "behavior": "create", "markdown": "# Hi"}}});
        let saved = handle(&db, &Stub, &clip);
        assert_eq!(
            (saved["status"].as_str(), saved["document_key"].as_str()),
            (Some("created"), Some("clip:r1"))
        );
        assert_eq!(handle(&db, &Stub, &clip)["status"], "replayed");
        let listed = handle(&db, &Stub, &json!({"type": "documents"}));
        assert_eq!(listed["documents"][0]["markdown"], "# Hi");
        assert_eq!(
            handle(&db, &Stub, &json!({"type": "document", "key": "clip:r1"}))["document"]
                ["versions"],
            1
        );
        assert_eq!(
            handle(&db, &Stub, &json!({"type": "document", "key": "x"}))["error"],
            "not_found"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_and_interpretation_go_through_the_socket_requests() {
        let dir = std::env::temp_dir().join(format!("clip-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("comms.db");
        let ask = |request: Value| handle(&db, &Stub, &request);
        let empty = ask(json!({"type": "settings_get"}));
        assert_eq!(
            (empty["revision"].as_i64(), empty["templates"].is_null()),
            (Some(0), true)
        );
        assert_eq!(empty["interpreter"]["model"], "stub");
        let templates = json!([{"id": "t", "name": "Default", "behavior": "create"}]);
        assert_eq!(
            ask(json!({"type": "settings_seed", "templates": templates}))["revision"],
            1
        );
        assert_eq!(
            ask(json!({"type": "settings_seed", "templates": templates}))["error"],
            "already_seeded"
        );
        assert_eq!(
            ask(json!({"type": "settings_put", "templates": templates, "base_revision": 0}))
                ["error"],
            "stale"
        );
        assert_eq!(
            ask(json!({"type": "settings_put", "templates": templates, "base_revision": 1}))
                ["revision"],
            2
        );
        assert_eq!(
            ask(json!({"type": "settings_put", "templates": templates}))["error"],
            "invalid"
        );
        assert_eq!(ask(json!({"type": "settings_get"}))["templates"], templates);
        assert_eq!(
            ask(json!({"type": "interpret", "prompt": "abc"}))["content"],
            "3 chars"
        );
        assert_eq!(
            ask(json!({"type": "interpret", "prompt": " "}))["error"],
            "invalid"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
