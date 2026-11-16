use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
    process::Command,
    time::Duration,
};

use comms::{
    mail_evidence_eval::{self, Corpus, Report, Review, Source},
    mail_evidence_source::{NativeReview, NativeRun, REPRESENTATION},
};

type Error = Box<dyn std::error::Error>;
const MAX_ARTIFACT: usize = 4 * 1024 * 1024;
const MAX_NATIVE_REQUEST: usize = 2 * 1024 * 1024;

fn main() {
    if let Err(error) = execute() {
        eprintln!("mail evidence evaluation: {error}");
        std::process::exit(2);
    }
}

fn validate_output(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let overlay = sjel_config::overlay_root()
        .ok_or("private overlay unavailable")?
        .canonicalize()?;
    let parent = path
        .parent()
        .ok_or("output needs a parent directory")?
        .canonicalize()?;
    if !parent.starts_with(&overlay) {
        return Err("evaluation artifacts must stay in the private overlay".into());
    }
    if path.exists() {
        return Err("refusing to overwrite an existing evaluation artifact".into());
    }
    Ok(())
}

fn private_write(
    path: &Path,
    value: &impl serde::Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_output(path)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(input: impl Read) -> Result<T, Error> {
    let mut bytes = Vec::new();
    input
        .take(MAX_ARTIFACT as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ARTIFACT {
        return Err("evaluation input exceeds 4 MiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid evaluation JSON".into())
}

fn native_post<T: serde::de::DeserializeOwned>(
    action: &str,
    body: &impl serde::Serialize,
    timeout: Duration,
) -> Result<T, Error> {
    if sjel_config::in_agent_session() {
        return Err(
            "native evaluation requires an owner terminal; agent sessions are refused".into(),
        );
    }
    let bytes = serde_json::to_vec(body)?;
    if bytes.len() > MAX_NATIVE_REQUEST {
        return Err("native evaluation request exceeds 2 MiB".into());
    }
    let discovered = Command::new("sjel")
        .args(["capability", "url", "comms"])
        .output()
        .map_err(|_| "capability discovery unavailable")?;
    if !discovered.status.success() || discovered.stdout.len() > 4096 {
        return Err("capability discovery failed".into());
    }
    let base = std::str::from_utf8(&discovered.stdout)
        .ok()
        .and_then(|s| comms::mail_evidence_scan::local_base(s.trim()))
        .ok_or("native evaluation requires an HTTP loopback origin")?;
    let url = format!("{base}/triage/mail-evidence/evaluation/{action}");
    let http = sjel_http::builder(sjel_http::Purpose::new("comms-mail-evidence-eval"), timeout)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|_| "native evaluation client unavailable")?;
    let cfg = comms::config::Config::load();
    let response = sjel_server::InboundAuth::with_capability_loopback_auth(
        http.post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(bytes),
        &url,
        cfg.api_secret,
    )
    .send()
    .map_err(|_| "native evaluation transport failed; no artifact written")?;
    native_response(response)
}

fn native_response<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, Error> {
    if !response.status().is_success() {
        // Error bodies are not retained: middleware responses can contain private context.
        return Err(format!(
            "native evaluation refused (HTTP {}); no artifact written",
            response.status().as_u16()
        )
        .into());
    }
    read_json(response)
}

fn native(args: &[String]) -> Result<(), Error> {
    if sjel_config::in_agent_session() {
        return Err(
            "native evaluation requires an owner terminal; agent sessions are refused".into(),
        );
    }
    let freeze = args[0] == "freeze-native";
    validate_output(Path::new(if freeze { &args[1] } else { &args[2] }))?;
    if freeze {
        let review: NativeReview = read_json(io::stdin())?;
        let corpus: Corpus = native_post("freeze", &review, Duration::from_secs(300))?;
        if corpus.representation != REPRESENTATION {
            return Err("native evaluation returned an unexpected representation".into());
        }
        private_write(Path::new(&args[1]), &corpus)?;
        println!(
            "frozen {} native cases; no message bodies written",
            corpus.cases.len()
        );
    } else {
        let corpus: Corpus = read_json(fs::File::open(&args[1])?)?;
        if corpus.representation != REPRESENTATION
            || corpus.cases.is_empty()
            || corpus.cases.len() > 50
        {
            return Err("a frozen native corpus of 1 to 50 cases is required".into());
        }
        let role = args.get(3).map_or("rules", String::as_str);
        let timeout = Duration::from_secs(
            300 + if role == "rules" {
                0
            } else {
                corpus.cases.len() as u64 * 120
            },
        );
        let request = NativeRun {
            corpus,
            role: role.into(),
        };
        let report: Report = native_post("run", &request, timeout)?;
        if report.role != role {
            return Err("native evaluation returned an unexpected role".into());
        }
        save_report(Path::new(&args[2]), &report)?;
    }
    Ok(())
}

fn save_report(path: &Path, report: &Report) -> Result<(), Error> {
    private_write(path, report)?;
    println!("{}", serde_json::to_string_pretty(report)?);
    if !report.failures.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn execute() -> Result<(), Error> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 2
        || !matches!(
            args[0].as_str(),
            "freeze" | "run" | "freeze-native" | "run-native"
        )
        || (matches!(args[0].as_str(), "freeze" | "freeze-native") && args.len() != 2)
        || (args[0] == "run" && args.len() != 4)
        || (args[0] == "run-native" && !matches!(args.len(), 3 | 4))
    {
        return Err("usage: comms-mail-evidence-eval freeze CORPUS < reviewed.json; run CORPUS REPORT ROLE < sources.json; freeze-native CORPUS < native-review.json; run-native CORPUS REPORT [ROLE]".into());
    }
    if args[0].ends_with("-native") {
        return native(&args);
    }
    validate_output(Path::new(if args[0] == "freeze" {
        &args[1]
    } else {
        &args[2]
    }))?;
    if args[0] == "freeze" {
        let review: Review = read_json(io::stdin())?;
        let corpus = mail_evidence_eval::freeze(review)?;
        private_write(Path::new(&args[1]), &corpus)?;
        println!(
            "frozen {} cases; no message bodies written",
            corpus.cases.len()
        );
    } else {
        let corpus: Corpus = read_json(fs::File::open(&args[1])?)?;
        let sources: Vec<Source> = read_json(io::stdin())?;
        let cfg = comms::config::Config::load();
        let runtime = if args[3] == "rules" {
            sjel_runtime::Status::normal("rules-only-evaluation")
        } else {
            sjel_runtime::current().map_err(|_| "runtime unavailable")?
        };
        let report = mail_evidence_eval::run(&corpus, &sources, &cfg, &runtime, &args[3])?;
        save_report(Path::new(&args[2]), &report)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn input_limits_and_invalid_json_do_not_echo_private_text() {
        let error = read_json::<Corpus>(b"private malformed body".as_slice())
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "invalid evaluation JSON");
        let bytes = vec![b' '; MAX_ARTIFACT + 1];
        assert_eq!(
            read_json::<Corpus>(bytes.as_slice())
                .err()
                .unwrap()
                .to_string(),
            "evaluation input exceeds 4 MiB"
        );
    }

    #[test]
    fn unknown_source_fields_cannot_enter_retained_reports() {
        let body = serde_json::json!({
            "started_unix_seconds": 1, "corpus_hash": "hash", "role": "rules",
            "extraction_version": "purchase-sections-v1", "backend": "sjel", "model": "none",
            "context_tokens": null, "conventions_hash": "hash", "initial_admission_failure": null,
            "cases": {}, "failures": {}
        });
        let report: Report = read_json(body.to_string().as_bytes()).unwrap();
        assert_eq!(report.extraction_version, "purchase-sections-v1");
        let mut body = body;
        body["mail"] = "private body".into();
        assert!(read_json::<Report>(body.to_string().as_bytes()).is_err());
        body.as_object_mut().unwrap().remove("mail");
        body["cases"]["case-1"] =
            serde_json::to_value(mail_evidence_eval::Counts::default()).unwrap();
        body["cases"]["case-1"]["mail"] = "private body".into();
        assert!(read_json::<Report>(body.to_string().as_bytes()).is_err());
    }

    #[test]
    fn error_responses_discard_the_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            {
                use std::io::{BufRead, BufReader};
                let mut reader = BufReader::new(&mut stream);
                loop {
                    let mut line = String::new();
                    let count = reader.read_line(&mut line).unwrap();
                    assert!(count > 0);
                    if line == "\r\n" {
                        break;
                    }
                }
            }
            let body = "private error sentinel";
            write!(
                stream,
                "HTTP/1.1 409 Conflict\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let http = sjel_http::builder(
            sjel_http::Purpose::new("mail-eval-test"),
            Duration::from_secs(5),
        )
        .no_proxy()
        .build()
        .unwrap();
        let response = http.get(url).send().unwrap();
        let error = native_response::<Corpus>(response)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("HTTP 409"));
        assert!(!error.contains("sentinel"));
        server.join().unwrap();
    }
}
