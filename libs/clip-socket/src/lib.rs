//! The framed JSON protocol on comms' clip socket.
//!
//! `comms-server` listens on a Unix socket with owner-only permissions in the overlay data
//! directory; the Sjel Clipper's native messaging host and the GPUI app talk to it here. Every
//! message is a native messaging frame: a little-endian `u32` length, then that many bytes of
//! JSON. There is no TCP route, so nothing that proxies HTTP can reach it.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

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

/// Relay one request to the server's socket and return its answer. An absent or refusing
/// socket is `unavailable`, which the extension treats as "queue and retry".
#[cfg(unix)]
pub fn relay(socket: &Path, request: &Value) -> Value {
    let attempt = || -> std::io::Result<Value> {
        let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        write_frame(&mut stream, request)?;
        read_frame(&mut stream)?.ok_or_else(|| std::io::ErrorKind::UnexpectedEof.into())
    };
    attempt().unwrap_or_else(|_| json!({ "ok": false, "error": "unavailable" }))
}

/// std has no Unix socket off Unix, so the clip socket is always `unavailable` there.
#[cfg(not(unix))]
pub fn relay(_socket: &Path, _request: &Value) -> Value {
    json!({ "ok": false, "error": "unavailable" })
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
}
