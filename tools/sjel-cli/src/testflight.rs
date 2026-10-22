//! `sjel testflight`: the iOS app from this checkout to TestFlight in one command.
//!
//! `tools/distribution/ios-archive` owns the build (cargo, xcodegen, xcodebuild); this owns the
//! App Store Connect records around it, through the API key in `~/.appstoreconnect` (one
//! `AuthKey_<KEYID>.p8` and an `issuer_id`). The key is read here, never printed, and the token
//! signed from it lives fifteen minutes. Bundle ID and version come from the host's
//! `project.yml`, so the spec Xcode builds from is the one source for both. The Apple team is an
//! account fact, not a property of the source, so it sits beside the key in
//! `~/.appstoreconnect/team_id` and never in the public tree.
//!
//! What an App Manager key cannot do, measured 2026-10-22 against build 1318:
//! - create the app record: `POST /v1/apps` answers "'apps' does not allow 'CREATE'" for every
//!   role, so `app` prints the one step to do in the web UI;
//! - sign with a cloud-managed certificate (Admin only), so `profile` makes an App Store profile
//!   for the Apple Distribution identity in the login keychain and ios-archive exports manually.

use base64::{
    engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD, Engine as _,
};
use ring::signature::{EcdsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
use serde_json::{json, Value};
use sjel_http::{Idempotent, Purpose};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

pub const USAGE: &str = "Usage: sjel testflight [ship] [--notes <text>]
       sjel testflight app | profile | status | notes <text>

  ship      Archive this checkout, upload it, wait until App Store Connect has processed it,
            then set its What to Test note (default: the latest commit subjects). The build
            number is the commit count, or one past the newest uploaded build if that is higher.
  app       Register the bundle ID; print the one manual step if the app record is missing
  profile   Make or reuse the App Store profile for the local Apple Distribution identity
  status    The newest build App Store Connect has received, and its processing state
  notes     Set What to Test on the newest processed build

Every build reaches the internal group \"Team\", which ship creates when it is missing. Its
testers are added once in App Store Connect (TestFlight > Team > Testers).";

const API: &str = "https://api.appstoreconnect.apple.com";
const SKU: &str = "sjel-ios-001";
const SPEC: &str = "apps/gpui/ios/host/project.yml";
const PROFILE_SUFFIX: &str = "App Store";
const GROUP: &str = "Team";
/// Apple's limit on a What to Test note.
const NOTES_MAX: usize = 4000;
/// Processing took two minutes for build 1318; the ceiling allows for a busy queue.
const PROCESSING_LIMIT: Duration = Duration::from_secs(45 * 60);
const POLL: Duration = Duration::from_secs(30);

pub fn run(root: &Path, rest: &[String]) -> ExitCode {
    let words: Vec<&str> = rest.iter().map(String::as_str).collect();
    let result = match words.as_slice() {
        ["-h" | "--help"] => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        [] | ["ship"] => ship(root, None),
        ["ship", "--notes", text] | ["--notes", text] => ship(root, Some(text)),
        ["app"] => Spec::read(root).and_then(|spec| {
            let api = Api::new()?;
            api.app(&spec).map(|_| ())
        }),
        ["profile"] => Spec::read(root).and_then(|spec| Api::new()?.ensure_profile(&spec.bundle)),
        ["status"] => Spec::read(root).and_then(|spec| {
            let api = Api::new()?;
            let app = api.app(&spec)?;
            match api.newest_build(&app)? {
                Some(build) => println!("build {}: {}", build.version, build.state),
                None => println!("App Store Connect has received no build yet"),
            }
            Ok(())
        }),
        ["notes", text @ ..] if !text.is_empty() => Spec::read(root).and_then(|spec| {
            let api = Api::new()?;
            let app = api.app(&spec)?;
            let build = api.newest_build(&app)?.ok_or("no build uploaded yet")?;
            if build.state != "VALID" {
                return Err(format!(
                    "build {} is {}; wait for VALID",
                    build.version, build.state
                ));
            }
            api.set_notes(&build, &text.join(" "))
        }),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sjel testflight: {error}");
            ExitCode::FAILURE
        }
    }
}

fn ship(root: &Path, notes: Option<&str>) -> Result<(), String> {
    let spec = Spec::read(root)?;
    let api = Api::new()?;
    let app = api.app(&spec)?;
    api.ensure_profile(&spec.bundle)?;
    api.ensure_group(&app)?;

    // Apple refuses a build number it has seen, and two ships without a commit in between share
    // a commit count, so the number never goes below one past the newest upload.
    let commits: u64 = git(root, &["rev-list", "--count", "HEAD"])?
        .trim()
        .parse()
        .map_err(|_| "git rev-list --count HEAD did not print a number")?;
    let uploaded = api
        .newest_build(&app)?
        .and_then(|b| b.version.parse::<u64>().ok());
    let number = uploaded.map_or(commits, |n| commits.max(n + 1));

    let out = std::env::temp_dir().join(format!("sjel-testflight-{number}-{}", std::process::id()));
    println!(
        "archiving {} ({number}) into {}",
        spec.version,
        out.display()
    );
    let status = Command::new(root.join("tools/distribution/ios-archive"))
        .args(["--version", &spec.version, "--build", &number.to_string()])
        .args(["--team", &api.team, "--upload", "--out"])
        .arg(&out)
        .status()
        .map_err(|e| format!("could not run ios-archive: {e}"))?;
    if !status.success() {
        return Err(format!(
            "ios-archive failed ({status}); its output is above, the archive is in {}",
            out.display()
        ));
    }

    println!("uploaded; waiting for App Store Connect to process build {number}");
    let started = std::time::Instant::now();
    let build = loop {
        if let Some(build) = api
            .newest_build(&app)?
            .filter(|b| b.version == number.to_string())
        {
            match build.state.as_str() {
                "VALID" => break build,
                "INVALID" | "FAILED" => {
                    return Err(format!(
                        "App Store Connect marked build {number} {}; its email says why",
                        build.state
                    ))
                }
                _ => {}
            }
        }
        if started.elapsed() > PROCESSING_LIMIT {
            return Err(format!(
                "build {number} is still processing; run `sjel testflight status` later"
            ));
        }
        std::thread::sleep(POLL);
    };

    let text = match notes {
        Some(text) => text.to_owned(),
        None => default_notes(root)?,
    };
    api.set_notes(&build, &text)?;
    println!("build {number} is on TestFlight for \"{GROUP}\"");
    Ok(())
}

/// The latest commit subjects, and a line saying so when the build carried uncommitted work.
fn default_notes(root: &Path) -> Result<String, String> {
    let mut text = git(root, &["log", "-n", "8", "--format=- %s"])?;
    if !git(root, &["status", "--porcelain"])?.trim().is_empty() {
        text.push_str("- plus uncommitted changes in the working tree\n");
    }
    // Cut on a character boundary, never inside one.
    Ok(text.chars().take(NOTES_MAX).collect())
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !output.status.success() {
        return Err(format!("git {} failed", args.join(" ")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The values `project.yml` holds for every build. A line scan, because each key is a plain
/// `key: value` line and a YAML parser would be a new crate for three lookups.
struct Spec {
    bundle: String,
    version: String,
    name: String,
}

impl Spec {
    fn read(root: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(root.join(SPEC)).map_err(|e| format!("{SPEC}: {e}"))?;
        let value = |key: &str| {
            text.lines()
                .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix(':'))
                .map(|v| v.trim().trim_matches('"').to_owned())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("{SPEC} has no {key}"))
        };
        Ok(Self {
            bundle: value("PRODUCT_BUNDLE_IDENTIFIER")?,
            version: value("CFBundleShortVersionString")?,
            name: value("CFBundleDisplayName")?,
        })
    }
}

struct Build {
    id: String,
    version: String,
    state: String,
}

struct Api {
    client: reqwest::blocking::Client,
    key_id: String,
    issuer: String,
    /// The Apple Developer team that signs, from `~/.appstoreconnect/team_id`.
    team: String,
    key: EcdsaKeyPair,
}

impl Api {
    fn new() -> Result<Self, String> {
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unset")?);
        let dir = home.join(".appstoreconnect/private_keys");
        let keys: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("AuthKey_") && n.ends_with(".p8"))
            })
            .collect();
        let [path] = keys.as_slice() else {
            return Err(format!(
                "{} must hold exactly one AuthKey_*.p8, found {}",
                dir.display(),
                keys.len()
            ));
        };
        let key_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .trim_start_matches("AuthKey_")
            .to_owned();
        let issuer = std::fs::read_to_string(home.join(".appstoreconnect/issuer_id"))
            .map_err(|_| "~/.appstoreconnect/issuer_id is missing")?
            .trim()
            .to_owned();
        let team = std::fs::read_to_string(home.join(".appstoreconnect/team_id"))
            .map_err(|_| "~/.appstoreconnect/team_id is missing: write the 10-character Apple team ID there")?
            .trim()
            .to_owned();
        if team.len() != 10 || !team.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("~/.appstoreconnect/team_id is not a 10-character Apple team ID".into());
        }
        let pem = Zeroizing::new(
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?,
        );
        let body: Zeroizing<String> =
            Zeroizing::new(pem.lines().filter(|l| !l.starts_with("-----")).collect());
        let der = Zeroizing::new(
            STANDARD
                .decode(body.as_bytes())
                .map_err(|_| "the .p8 key is not base64 PEM")?,
        );
        let key = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &der,
            &ring::rand::SystemRandom::new(),
        )
        .map_err(|_| "the .p8 key is not a P-256 PKCS#8 key")?;
        let client = sjel_http::client(Purpose::new("testflight"), Duration::from_secs(60))
            .map_err(|e| format!("could not prepare an HTTP client: {e}"))?;
        Ok(Self {
            client,
            key_id,
            issuer,
            team,
            key,
        })
    }

    /// Minted per request: a ship waits longer than one token lives.
    fn token(&self) -> Result<String, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "clock before 1970")?
            .as_secs();
        let head = URL_SAFE_NO_PAD
            .encode(json!({"alg": "ES256", "kid": self.key_id, "typ": "JWT"}).to_string());
        let claims =
            json!({"iss": self.issuer, "iat": now, "exp": now + 900, "aud": "appstoreconnect-v1"});
        let signing_input = format!("{head}.{}", URL_SAFE_NO_PAD.encode(claims.to_string()));
        let signature = self
            .key
            .sign(&ring::rand::SystemRandom::new(), signing_input.as_bytes())
            .map_err(|_| "could not sign the App Store Connect token")?;
        Ok(format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature.as_ref())
        ))
    }

    fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let token = self.token()?;
        let idempotent = if method == reqwest::Method::GET {
            Idempotent::Yes
        } else {
            Idempotent::No
        };
        let response = sjel_http::send(
            || {
                let request = self
                    .client
                    .request(method.clone(), format!("{API}{path}"))
                    .bearer_auth(&token);
                match &body {
                    Some(body) => request.json(body),
                    None => request,
                }
            },
            idempotent,
        )
        .map_err(|e| format!("{method} {path}: {e}"))?;
        let status = response.status();
        let text = response.text().unwrap_or_default();
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let detail: Vec<String> = parsed["errors"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| {
                    format!(
                        "{}: {}",
                        e["title"].as_str().unwrap_or(""),
                        e["detail"].as_str().unwrap_or("")
                    )
                })
                .collect();
            return Err(format!("{method} {path} → {status} {}", detail.join("; ")));
        }
        Ok(parsed)
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        self.call(reqwest::Method::GET, path, None)
    }

    fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        self.call(reqwest::Method::POST, path, Some(body))
    }

    /// The bundle ID, registered if new, and the app record that only the web UI can make.
    fn app(&self, spec: &Spec) -> Result<String, String> {
        let bundle = &spec.bundle;
        let ids = self.get(&format!(
            "/v1/bundleIds?filter[identifier]={bundle}&fields[bundleIds]=identifier"
        ))?;
        if ids["data"].as_array().is_none_or(Vec::is_empty) {
            self.post(
                "/v1/bundleIds",
                json!({"data": {"type": "bundleIds",
                "attributes": {"identifier": bundle, "name": spec.name, "platform": "IOS"}}}),
            )?;
            println!("bundle id {bundle} registered");
        }
        let apps = self.get(&format!(
            "/v1/apps?filter[bundleId]={bundle}&fields[apps]=name"
        ))?;
        match apps["data"][0]["id"].as_str() {
            Some(id) => Ok(id.to_owned()),
            None => Err(format!(
                "no app record for {bundle}. Create it once at https://appstoreconnect.apple.com/apps > + > New App: \
                 platform iOS, bundle ID {bundle}, SKU {SKU}, any free name (the home screen shows {})",
                spec.name
            )),
        }
    }

    fn ensure_profile(&self, bundle: &str) -> Result<(), String> {
        let cert = self.distribution_certificate()?;
        let name = format!("{bundle} {PROFILE_SUFFIX}");
        let found = self.get(&format!(
            "/v1/profiles?filter[name]={}&filter[profileState]=ACTIVE&fields[profiles]=uuid,profileContent",
            name.replace(' ', "%20")
        ))?;
        let profile = match found["data"].get(0) {
            Some(profile) => profile.clone(),
            None => {
                let ids = self.get(&format!("/v1/bundleIds?filter[identifier]={bundle}"))?;
                let bundle_id = ids["data"][0]["id"]
                    .as_str()
                    .ok_or("bundle id is not registered; run `sjel testflight app`")?;
                let created = self.post(
                    "/v1/profiles",
                    json!({"data": {"type": "profiles",
                    "attributes": {"name": name, "profileType": "IOS_APP_STORE"},
                    "relationships": {
                        "bundleId": {"data": {"type": "bundleIds", "id": bundle_id}},
                        "certificates": {"data": [{"type": "certificates", "id": cert}]}}}}),
                )?;
                println!("profile \"{name}\" created");
                created["data"].clone()
            }
        };
        let uuid = profile["attributes"]["uuid"]
            .as_str()
            .ok_or("profile has no uuid")?;
        let content = STANDARD
            .decode(
                profile["attributes"]["profileContent"]
                    .as_str()
                    .unwrap_or_default(),
            )
            .map_err(|_| "profile content is not base64")?;
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unset")?);
        // Where Xcode 16 and later look; the older ~/Library/MobileDevice path is no longer read.
        let dir = home.join("Library/Developer/Xcode/UserData/Provisioning Profiles");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(dir.join(format!("{uuid}.mobileprovision")), content)
            .map_err(|e| format!("profile: {e}"))?;
        println!("profile \"{name}\" installed");
        Ok(())
    }

    /// The team certificate whose private key is in the login keychain, matched by the SHA-1
    /// fingerprint `security find-identity` prints and Apple's own copy of the certificate.
    fn distribution_certificate(&self) -> Result<String, String> {
        let output = Command::new("security")
            .args(["find-identity", "-v", "-p", "codesigning"])
            .output()
            .map_err(|e| format!("security: {e}"))?;
        let local: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.contains("\"Apple Distribution:"))
            .filter_map(|line| line.split_whitespace().nth(1).map(str::to_ascii_uppercase))
            .collect();
        if local.is_empty() {
            return Err("no Apple Distribution identity in the login keychain".to_owned());
        }
        let certs = self.get("/v1/certificates?filter[certificateType]=DISTRIBUTION,IOS_DISTRIBUTION&fields[certificates]=certificateContent&limit=200")?;
        for cert in certs["data"].as_array().into_iter().flatten() {
            let Ok(der) = STANDARD.decode(
                cert["attributes"]["certificateContent"]
                    .as_str()
                    .unwrap_or_default(),
            ) else {
                continue;
            };
            let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, &der);
            let fingerprint: String = digest.as_ref().iter().map(|b| format!("{b:02X}")).collect();
            if local.contains(&fingerprint) {
                return Ok(cert["id"].as_str().unwrap_or_default().to_owned());
            }
        }
        Err("the Apple Distribution identity in this keychain is not one of the team's certificates".to_owned())
    }

    /// An internal group that sees every build: no Beta App Review, nothing to assign per build.
    fn ensure_group(&self, app: &str) -> Result<(), String> {
        let groups = self.get(&format!("/v1/betaGroups?filter[app]={app}&fields[betaGroups]=isInternalGroup,hasAccessToAllBuilds"))?;
        let exists = groups["data"].as_array().into_iter().flatten().any(|g| {
            g["attributes"]["isInternalGroup"].as_bool() == Some(true)
                && g["attributes"]["hasAccessToAllBuilds"].as_bool() == Some(true)
        });
        if !exists {
            self.post("/v1/betaGroups", json!({"data": {"type": "betaGroups",
                "attributes": {"name": GROUP, "isInternalGroup": true, "hasAccessToAllBuilds": true},
                "relationships": {"app": {"data": {"type": "apps", "id": app}}}}}))?;
            println!(
                "internal group \"{GROUP}\" created; add its testers once in App Store Connect"
            );
        }
        Ok(())
    }

    fn newest_build(&self, app: &str) -> Result<Option<Build>, String> {
        let answer = self.get(&format!("/v1/builds?filter[app]={app}&sort=-uploadedDate&limit=1&fields[builds]=version,processingState"))?;
        Ok(answer["data"].get(0).map(|b| Build {
            id: b["id"].as_str().unwrap_or_default().to_owned(),
            version: b["attributes"]["version"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            state: b["attributes"]["processingState"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        }))
    }

    /// A new build has no beta localisation, so the first note creates one.
    fn set_notes(&self, build: &Build, text: &str) -> Result<(), String> {
        let list = self.get(&format!(
            "/v1/builds/{}/betaBuildLocalizations?fields[betaBuildLocalizations]=locale",
            build.id
        ))?;
        match list["data"][0]["id"].as_str() {
            Some(id) => {
                self.call(reqwest::Method::PATCH, &format!("/v1/betaBuildLocalizations/{id}"), Some(json!({"data": {
                    "type": "betaBuildLocalizations", "id": id, "attributes": {"whatsNew": text}}})))?;
            }
            None => {
                self.post(
                    "/v1/betaBuildLocalizations",
                    json!({"data": {"type": "betaBuildLocalizations",
                    "attributes": {"locale": "en-US", "whatsNew": text},
                    "relationships": {"build": {"data": {"type": "builds", "id": build.id}}}}}),
                )?;
            }
        }
        println!("What to Test set on build {}", build.version);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_reads_the_host_project() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let spec = Spec::read(&root).expect("project.yml parses");
        assert_eq!(spec.bundle, "dev.sjel.ios");
        assert!(spec
            .version
            .split('.')
            .all(|part| part.parse::<u32>().is_ok()));
    }
}
