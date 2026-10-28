//! The native messaging host, driven the way Chrome drives it: frames on stdin, answers on
//! stdout, the overlay named only by `SJEL_PERSONAL_ROOT`.

use std::io::Write;
use std::process::{Command, Stdio};

use comms::web_clip_socket::{read_frame, serve, write_frame};
use serde_json::json;

fn run_host(overlay: &std::path::Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_comms-clip-host"))
        .env_clear()
        .env("SJEL_PERSONAL_ROOT", overlay)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for request in requests {
        write_frame(&mut stdin, request).unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    let mut cursor = std::io::Cursor::new(output.stdout);
    std::iter::from_fn(|| read_frame(&mut cursor).unwrap()).collect()
}

#[test]
fn chrome_framed_saves_reach_the_store_and_a_missing_server_reads_as_unavailable() {
    let overlay = std::env::temp_dir().join(format!("clip-host-{}", std::process::id()));
    let socket = overlay.join("data").join("comms").join("clip.sock");
    std::fs::create_dir_all(&overlay).unwrap();
    let clip = json!({"type": "save", "clip": {
        "request_id": "r1", "url": "https://example.org/",
        "payload": {"version": 1, "behavior": "create", "markdown": "# Saved"}}});

    let answers = run_host(&overlay, &[json!({"type": "ping"}), clip.clone()]);
    assert_eq!(
        answers,
        vec![json!({"ok": false, "error": "unavailable"}); 2]
    );

    let (s, db) = (socket.clone(), overlay.join("comms.db"));
    std::thread::spawn(move || serve(&s, &db));
    let started = std::time::Instant::now();
    while !socket.exists() && started.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let answers = run_host(
        &overlay,
        &[clip.clone(), clip, json!({"type": "documents"})],
    );
    assert_eq!(answers[0]["status"], "created");
    assert_eq!(answers[1]["status"], "replayed");
    assert_eq!(answers[2]["documents"][0]["markdown"], "# Saved");

    assert_eq!(
        run_host(
            std::path::Path::new("/nonexistent"),
            &[json!({"type": "ping"})]
        )[0]["error"],
        "unavailable"
    );
    std::fs::remove_dir_all(overlay).unwrap();
}
