//! Checks endpoint authentication only; it does not identify the responding process.

use std::io::Read;
use std::net::IpAddr;
use std::time::Duration;

use reqwest::Url;
use serde_json::{json, Value};

const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BODY: u64 = 64 * 1024;

/// The caller must first prove that `old_token` is unexpired and works with the old key,
/// and must control the service lifecycle. This probe does not establish process identity.
pub(crate) fn probe_rotation(
    endpoint: &str,
    new_token: &str,
    old_token: &str,
) -> Result<(), &'static str> {
    let url = loopback_url(endpoint)?;
    let client = sjel_http::builder(sjel_http::Purpose::new("sqld-rotation-probe"), TIMEOUT)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "rotation probe: HTTP client unavailable")?;
    probe_with(
        |token| {
            let mut request = client.post(url.clone()).json(&json!({
                "requests": [{"type": "execute", "stmt": {"sql": "SELECT 1"}}]
            }));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            let response = request.send().map_err(|_| ())?;
            let status = response.status().as_u16();
            if token == Some(new_token) && (200..300).contains(&status) {
                let mut body = Vec::new();
                response
                    .take(MAX_BODY + 1)
                    .read_to_end(&mut body)
                    .map_err(|_| ())?;
                if body.len() as u64 > MAX_BODY {
                    return Err(());
                }
                Ok((status, body))
            } else {
                Ok((status, Vec::new()))
            }
        },
        new_token,
        old_token,
    )
}

fn loopback_url(endpoint: &str) -> Result<Url, &'static str> {
    let mut url = Url::parse(endpoint).map_err(|_| "rotation probe: invalid loopback endpoint")?;
    let loopback = url
        .host_str()
        .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
        .is_some_and(|ip| ip.is_loopback());
    if url.scheme() != "http"
        || !loopback
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("rotation probe: invalid loopback endpoint");
    }
    url.set_path("/v2/pipeline");
    Ok(url)
}

fn probe_with(
    mut send: impl FnMut(Option<&str>) -> Result<(u16, Vec<u8>), ()>,
    new_token: &str,
    old_token: &str,
) -> Result<(), &'static str> {
    let (status, body) = send(Some(new_token)).map_err(|_| "rotation probe: request failed")?;
    if !(200..300).contains(&status) {
        return Err("rotation probe: new token was not accepted");
    }
    let body: Value =
        serde_json::from_slice(&body).map_err(|_| "rotation probe: invalid SQL response")?;
    let results = body
        .get("results")
        .and_then(Value::as_array)
        .ok_or("rotation probe: invalid SQL response")?;
    if results.len() != 1
        || results[0]["type"] != "ok"
        || results[0]["response"]["type"] != "execute"
        || results[0]["response"]["result"]["rows"] != json!([[{"type": "integer", "value": "1"}]])
    {
        return Err("rotation probe: invalid SQL response");
    }
    for token in [Some(old_token), None] {
        let (status, _) = send(token).map_err(|_| "rotation probe: request failed")?;
        if status != 401 && status != 403 {
            return Err("rotation probe: credential rejection not established");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::Instant;

    use libsql_auth::{Access, PrimaryKey};
    use ring::rand::{SecureRandom, SystemRandom};

    const NEW: &str = "new-test-token-secret";
    const OLD: &str = "old-test-token-secret";
    const OK: &str = r#"{"results":[{"type":"ok","response":{"type":"execute","result":{"cols":[{"name":"1"}],"rows":[[{"type":"integer","value":"1"}]]}}}]}"#;

    fn simulated(responses: Vec<Result<(u16, Vec<u8>), ()>>) -> Result<(), &'static str> {
        let mut responses = responses.into_iter();
        probe_with(|_| responses.next().unwrap(), NEW, OLD)
    }
    fn reply(status: u16, body: &str) -> Result<(u16, Vec<u8>), ()> {
        Ok((status, body.as_bytes().to_vec()))
    }

    #[test]
    fn validates_statuses_and_sql_result_without_leaking_tokens() {
        assert!(simulated(vec![reply(200, OK), reply(401, ""), reply(403, "")]).is_ok());
        let cases = vec![
            vec![Err(())],
            vec![reply(401, "")],
            vec![reply(200, "not json")],
            vec![reply(200, "{}")],
            vec![reply(
                200,
                r#"{"results":[{"type":"ok","response":{"type":"execute","result":{}}}]}"#,
            )],
            vec![reply(200, r#"{"results":[{"type":"error"}]}"#)],
            vec![reply(
                200,
                r#"{"results":[{"type":"ok","response":{"type":"close"}}]}"#,
            )],
            vec![reply(200, OK), Err(())],
            vec![reply(200, OK), reply(200, "")],
            vec![reply(200, OK), reply(401, ""), Err(())],
            vec![reply(200, OK), reply(403, ""), reply(500, "")],
        ];
        for case in cases {
            let error = simulated(case).unwrap_err();
            assert!(!error.contains(NEW) && !error.contains(OLD));
        }
    }

    #[test]
    fn rejects_non_loopback_and_ambiguous_urls() {
        for endpoint in [
            "http://example.com",
            "http://localhost",
            "http://192.168.1.1",
            "https://127.0.0.1",
            "http://user:pass@127.0.0.1",
            "http://127.0.0.1/?token=secret",
            "http://127.0.0.1/path",
            "invalid",
        ] {
            let error = loopback_url(endpoint).unwrap_err();
            assert_eq!(error, "rotation probe: invalid loopback endpoint");
        }
        assert!(loopback_url("http://127.0.0.1:1234").is_ok());
        assert!(loopback_url("http://[::1]:1234").is_ok());
    }

    #[test]
    fn timeout_is_sanitized() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(TIMEOUT + Duration::from_millis(300));
        });
        let error = probe_rotation(&endpoint, NEW, OLD).unwrap_err();
        assert_eq!(error, "rotation probe: request failed");
        server.join().unwrap();
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let mut random = [0u8; 16];
            SystemRandom::new()
                .fill(&mut random)
                .expect("test directory randomness");
            let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            let path = std::env::temp_dir().join(format!("sqld-probe-{name}"));
            std::fs::create_dir(&path).expect("create disposable test directory");
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct TestServer(Child);

    impl Drop for TestServer {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn start_sqld(binary: &Path, dir: &Path, public_key: &str) -> (TestServer, SocketAddr) {
        let key_file = dir.join("public-key");
        std::fs::write(&key_file, public_key).expect("write synthetic public key");
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let address = listener.local_addr().expect("loopback address");
        drop(listener);
        let child = Command::new(binary)
            .env_clear()
            .current_dir(dir)
            .args(["--db-path", "database", "--http-listen-addr"])
            .arg(address.to_string())
            .arg("--auth-jwt-key-file")
            .arg(key_file)
            .args(["--no-welcome", "--disable-metrics"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start disposable sqld");
        (TestServer(child), address)
    }

    fn wait_for_sqld(server: &mut TestServer, address: SocketAddr, token: &str) {
        let client = sjel_http::builder(
            sjel_http::Purpose::new("sqld-test-ready"),
            Duration::from_millis(500),
        )
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build test client");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(
                server.0.try_wait().expect("check child").is_none(),
                "disposable sqld exited"
            );
            let response = client
                .post(format!("http://{address}/v2/pipeline"))
                .bearer_auth(token)
                .json(&json!({"requests": [{"type": "execute", "stmt": {"sql": "SELECT 1"}}]}))
                .send();
            if let Ok(response) = response {
                if response.status().is_success() {
                    let body: Value = response.json().expect("read SQL response");
                    assert!(body["results"][0]["type"] == "ok", "SQL execution failed");
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "disposable sqld did not become ready"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn pinned_sqld_rotation_probe_after_restart() {
        let binary = std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join("sqld"))
                    .find(|path| path.is_file())
            })
            .and_then(|path| path.canonicalize().ok());
        let Some(binary) = binary else {
            eprintln!("skipped: sqld 0.24.32 is not installed");
            return;
        };
        let version = match Command::new(&binary)
            .env_clear()
            .arg("--version")
            .stdin(Stdio::null())
            .output()
        {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("skipped: sqld 0.24.32 is not installed");
                return;
            }
            result => result.expect("run sqld --version"),
        };
        assert!(version.status.success(), "sqld --version failed");
        let version = String::from_utf8(version.stdout).expect("sqld version is UTF-8");
        assert!(
            version.split_whitespace().any(|part| part == "0.24.32"),
            "integration test requires sqld 0.24.32"
        );

        let dir = TestDir::new();
        let old_key = PrimaryKey::generate().expect("generate synthetic old key");
        let new_key = PrimaryKey::generate().expect("generate synthetic new key");
        let old_token = old_key
            .mint_for_days(None, Access::Write, 1)
            .expect("mint synthetic old token");
        let new_token = new_key
            .mint_for_days(None, Access::Write, 1)
            .expect("mint synthetic new token");
        let (mut old_server, old_address) =
            start_sqld(&binary, &dir.0, &old_key.public_key_base64url());
        wait_for_sqld(&mut old_server, old_address, &old_token);
        drop(old_server);
        let (mut new_server, new_address) =
            start_sqld(&binary, &dir.0, &new_key.public_key_base64url());
        wait_for_sqld(&mut new_server, new_address, &new_token);
        assert_eq!(
            probe_rotation(&format!("http://{new_address}"), &new_token, &old_token),
            Ok(())
        );
    }

    #[test]
    fn sends_three_pipeline_requests_to_loopback() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for (auth, status, body) in [
                (Some(NEW), "200 OK", OK),
                (Some(OLD), "401 Unauthorized", ""),
                (None, "403 Forbidden", ""),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut data = Vec::new();
                let mut buf = [0; 4096];
                loop {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&buf[..n]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&data[..end]);
                        let length: usize = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .and_then(|v| v.parse().ok())
                            })
                            .unwrap();
                        if data.len() < end + 4 + length {
                            continue;
                        }
                        assert!(headers.starts_with("POST /v2/pipeline HTTP/1.1"));
                        let header = headers
                            .lines()
                            .find(|l| l.to_ascii_lowercase().starts_with("authorization:"));
                        match auth {
                            Some(token) => assert!(
                                header == Some(format!("authorization: Bearer {token}").as_str()),
                                "incorrect authorization header"
                            ),
                            None => assert!(header.is_none()),
                        }
                        let value: Value =
                            serde_json::from_slice(&data[end + 4..end + 4 + length]).unwrap();
                        assert_eq!(value["requests"][0]["stmt"]["sql"], "SELECT 1");
                        break;
                    }
                }
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        assert_eq!(probe_rotation(&endpoint, NEW, OLD), Ok(()));
        server.join().unwrap();
    }
}
