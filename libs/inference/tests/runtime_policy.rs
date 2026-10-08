use sjel_inference::{InferenceConfig, TextRole};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn policy(root: &Path, allow: &[&str]) {
    let path = root.join("config/runtime/test.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::json!({"selection":"on-the-go", "manual_power":"battery", "allow":allow, "revision":1}).to_string()).unwrap();
}

#[test]
fn runtime_child() {
    let Ok(root) = std::env::var("SJEL_RUNTIME_TEST_CHILD") else { return };
    let root = Path::new(&root);
    let port = std::env::var("SJEL_RUNTIME_TEST_PORT").unwrap();
    let config: InferenceConfig = serde_json::json!({
        "backends": {
            "local": {"api":"openai", "base_url": format!("http://127.0.0.1:{port}/v1")},
            "foundation-models": {"api":"openai", "base_url": format!("http://127.0.0.1:{port}/v1")},
            "remote": {"api":"openai", "base_url": "https://example.invalid/v1"}
        },
        "roles": {
            "embedding": {"backend":"local", "model":"small"},
            "summarization_light": {"backend":"local", "model":"small"},
            "afm": {"backend":"foundation-models", "model":"apple-foundationmodel", "max_input_tokens":4096},
            "peer": {"backend":"remote", "model":"large"}
        }
    }).to_string().parse().unwrap();
    let cached = config.role("embedding").unwrap();
    let input = ["text".to_owned()];
    assert!(cached.embed(&input, TextRole::Document).unwrap_err().starts_with("deferred by runtime profile:"));
    assert!(cached.rerank("query", &input).unwrap_err().starts_with("deferred by runtime profile:"));
    assert!(!cached.model_reachable());
    assert!(config.role("summarization_light").unwrap().runtime_admission().is_err());
    assert!(config.role("peer").unwrap().runtime_admission().is_err());
    let afm = config.role("afm").unwrap();
    assert!(afm.runtime_admission().is_ok());
    let body = serde_json::json!({"model":"apple-foundationmodel","messages":[{"role":"user","content":"Hi"}],"max_tokens":100});
    let mut wrong = body.clone();
    wrong["model"] = serde_json::json!("other");
    assert!(afm.admit_chat_request(&wrong).is_err());
    wrong = body;
    wrong["messages"][0]["content"] = serde_json::json!("x".repeat(4096));
    assert!(afm.admit_chat_request(&wrong).is_err());

    // Only this final operation is permitted to reach the mock. A cached role must see
    // changed preferences, and allowing bulk work alone never allows another backend.
    policy(root, &["bulk-indexing"]);
    assert!(cached.runtime_admission().is_err());
    policy(root, &["bulk-indexing", "other-local-models"]);
    assert!(cached.runtime_admission().is_ok());
    assert_eq!(cached.embed(&input, TextRole::Document).unwrap(), vec![vec![1.0, 0.0]]);
    policy(root, &[]);
    assert!(cached.runtime_admission().is_err());
}

#[test]
fn afm_health_requires_explicit_model_availability() {
    for (payload, available) in [
        (r#"{"model_available":true}"#, true),
        (r#"{"model_available":false}"#, false),
        (r#"{"status":"ok"}"#, false),
        ("not JSON", false),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with("GET /health "));
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" { break }
            }
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).unwrap();
        });
        assert_eq!(sjel_inference::afm_available(&base), available);
        server.join().unwrap();
    }
}

#[test]
fn profile_prevents_probes_and_requests_until_explicitly_enabled() {
    let root = std::env::temp_dir().join(format!("sjel-inference-runtime-{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
    policy(&root, &[]);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut requests = Vec::new();
        while std::time::Instant::now() < until {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    requests.push(line.clone());
                    let mut length = 0;
                    loop {
                        line.clear();
                        reader.read_line(&mut line).unwrap();
                        if line == "\r\n" { break }
                        if let Some((key, value)) = line.split_once(':') {
                            if key.eq_ignore_ascii_case("content-length") { length = value.trim().parse().unwrap(); }
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(body["model"], "small");
                    let response = r#"{"data":[{"index":0,"embedding":[1.0,0.0]}]}"#;
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
                    return requests;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(10)),
                Err(e) => panic!("mock server: {e}"),
            }
        }
        panic!("the explicitly enabled embedding request never arrived")
    });
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "runtime_child", "--nocapture"])
        .env("SJEL_RUNTIME_TEST_CHILD", &root)
        .env("SJEL_PERSONAL_ROOT", &root)
        .env("SJEL_MACHINE_TOML", root.join("config/machines/test.toml"))
        .env("SJEL_RUNTIME_TEST_PORT", port.to_string())
        .env_remove("SJEL_INFERENCE_BACKEND")
        .output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(server.join().unwrap(), vec!["POST /v1/embeddings HTTP/1.1\r\n"]);
    std::fs::remove_dir_all(root).unwrap();
}
