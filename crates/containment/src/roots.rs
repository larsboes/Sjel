//! TLS roots for a contained run. The launcher reads them from the machine's credential store,
//! which the sandbox does not reach, and hands the child one PEM file.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The variables a runtime reads a CA bundle from. `NODE_EXTRA_CA_CERTS` adds to the roots a
/// runtime already carries.
pub const CA_VARIABLES: &[&str] = &[
    "SSL_CERT_FILE",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
    "GIT_SSL_CAINFO",
    "NODE_EXTRA_CA_CERTS",
];

/// Every certificate the machine trusts, as one PEM bundle: the login keychain's and the system
/// roots, which macOS keeps in a keychain of its own. None when neither answers.
pub fn export_roots() -> Option<String> {
    let mut bundle = String::new();
    for keychain in [None, Some(SYSTEM_ROOTS)] {
        if let Some(block) = keychain_certificates(keychain) {
            bundle.push_str(&block);
        }
    }
    (!bundle.is_empty()).then_some(bundle)
}

/// macOS keeps the public roots here, outside the login keychain that a bare query reads.
const SYSTEM_ROOTS: &str = "/System/Library/Keychains/SystemRootCertificates.keychain";

fn keychain_certificates(keychain: Option<&str>) -> Option<String> {
    let mut command = std::process::Command::new("/usr/bin/security");
    command.args(["find-certificate", "-a", "-p"]);
    if let Some(path) = keychain {
        command.arg(path);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(bundle_from(&output.stdout))
}

/// Keep the certificate blocks and drop everything else, so a stray label cannot reach a parser.
pub fn bundle_from(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut inside = false;
    for line in text.lines() {
        if line.contains("BEGIN CERTIFICATE") {
            inside = true;
        }
        if inside {
            out.push_str(line.trim_end());
            out.push('\n');
        }
        if line.contains("END CERTIFICATE") {
            inside = false;
        }
    }
    out
}

/// Write the bundle owner-only, beside the run. The caller removes it when the child exits.
pub fn write_bundle(bundle: &str, dir: &Path) -> Option<PathBuf> {
    let path = dir.join(format!("sjel-contain-roots-{}.pem", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode_owner_only()
        .open(&path)
        .ok()?;
    file.write_all(bundle.as_bytes()).ok()?;
    Some(path)
}

#[cfg(target_os = "macos")]
trait OwnerOnly {
    fn mode_owner_only(&mut self) -> &mut Self;
}

#[cfg(target_os = "macos")]
impl OwnerOnly for std::fs::OpenOptions {
    fn mode_owner_only(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt as _;
        self.mode(0o600)
    }
}

#[cfg(not(target_os = "macos"))]
trait OwnerOnly {
    fn mode_owner_only(&mut self) -> &mut Self;
}

#[cfg(not(target_os = "macos"))]
impl OwnerOnly for std::fs::OpenOptions {
    fn mode_owner_only(&mut self) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_keeps_certificates_and_drops_labels() {
        let raw = b"subject=CN=Probe\n-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\ntrailing label\n";
        let bundle = bundle_from(raw);
        assert!(bundle.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(bundle.contains("MIIB"));
        assert!(!bundle.contains("subject="));
        assert!(!bundle.contains("trailing label"));
    }

    #[test]
    fn an_owner_only_bundle_is_written_and_read_back() {
        let dir = std::env::temp_dir().join(format!("sjel-roots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = write_bundle(
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
            &dir,
        )
        .expect("write the bundle");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("MIIB"));
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
