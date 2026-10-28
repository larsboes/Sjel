//! The host-only path from the Sjel Clipper extension to the clip store.
//!
//! The browser starts `comms-clip-host` through native messaging; the host relays each
//! message over a Unix socket that only `comms-server` listens on. The socket is a file
//! with owner-only permissions in the overlay data directory, so there is no TCP route for
//! `sjel-status` to proxy and no bearer token in browser storage. Both hops use the native
//! messaging frame: a little-endian `u32` length, then that many bytes of JSON.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::store::{ClipError, Store, WebClipInput, WebClipWrite};

/// Largest frame accepted on either hop. Chrome allows 64 MiB towards a host; the store
/// refuses payloads above 2 MiB, so anything near this bound is already an error.
pub const MAX_FRAME_BYTES: u32 = 8 * 1024 * 1024;

/// The Chrome native messaging host name; the extension and the installed manifest use it.
pub const HOST_NAME: &str = "dev.sjel.clip";

/// `<overlay>/data/comms/clip.sock`, or `None` when no overlay is configured.
pub fn socket_path() -> Option<PathBuf> {
    sjel_config::overlay_data_dir("comms").map(|dir| dir.join("clip.sock"))
}

/// Read one frame. `Ok(None)` is a clean end of input.
///
/// # Errors
/// The input ends inside a frame, the frame exceeds [`MAX_FRAME_BYTES`], or it is not JSON.
pub fn read_frame(input: &mut impl Read) -> std::io::Result<Option<Value>> {
    let mut length = [0u8; 4];
    match input.read_exact(&mut length) {
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        other => other?,
    }
    let length = u32::from_le_bytes(length);
    if length > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame exceeds the limit",
        ));
    }
    let mut body = vec![0u8; length as usize];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

/// Write one frame and flush it.
///
/// # Errors
/// The output fails, or the message exceeds [`MAX_FRAME_BYTES`].
pub fn write_frame(output: &mut impl Write, message: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(message)?;
    let length = u32::try_from(body.len())
        .ok()
        .filter(|length| *length <= MAX_FRAME_BYTES)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "frame exceeds the limit")
        })?;
    output.write_all(&length.to_le_bytes())?;
    output.write_all(&body)?;
    output.flush()
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
/// Requests: `{"type":"ping"}`, `{"type":"save","clip":{request_id,url,payload}}`,
/// `{"type":"documents","limit":N}` and `{"type":"document","key":K}`.
pub fn handle(database: &Path, request: &Value) -> Value {
    let kind = request.get("type").and_then(Value::as_str).unwrap_or("");
    if kind == "ping" {
        return json!({ "ok": true, "service": "comms", "version": 1 });
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
        _ => failure("unknown_request"),
    }
}

/// Relay one request to the server's socket and return its answer. An absent or refusing
/// socket is `unavailable`, which the extension treats as "queue and retry".
pub fn relay(socket: &Path, request: &Value) -> Value {
    let attempt = || -> std::io::Result<Value> {
        let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        write_frame(&mut stream, request)?;
        read_frame(&mut stream)?.ok_or_else(|| std::io::ErrorKind::UnexpectedEof.into())
    };
    attempt().unwrap_or_else(|_| failure("unavailable"))
}

/// Serve clip requests on the socket until the process exits. Owner-only permissions on the
/// socket and its directory are what authorize a caller. Another live server on the same
/// socket leaves this one without a clip listener rather than taking the path over.
///
/// # Errors
/// The directory or socket cannot be created or restricted.
pub fn serve(socket: &Path, database: &Path) -> std::io::Result<()> {
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
            Ok(Some(request)) => handle(database, &request),
            _ => failure("invalid"),
        };
        let _ = write_frame(&mut stream, &answer);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_refuse_oversized_or_truncated_input() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &json!({"type": "ping"})).unwrap();
        write_frame(&mut buffer, &json!({"n": 2})).unwrap();
        let mut input = std::io::Cursor::new(buffer);
        assert_eq!(
            read_frame(&mut input).unwrap(),
            Some(json!({"type": "ping"}))
        );
        assert_eq!(read_frame(&mut input).unwrap(), Some(json!({"n": 2})));
        assert_eq!(read_frame(&mut input).unwrap(), None);

        let oversized = (MAX_FRAME_BYTES + 1).to_le_bytes();
        assert!(read_frame(&mut std::io::Cursor::new(oversized)).is_err());
        let truncated = [10u8, 0, 0, 0, b'{'];
        assert!(read_frame(&mut std::io::Cursor::new(truncated)).is_err());
    }

    #[test]
    fn an_absent_socket_reads_as_unavailable() {
        let socket = std::env::temp_dir().join(format!("no-clip-{}.sock", std::process::id()));
        assert_eq!(
            relay(&socket, &json!({"type": "ping"})),
            json!({"ok": false, "error": "unavailable"})
        );
    }

    #[test]
    fn the_socket_is_owner_only_and_relays_to_the_store() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("clip-sock-{}", std::process::id()));
        let socket = dir.join("s").join("clip.sock");
        let db = dir.join("comms.db");
        std::fs::create_dir_all(&dir).unwrap();
        let (s, d) = (socket.clone(), db.clone());
        std::thread::spawn(move || serve(&s, &d));
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
            serve(&socket, &db).is_err(),
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
        assert_eq!(handle(&db, &json!({"type": "ping"}))["ok"], true);
        assert_eq!(
            handle(&db, &json!({"type": "nope"}))["error"],
            "unknown_request"
        );
        assert_eq!(
            handle(&db, &json!({"type": "save", "clip": 1}))["error"],
            "invalid"
        );
        let clip = json!({"type": "save", "clip": {
            "request_id": "r1", "url": "https://example.org/",
            "payload": {"version": 1, "behavior": "create", "markdown": "# Hi"}}});
        let saved = handle(&db, &clip);
        assert_eq!(
            (saved["status"].as_str(), saved["document_key"].as_str()),
            (Some("created"), Some("clip:r1"))
        );
        assert_eq!(handle(&db, &clip)["status"], "replayed");
        let listed = handle(&db, &json!({"type": "documents"}));
        assert_eq!(listed["documents"][0]["markdown"], "# Hi");
        assert_eq!(
            handle(&db, &json!({"type": "document", "key": "clip:r1"}))["document"]["versions"],
            1
        );
        assert_eq!(
            handle(&db, &json!({"type": "document", "key": "x"}))["error"],
            "not_found"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
