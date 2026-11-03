use std::{io::Read, time::Duration};

use serde::Deserialize;

pub fn decode_item_ref(reference: &str) -> Result<String, &'static str> {
    let hex = reference
        .strip_prefix("inv:")
        .ok_or("invalid Inventory reference")?
        .as_bytes();
    if hex.is_empty() || hex.len() > 512 || hex.len() % 2 != 0 {
        return Err("invalid Inventory reference length");
    }
    fn digit(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        }
    }
    let bytes: Option<Vec<u8>> = hex
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Some(digit(pair[0])? * 16 + digit(pair[1])?))
        .collect();
    String::from_utf8(bytes.ok_or("Inventory reference must use lowercase hex")?)
        .map_err(|_| "Inventory reference must encode UTF-8")
}

pub fn project_ref(path: &str) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::from("project:");
    for b in path.bytes() {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 15) as usize] as char);
    }
    out
}

#[derive(Deserialize)]
struct Identity {
    typed_ref: String,
    raw_id: String,
    revision: i64,
    state: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum VerifyError {
    Invalid,
    Missing,
    NotOwned,
    Unavailable,
}

pub fn verify_with_auth(
    base: &str,
    reference: &str,
    auth: sjel_server::InboundAuth,
) -> Result<i64, VerifyError> {
    let raw = decode_item_ref(reference).map_err(|_| VerifyError::Invalid)?;
    let parsed = reqwest::Url::parse(base).map_err(|_| VerifyError::Unavailable)?;
    if parsed.scheme() != "http"
        || parsed.host_str() != Some("127.0.0.1")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        return Err(VerifyError::Unavailable);
    }
    let url = format!("{}/api/item-refs/{reference}", base.trim_end_matches('/'));
    let client = sjel_http::builder(
        sjel_http::Purpose::new("projects-inventory"),
        Duration::from_secs(3),
    )
    .redirect(reqwest::redirect::Policy::none())
    .no_proxy()
    .build()
    .map_err(|_| VerifyError::Unavailable)?;
    let bearer = auth.bearer_header().ok_or(VerifyError::Unavailable)?;
    let response = client
        .get(&url)
        .header("authorization", bearer)
        .send()
        .map_err(|_| VerifyError::Unavailable)?;
    match response.status().as_u16() {
        404 => return Err(VerifyError::Missing),
        200 => {}
        _ => return Err(VerifyError::Unavailable),
    }
    let mut bytes = Vec::new();
    response
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| VerifyError::Unavailable)?;
    if bytes.len() > 8192 {
        return Err(VerifyError::Unavailable);
    }
    let answer: Identity = serde_json::from_slice(&bytes).map_err(|_| VerifyError::Unavailable)?;
    if answer.typed_ref != reference || answer.raw_id != raw || answer.revision < 1 {
        return Err(VerifyError::Unavailable);
    }
    if answer.state.as_deref() != Some("owned") {
        return Err(VerifyError::NotOwned);
    }
    Ok(answer.revision)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identities_are_canonical_and_paths_are_reversible_hex() {
        assert_eq!(decode_item_ref("inv:613ac3a9").unwrap(), "a:é");
        assert_eq!(project_ref("a:é/Hub.md"), "project:613ac3a92f4875622e6d64");
        for bad in ["inv:", "inv:6A", "inv:ff", "inv:0", "inv:61:62", "x:61"] {
            assert!(decode_item_ref(bad).is_err());
        }
    }
    fn mock(status: &str, body: &str) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request);
            socket.write_all(reply.as_bytes()).unwrap();
        });
        format!("http://{address}")
    }

    fn verify(base: &str, reference: &str) -> Result<i64, VerifyError> {
        verify_with_auth(
            base,
            reference,
            sjel_server::InboundAuth::with_token(Some("synthetic-token".into())),
        )
    }

    #[test]
    fn owner_responses_are_checked_before_linking() {
        let owned = r#"{"typed_ref":"inv:61","raw_id":"a","revision":4,"state":"owned"}"#;
        assert_eq!(verify(&mock("200 OK", owned), "inv:61"), Ok(4));
        assert_eq!(
            verify(&mock("404 Not Found", "{}"), "inv:61"),
            Err(VerifyError::Missing)
        );
        assert_eq!(
            verify(&mock("200 OK", &owned.replace("owned", "gone")), "inv:61"),
            Err(VerifyError::NotOwned)
        );
        assert_eq!(
            verify(
                &mock("200 OK", &owned.replace("inv:61", "inv:62")),
                "inv:61"
            ),
            Err(VerifyError::Unavailable)
        );
        assert_eq!(
            verify(
                &mock(
                    "200 OK",
                    &owned.replace("\"raw_id\":\"a\"", "\"raw_id\":\"b\"")
                ),
                "inv:61"
            ),
            Err(VerifyError::Unavailable)
        );
    }

    #[test]
    fn offline_and_non_loopback_fail_closed() {
        assert_eq!(
            verify("http://127.0.0.1:1", "inv:61"),
            Err(VerifyError::Unavailable)
        );
        assert_eq!(
            verify("http://example.com", "inv:61"),
            Err(VerifyError::Unavailable)
        );
    }
}
