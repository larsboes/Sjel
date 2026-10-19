//! Inspect release packages without activating them. Installation remains an explicit action.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path};
use std::process::{Command, ExitCode, Stdio};

const MAX_MANIFEST: u64 = 1_048_576;
const MAX_PACKAGE: u64 = 1_073_741_824;
pub const USAGE: &str = "Usage: sjel distribution <verify|install-package> <archive> --sha256 <trusted-release-digest>\nVerify never installs. Install requires a writable packaged runtime of the same release; it never enables or starts a service.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Package {
    schema: u32,
    name: String,
    version: String,
    platform: String,
    requires: Vec<String>,
    files: Vec<PackageFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageFile {
    path: String,
    bytes: u64,
    sha256: String,
    executable: bool,
}

#[derive(Deserialize)]
struct Runtime {
    schema: u32,
    version: String,
    platform: String,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !value.starts_with('-')
}

fn relative_file(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn platform() -> String {
    let suffix = match std::env::consts::OS {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-gnu",
        other => other,
    };
    format!("{}-{suffix}", std::env::consts::ARCH)
}

impl Package {
    fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || !identifier(&self.name) || self.platform != platform() {
            return Err("unsupported package schema, name or host platform".into());
        }
        let version = self
            .version
            .strip_prefix('v')
            .ok_or("package version must start with v")?;
        let parts: Vec<_> = version.split('.').collect();
        if parts.len() != 3
            || parts
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err("package version must be vMAJOR.MINOR.PATCH".into());
        }
        if self
            .requires
            .iter()
            .any(|r| !identifier(r) || r == &self.name)
        {
            return Err("invalid package prerequisites".into());
        }
        if self.requires.iter().collect::<BTreeSet<_>>().len() != self.requires.len() {
            return Err("repeated package prerequisite".into());
        }
        if self.files.is_empty() || self.files.len() > 4096 {
            return Err("package must contain 1–4096 files".into());
        }
        let mut paths = BTreeSet::new();
        let mut total = 0_u64;
        for file in &self.files {
            if !relative_file(&file.path)
                || !digest_valid(&file.sha256)
                || !paths.insert(&file.path)
            {
                return Err("invalid or repeated package file".into());
            }
            total = total
                .checked_add(file.bytes)
                .ok_or("package size overflow")?;
            if total > MAX_PACKAGE {
                return Err("package exceeds the 1 GiB unpacked limit".into());
            }
        }
        if !paths.contains(&"service.toml".to_owned()) {
            return Err("package is missing service.toml".into());
        }
        for path in &paths {
            if Path::new(path).ancestors().skip(1).any(|parent| {
                parent
                    .to_str()
                    .is_some_and(|p| paths.contains(&p.to_owned()))
            }) {
                return Err("a package file is also used as a directory".into());
            }
        }
        Ok(())
    }
}

fn archive_file(archive: &Path, member: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut child = Command::new("tar")
        .arg("-xOf")
        .arg(archive)
        .arg("--")
        .arg(member)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot read package archive: {e}"))?;
    let mut bytes = Vec::new();
    let result = child
        .stdout
        .take()
        .ok_or("archive reader has no stdout")?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map(|_| bytes);
    let result = match result {
        Ok(bytes) if u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= limit => Ok(bytes),
        Ok(_) => Err("archive member exceeds its declared size".to_owned()),
        Err(e) => Err(format!("cannot read archive member: {e}")),
    };
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|e| format!("cannot wait for archive reader: {e}"))?;
    if !status.success() {
        return Err(format!("archive member missing or unreadable: {member}"));
    }
    result
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut input = File::open(path).map_err(|e| format!("cannot open package: {e}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex_bytes(&hash.finalize()))
}

fn inspect(archive: &Path, expected: &str) -> Result<Package, String> {
    if !digest_valid(expected) || hash_file(archive)? != expected {
        return Err("package SHA-256 does not match the trusted release catalogue".into());
    }
    let package: Package =
        serde_json::from_slice(&archive_file(archive, "package.json", MAX_MANIFEST)?)
            .map_err(|e| format!("invalid package manifest: {e}"))?;
    package.validate()?;
    for file in &package.files {
        let body = archive_file(archive, &format!("payload/{}", file.path), file.bytes)?;
        if u64::try_from(body.len()).unwrap_or(u64::MAX) != file.bytes
            || hex_bytes(&Sha256::digest(&body)) != file.sha256
        {
            return Err(format!("file does not match its manifest: {}", file.path));
        }
    }
    let manifest = archive_file(archive, "payload/service.toml", MAX_MANIFEST)?;
    validate_service(&package, &manifest)?;
    Ok(package)
}

fn validate_service(package: &Package, bytes: &[u8]) -> Result<(), String> {
    let source = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let table: toml::Table = toml::from_str(source).map_err(|e| e.to_string())?;
    if table.get("name").and_then(toml::Value::as_str) != Some(&package.name)
        || table.get("kind").and_then(toml::Value::as_str) != Some("process")
    {
        return Err("package must declare its own process capability".into());
    }
    if table.contains_key("build") || table.contains_key("panel_build") {
        return Err("binary packages must not run source builds".into());
    }
    let required: Vec<_> = table
        .get("requires")
        .and_then(toml::Value::as_array)
        .map(|rows| rows.iter().filter_map(toml::Value::as_str).collect())
        .unwrap_or_default();
    if required
        != package
            .requires
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    {
        return Err("package and service prerequisites disagree".into());
    }
    let expected = format!("capabilities/{}/bin/", package.name);
    let command = table
        .get("command")
        .and_then(toml::Value::as_array)
        .and_then(|args| args.first())
        .and_then(toml::Value::as_str)
        .ok_or("package has no executable")?;
    let local = command
        .strip_prefix(&format!("capabilities/{}/", package.name))
        .filter(|_| command.starts_with(&expected))
        .ok_or("package command escapes its owner")?;
    if !package
        .files
        .iter()
        .any(|file| file.path == local && file.executable)
    {
        return Err("package executable is missing".into());
    }
    Ok(())
}

fn install(root: &Path, archive: &Path, package: &Package) -> Result<(), String> {
    if !root.join("distribution.json").is_file()
        || root.join("Cargo.toml").exists()
        || root.join(".git").exists()
        || root
            .ancestors()
            .any(|path| path.extension().is_some_and(|ext| ext == "app"))
    {
        return Err(
            "package installation requires a packaged runtime, never a source checkout".into(),
        );
    }
    let runtime: Runtime = serde_json::from_slice(
        &fs::read(root.join("distribution.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("invalid runtime identity: {e}"))?;
    if runtime.schema != 1
        || runtime.version != package.version
        || runtime.platform != package.platform
    {
        return Err("package and runtime releases must match exactly".into());
    }
    for name in &package.requires {
        if !root
            .join("capabilities")
            .join(name)
            .join("service.toml")
            .is_file()
        {
            return Err(format!("install prerequisite {name} first"));
        }
    }
    let parent = root.join("capabilities");
    if parent
        .symlink_metadata()
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("capability root must not be a symlink".into());
    }
    let destination = parent.join(&package.name);
    if destination.symlink_metadata().is_ok() {
        return Err(
            "capability already exists; stop and back it up before a reviewed upgrade".into(),
        );
    }
    let stage = parent.join(format!(".{}-{}.partial", package.name, std::process::id()));
    fs::create_dir(&stage).map_err(|e| format!("cannot reserve package staging: {e}"))?;
    let result = (|| {
        for file in &package.files {
            let output = stage.join(&file.path);
            fs::create_dir_all(output.parent().ok_or("file has no parent")?)
                .map_err(|e| e.to_string())?;
            let body = archive_file(archive, &format!("payload/{}", file.path), file.bytes)?;
            if hex_bytes(&Sha256::digest(&body)) != file.sha256 {
                return Err("archive changed during installation".into());
            }
            let mut target = File::options()
                .write(true)
                .create_new(true)
                .open(&output)
                .map_err(|e| e.to_string())?;
            target.write_all(&body).map_err(|e| e.to_string())?;
            target.sync_all().map_err(|e| e.to_string())?;
            fs::set_permissions(
                &output,
                fs::Permissions::from_mode(if file.executable { 0o755 } else { 0o644 }),
            )
            .map_err(|e| e.to_string())?;
        }
        fs::rename(&stage, &destination).map_err(|e| format!("cannot activate package files: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

/// Validate, inspect or install a package without enabling or starting it.
///
/// # Errors
/// Prints the reason and exits unsuccessfully for invalid packages or unsupported actions.
pub fn run(root: &Path, args: &[String]) -> ExitCode {
    if matches!(args, [arg] if arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let result = (|| {
        let [action, archive, flag, digest] = args else {
            return Err("usage: sjel distribution <verify|install-package> <archive> --sha256 <trusted-digest>".into());
        };
        if flag != "--sha256" || !matches!(action.as_str(), "verify" | "install-package") {
            return Err("expected verify or install-package and --sha256".into());
        }
        let package = inspect(Path::new(archive), digest)?;
        if action == "install-package" {
            install(root, Path::new(archive), &package)?;
        }
        println!(
            "{} {} {}: {} verified files; {}",
            package.name,
            package.version,
            package.platform,
            package.files.len(),
            if action == "verify" {
                "nothing installed"
            } else {
                "installed; not enabled or started"
            }
        );
        Ok::<_, String>(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("distribution: {error}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> Package {
        Package {
            schema: 1,
            name: "calendar".into(),
            version: "v1.2.3".into(),
            platform: platform(),
            requires: vec![],
            files: vec![PackageFile {
                path: "service.toml".into(),
                bytes: 12,
                sha256: "a".repeat(64),
                executable: false,
            }],
        }
    }

    #[test]
    fn untrusted_paths_and_overlapping_files_are_refused() {
        for name in ["../outside", "/outside", "a/../b", "a//b", "a\\b", "./a"] {
            let mut p = package();
            p.files[0].path = name.into();
            assert!(p.validate().is_err());
        }
        let mut p = package();
        p.files.push(PackageFile {
            path: "service.toml/nested".into(),
            bytes: 0,
            sha256: "b".repeat(64),
            executable: false,
        });
        assert!(p.validate().is_err());
    }

    #[test]
    fn compatibility_and_decompression_limits_are_checked_before_extraction() {
        assert!(package().validate().is_ok());
        let mut p = package();
        p.platform = "wrong-host".into();
        assert!(p.validate().is_err());
        let mut p = package();
        p.files[0].bytes = MAX_PACKAGE + 1;
        assert!(p.validate().is_err());
        let mut p = package();
        p.version = "latest".into();
        assert!(p.validate().is_err());
    }

    #[test]
    fn archive_verification_reads_bounded_members_without_installing() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock")
            .as_nanos();
        let scratch =
            std::env::temp_dir().join(format!("sjel-archive-{}-{nonce}", std::process::id()));
        fs::create_dir_all(scratch.join("payload/bin")).expect("test directory");
        let service = b"name = 'calendar'\nkind = 'process'\ncommand = ['capabilities/calendar/bin/calendar-server']\n";
        let binary = b"synthetic fixture, never executed";
        fs::write(scratch.join("payload/service.toml"), service).expect("test manifest");
        fs::write(scratch.join("payload/bin/calendar-server"), binary).expect("test bytes");
        let value = serde_json::json!({"schema":1,"name":"calendar","version":"v1.2.3",
        "platform":platform(),"requires":[],"files":[
            {"path":"service.toml","bytes":service.len(),"sha256":hex_bytes(&Sha256::digest(service)),"executable":false},
            {"path":"bin/calendar-server","bytes":binary.len(),"sha256":hex_bytes(&Sha256::digest(binary)),"executable":true}
        ]});
        fs::write(scratch.join("package.json"), value.to_string()).expect("test metadata");
        let archive = scratch.join("package.tar.gz");
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .args(["package.json", "payload"])
            .current_dir(&scratch)
            .status()
            .expect("test tar")
            .success());
        let digest = hash_file(&archive).expect("test archive digest");
        assert_eq!(
            inspect(&archive, &digest).expect("valid archive").name,
            "calendar"
        );
        assert!(inspect(&archive, &"0".repeat(64)).is_err());
        assert!(archive_file(&archive, "payload/bin/calendar-server", 1).is_err());
        assert!(archive_file(&archive, "payload/missing", 1).is_err());
        fs::remove_dir_all(scratch).expect("test cleanup");
    }

    #[test]
    fn source_checkout_is_never_a_package_install_target() {
        assert!(install(
            Path::new("/nonexistent/sjel-source"),
            Path::new("unused"),
            &package()
        )
        .is_err());
    }
}
