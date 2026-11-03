use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use libsql_auth::{Access, PrimaryKey};
use ring::rand::{SecureRandom, SystemRandom};

struct TestDir(std::path::PathBuf);

impl TestDir {
    fn new() -> Self {
        let mut random = [0u8; 16];
        SystemRandom::new()
            .fill(&mut random)
            .expect("random test directory name");
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let path = std::env::temp_dir().join(format!("sqld-rotation-{name}"));
        std::fs::create_dir(&path).expect("create isolated database directory");
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_server(binary: &Path, dir: &Path, public_key: &str) -> (Server, SocketAddr) {
    let key_file = dir.join("public-key");
    std::fs::write(&key_file, public_key).expect("write test public key");
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a loopback port");
    let address = listener.local_addr().expect("read loopback address");
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
        .expect("start sqld");
    (Server(child), address)
}

fn request(address: SocketAddr, token: Option<&str>) -> std::io::Result<(u16, String)> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(300))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let body = r#"{"requests":[{"type":"execute","stmt":{"sql":"SELECT 1"}}]}"#;
    let auth = token
        .map(|value| format!("Authorization: Bearer {value}\r\n"))
        .unwrap_or_default();
    write!(stream, "POST /v2/pipeline HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| std::io::Error::other("invalid HTTP status"))?;
    Ok((status, response))
}

fn ready(server: &mut Server, address: SocketAddr, token: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        assert!(
            server.0.try_wait().expect("check sqld process").is_none(),
            "sqld exited during startup"
        );
        if let Ok((200, response)) = request(address, Some(token)) {
            let body = response
                .split_once("\r\n\r\n")
                .expect("HTTP response body")
                .1;
            let json: serde_json::Value = serde_json::from_str(body).expect("pipeline JSON");
            assert_eq!(json["results"][0]["type"], "ok", "SELECT 1 must execute");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "sqld did not accept the test credential before the startup deadline"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn rotating_the_public_key_revokes_old_tokens() {
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
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
    let old_key = PrimaryKey::generate().expect("generate old test key");
    let new_key = PrimaryKey::generate().expect("generate new test key");
    let old_token = old_key
        .mint_for_days(None, Access::Write, 1)
        .expect("mint old test token");
    let new_token = new_key
        .mint_for_days(None, Access::Write, 1)
        .expect("mint new test token");

    let (mut old_server, old_address) =
        start_server(&binary, &dir.0, &old_key.public_key_base64url());
    ready(&mut old_server, old_address, &old_token);
    drop(old_server);

    let (mut new_server, new_address) =
        start_server(&binary, &dir.0, &new_key.public_key_base64url());
    ready(&mut new_server, new_address, &new_token);
    for token in [Some(old_token.as_str()), None] {
        let (status, _) = request(new_address, token).expect("request against rotated server");
        assert!(
            matches!(status, 401 | 403),
            "rotated server must explicitly reject old or absent credentials; got {status}"
        );
    }
}
