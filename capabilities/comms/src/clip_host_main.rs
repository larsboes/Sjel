//! `comms-clip-host` — the Chrome native messaging host for the Sjel Clipper extension.
//!
//! Chrome starts it per connection and talks over stdin and stdout. It holds no state and
//! opens no database: each message is relayed to `comms-server`'s clip socket, and an absent
//! server is answered with `unavailable` so the extension keeps the clip in its queue.

use comms::web_clip_socket::{read_frame, relay, socket_path, write_frame};
use serde_json::json;

fn main() {
    let socket = socket_path();
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    loop {
        let answer = match read_frame(&mut input) {
            Ok(Some(request)) => match &socket {
                Some(socket) => relay(socket, &request),
                None => json!({ "ok": false, "error": "unconfigured" }),
            },
            Ok(None) => return,
            Err(_) => {
                // A broken frame leaves the stream position unknown, so answer once and stop.
                let _ = write_frame(&mut output, &json!({ "ok": false, "error": "invalid" }));
                return;
            }
        };
        if write_frame(&mut output, &answer).is_err() {
            return;
        }
    }
}
