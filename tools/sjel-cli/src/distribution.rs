//! Inspect release packages without activating them. Installation remains an explicit action.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

const MAX_MANIFEST: u64 = 1_048_576;
const MAX_PACKAGE: u64 = 1_073_741_824;
pub const USAGE: &str = "Usage: sjel distribution <verify|install-package> <archive> --sha256 <trusted-release-digest>\n       sjel distribution list --json\n       sjel distribution uninstall-package <name> --preserve-data\n       sjel distribution place\nVerify never installs. Install requires a writable packaged runtime of the same release; it never enables or starts a service. List reports installed feature receipts and file integrity. Uninstall stops and disables the package, removes only receipt-owned files, and retains data. Place copies the app's packaged runtime to a writable per-user location and creates an empty overlay; it never starts a service or creates a credential.";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Package {
    schema: u32,
    name: String,
    version: String,
    platform: String,
    requires: Vec<String>,
    feature: FeatureMetadata,
    files: Vec<PackageFile>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FeatureMetadata {
    schema: u32,
    title: String,
    summary: String,
    category: String,
    recommend_topics: Vec<String>,
    data_access: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackageFile {
    path: String,
    bytes: u64,
    sha256: String,
    executable: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PackageReceipt {
    schema: u32,
    package_sha256: String,
    package: Package,
}

#[derive(Serialize)]
struct InstalledPackage {
    name: String,
    version: String,
    platform: String,
    feature: FeatureMetadata,
    requires: Vec<String>,
    package_sha256: String,
    integrity: &'static str,
}

#[derive(Deserialize)]
struct Runtime {
    schema: u32,
    version: String,
    platform: String,
    #[serde(default)]
    core: Vec<String>,
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
        let feature = &self.feature;
        if feature.schema != 1
            || feature.title.trim().is_empty()
            || feature.title.len() > 80
            || feature.summary.trim().is_empty()
            || feature.summary.len() > 240
            || feature.category.trim().is_empty()
            || feature.category.len() > 40
            || feature.recommend_topics.is_empty()
            || feature.recommend_topics.len() > 24
            || feature
                .recommend_topics
                .iter()
                .any(|topic| !identifier(topic) || topic.len() > 48)
            || feature
                .recommend_topics
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != feature.recommend_topics.len()
            || feature.data_access.len() > 32
            || feature
                .data_access
                .iter()
                .any(|item| item.trim().is_empty() || item.len() > 120)
            || feature.data_access.iter().collect::<BTreeSet<_>>().len()
                != feature.data_access.len()
        {
            return Err("invalid package feature metadata".into());
        }
        if self.files.is_empty() || self.files.len() > 4096 {
            return Err("package must contain 1–4096 files".into());
        }
        let mut paths = BTreeSet::new();
        let mut total = 0_u64;
        for file in &self.files {
            if !relative_file(&file.path)
                || file.path == "sjel-package.json"
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

fn install(
    root: &Path,
    archive: &Path,
    package: &Package,
    package_sha256: &str,
) -> Result<(), String> {
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
        let receipt = PackageReceipt {
            schema: 1,
            package_sha256: package_sha256.to_owned(),
            package: Package {
                schema: package.schema,
                name: package.name.clone(),
                version: package.version.clone(),
                platform: package.platform.clone(),
                requires: package.requires.clone(),
                feature: FeatureMetadata {
                    schema: package.feature.schema,
                    title: package.feature.title.clone(),
                    summary: package.feature.summary.clone(),
                    category: package.feature.category.clone(),
                    recommend_topics: package.feature.recommend_topics.clone(),
                    data_access: package.feature.data_access.clone(),
                },
                files: package
                    .files
                    .iter()
                    .map(|file| PackageFile {
                        path: file.path.clone(),
                        bytes: file.bytes,
                        sha256: file.sha256.clone(),
                        executable: file.executable,
                    })
                    .collect(),
            },
        };
        let receipt_path = stage.join("sjel-package.json");
        let mut receipt_file = File::options()
            .write(true)
            .create_new(true)
            .open(&receipt_path)
            .map_err(|e| format!("cannot create package receipt: {e}"))?;
        serde_json::to_writer(&mut receipt_file, &receipt)
            .map_err(|e| format!("cannot write package receipt: {e}"))?;
        receipt_file
            .write_all(b"\n")
            .and_then(|()| receipt_file.sync_all())
            .map_err(|e| format!("cannot sync package receipt: {e}"))?;
        fs::rename(&stage, &destination).map_err(|e| format!("cannot activate package files: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

fn package_files_match(directory: &Path, package: &Package) -> Result<bool, String> {
    for file in &package.files {
        let mut path = directory.to_path_buf();
        let components: Vec<_> = Path::new(&file.path).components().collect();
        for (index, component) in components.iter().enumerate() {
            path.push(component.as_os_str());
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(format!("cannot inspect package file: {error}")),
            };
            if metadata.file_type().is_symlink()
                || (index + 1 == components.len() && !metadata.is_file())
                || (index + 1 < components.len() && !metadata.is_dir())
            {
                return Ok(false);
            }
        }
        if hash_file(&path)? != file.sha256 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn installed_packages(root: &Path) -> Result<Vec<InstalledPackage>, String> {
    let runtime = packaged(root)?;
    let capabilities = root.join("capabilities");
    let metadata = match fs::symlink_metadata(&capabilities) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot inspect runtime capabilities: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("runtime capability root must be a directory, not a symlink".into());
    }
    let mut installed = Vec::new();
    for entry in fs::read_dir(capabilities).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let metadata = entry.file_type().map_err(|e| e.to_string())?;
        if metadata.is_symlink() || !metadata.is_dir() {
            continue;
        }
        let directory = entry.path();
        let receipt_path = directory.join("sjel-package.json");
        let receipt_metadata = match fs::symlink_metadata(&receipt_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("cannot inspect package receipt: {error}")),
        };
        if receipt_metadata.file_type().is_symlink() || !receipt_metadata.is_file() {
            return Err(format!(
                "invalid package receipt at {}",
                receipt_path.display()
            ));
        }
        let receipt: PackageReceipt = serde_json::from_slice(
            &fs::read(&receipt_path).map_err(|e| format!("cannot read package receipt: {e}"))?,
        )
        .map_err(|e| format!("invalid package receipt: {e}"))?;
        let folder = entry.file_name();
        let folder = folder.to_string_lossy();
        if receipt.schema != 1
            || !digest_valid(&receipt.package_sha256)
            || receipt.package.name != folder
            || receipt.package.version != runtime.version
            || receipt.package.platform != runtime.platform
        {
            return Err(format!(
                "package receipt does not match runtime at {}",
                directory.display()
            ));
        }
        receipt.package.validate()?;
        let integrity = if package_files_match(&directory, &receipt.package)? {
            "verified"
        } else {
            "modified"
        };
        let package = receipt.package;
        installed.push(InstalledPackage {
            name: package.name,
            version: package.version,
            platform: package.platform,
            feature: package.feature,
            requires: package.requires,
            package_sha256: receipt.package_sha256,
            integrity,
        });
    }
    installed.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(installed)
}

fn owned_tree(directory: &Path, package: &Package) -> Result<(), String> {
    fn visit(directory: &Path, root: &Path, expected: &BTreeSet<String>) -> Result<(), String> {
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let metadata = entry.file_type().map_err(|e| e.to_string())?;
            if metadata.is_symlink() {
                return Err(format!(
                    "uninstall refused a symlink at {}",
                    entry.path().display()
                ));
            }
            if metadata.is_dir() {
                visit(&entry.path(), root, expected)?;
            } else if metadata.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_str()
                    .ok_or("package path is not valid UTF-8")?
                    .to_owned();
                if !expected.contains(&relative) {
                    return Err(format!(
                        "uninstall refused unowned file {}; review or move it first",
                        entry.path().display()
                    ));
                }
            } else {
                return Err(format!(
                    "uninstall refused a special file at {}",
                    entry.path().display()
                ));
            }
        }
        Ok(())
    }

    let mut expected: BTreeSet<String> =
        package.files.iter().map(|file| file.path.clone()).collect();
    expected.insert("sjel-package.json".to_owned());
    visit(directory, directory, &expected)
}

fn prune_empty_dirs(directory: &Path) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            prune_empty_dirs(&entry.path())?;
            if fs::read_dir(entry.path())
                .map_err(|e| e.to_string())?
                .next()
                .is_none()
            {
                fs::remove_dir(entry.path()).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn run_runtime_tool(root: &Path, tool: &str, args: &[&str]) -> Result<(), String> {
    let output = Command::new(root.join("tools").join(tool))
        .args(args)
        .output()
        .map_err(|error| format!("cannot run {tool}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if detail.is_empty() {
            format!("{tool} failed with {}", output.status)
        } else {
            detail
        })
    }
}

fn uninstall_package(root: &Path, name: &str) -> Result<(), String> {
    if !identifier(name) {
        return Err("invalid package name".into());
    }
    let runtime = packaged(root)?;
    let capabilities = root.join("capabilities");
    let capability_metadata = fs::symlink_metadata(&capabilities).map_err(|e| e.to_string())?;
    if capability_metadata.file_type().is_symlink() || !capability_metadata.is_dir() {
        return Err("runtime capability root must be a directory, not a symlink".into());
    }
    let directory = capabilities.join(name);
    let directory_metadata = fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
    if directory_metadata.file_type().is_symlink() || !directory_metadata.is_dir() {
        return Err("package directory must be a real directory".into());
    }
    let receipt_path = directory.join("sjel-package.json");
    let receipt_metadata = fs::symlink_metadata(&receipt_path).map_err(|e| e.to_string())?;
    if receipt_metadata.file_type().is_symlink() || !receipt_metadata.is_file() {
        return Err("package receipt must be a regular file".into());
    }
    let receipt: PackageReceipt = serde_json::from_slice(
        &fs::read(&receipt_path).map_err(|e| format!("cannot read package receipt: {e}"))?,
    )
    .map_err(|e| format!("invalid package receipt: {e}"))?;
    if receipt.schema != 1
        || !digest_valid(&receipt.package_sha256)
        || receipt.package.name != name
        || receipt.package.version != runtime.version
        || receipt.package.platform != runtime.platform
    {
        return Err("package receipt does not match this runtime".into());
    }
    receipt.package.validate()?;
    if !package_files_match(&directory, &receipt.package)? {
        return Err(
            "package files differ from the receipt; repair or review them before uninstall".into(),
        );
    }
    owned_tree(&directory, &receipt.package)?;

    let paths = crate::paths::Paths::from_shell(root)?;
    let services = crate::registry::services(&paths).map_err(|error| error.msg)?;
    let dependents: Vec<_> = services
        .iter()
        .filter(|service| service.name != name && service.requires.iter().any(|dep| dep == name))
        .map(|service| service.name.as_str())
        .collect();
    if !dependents.is_empty() {
        return Err(format!(
            "cannot uninstall '{name}'; enabled features depend on it: {}",
            dependents.join(", ")
        ));
    }

    run_runtime_tool(root, "service-runner.sh", &["remove-persistence", name])?;
    run_runtime_tool(root, "service-runner.sh", &["stop", name, "--no-hold"])?;
    run_runtime_tool(root, "capability.sh", &["disable", name])?;

    for file in &receipt.package.files {
        fs::remove_file(directory.join(&file.path))
            .map_err(|e| format!("could not remove owned package file {}: {e}", file.path))?;
    }
    fs::remove_file(&receipt_path).map_err(|e| format!("could not remove package receipt: {e}"))?;
    prune_empty_dirs(&directory)?;
    fs::remove_dir(&directory)
        .map_err(|e| format!("package files removed but its directory is not empty: {e}"))?;
    Ok(())
}

/// The per-user home of placed runtimes and the overlay, relative to `$HOME`.
/// `desktop_launcher.rs` reads the same location; change both together.
pub const SUPPORT: &str = "Library/Application Support/Sjel";

/// A packaged runtime: release identity present, no source checkout markers.
fn packaged(root: &Path) -> Result<Runtime, String> {
    if root.join("Cargo.toml").exists() || root.join(".git").exists() {
        return Err("expected a packaged runtime, never a source checkout".into());
    }
    let runtime: Runtime = serde_json::from_slice(
        &fs::read(root.join("distribution.json"))
            .map_err(|e| format!("no packaged runtime identity: {e}"))?,
    )
    .map_err(|e| format!("invalid runtime identity: {e}"))?;
    if runtime.schema != 1 || !runtime.version.starts_with('v') || !relative_file(&runtime.version)
    {
        return Err("unsupported runtime identity".into());
    }
    Ok(runtime)
}

/// Copy a tree, keeping permission bits. Symlinks are refused, as `prepare.ts` refuses them.
fn copy_tree(from: &Path, to: &Path) -> Result<u64, String> {
    fs::create_dir(to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
    let mut bytes = 0;
    for entry in fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            return Err(format!(
                "runtime contains a symlink: {}",
                entry.path().display()
            ));
        }
        bytes += if kind.is_dir() {
            copy_tree(&entry.path(), &target)?
        } else {
            fs::copy(entry.path(), &target).map_err(|e| e.to_string())?
        };
    }
    Ok(bytes)
}

/// Copy the app's read-only runtime to `<support>/runtime/<version>` and create an empty
/// overlay beside it. Existing files are never overwritten, so a second run changes nothing.
/// Returns the placed runtime and the bytes copied (0 when already placed).
fn place(source: &Path, support: &Path) -> Result<(PathBuf, u64), String> {
    let runtime = packaged(source)?;
    let overlay = support.join("overlay");
    let text = overlay.to_str().ok_or("home path is not UTF-8")?;
    if text.contains(['"', '\\', '\n']) {
        return Err("home path cannot be written to sjel.local.toml".into());
    }
    let versions = support.join("runtime");
    fs::create_dir_all(&versions).map_err(|e| e.to_string())?;
    let destination = versions.join(&runtime.version);
    let mut bytes = 0;
    if !destination.join("distribution.json").is_file() {
        let stage = versions.join(format!(
            ".{}-{}.partial",
            runtime.version,
            std::process::id()
        ));
        let result = copy_tree(source, &stage).and_then(|copied| {
            bytes = copied;
            fs::rename(&stage, &destination)
                .map_err(|e| format!("cannot activate {}: {e}", destination.display()))
        });
        if result.is_err() {
            let _ = fs::remove_dir_all(&stage);
        }
        result?;
    }
    for dir in ["config", "data"] {
        fs::create_dir_all(overlay.join(dir)).map_err(|e| e.to_string())?;
    }
    // Core services are processes; container_runtime is required by platform.sh but unused.
    let capabilities = runtime
        .core
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    create_once(
        &overlay.join("config/machine.toml"),
        &format!("# Written by 'sjel distribution place'. See schemas/machine.toml.example.\nos = \"macos\"\ncontainer_runtime = \"docker\"\ncapabilities = [{capabilities}]\n"),
    )?;
    create_once(
        &destination.join("sjel.local.toml"),
        &format!("# Written by 'sjel distribution place'. tools/lib/paths.sh reads it.\noverlay = \"{text}\"\n"),
    )?;
    Ok((destination, bytes))
}

fn create_once(path: &Path, body: &str) -> Result<(), String> {
    match File::options().write(true).create_new(true).open(path) {
        Ok(mut file) => file.write_all(body.as_bytes()).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(format!("cannot write {}: {e}", path.display())),
    }
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
    if matches!(args, [arg] if arg == "place") {
        let home = std::env::var_os("HOME").filter(|home| !home.is_empty());
        let result = home
            .ok_or_else(|| "HOME is unset".to_owned())
            .and_then(|home| place(root, &Path::new(&home).join(SUPPORT)));
        return match result {
            Ok((runtime, bytes)) => {
                println!(
                    "runtime at {} ({bytes} bytes copied); no service started",
                    runtime.display()
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("distribution: {error}");
                ExitCode::from(1)
            }
        };
    }
    if matches!(args, [action, format] if action == "list" && format == "--json") {
        return match installed_packages(root).and_then(|packages| {
            serde_json::to_string(&packages).map_err(|e| format!("cannot encode package list: {e}"))
        }) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("distribution: {error}");
                ExitCode::from(1)
            }
        };
    }
    if let [action, name, preserve] = args {
        if action == "uninstall-package" && preserve == "--preserve-data" {
            return match uninstall_package(root, name) {
                Ok(()) => {
                    println!("{name} uninstalled; capability data retained");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("distribution: {error}");
                    ExitCode::from(1)
                }
            };
        }
    }
    let result = (|| {
        let [action, archive, flag, digest] = args else {
            return Err("usage: sjel distribution <verify|install-package> <archive> --sha256 <trusted-digest> | list --json".into());
        };
        if flag != "--sha256" || !matches!(action.as_str(), "verify" | "install-package") {
            return Err("expected verify or install-package and --sha256".into());
        }
        let package = inspect(Path::new(archive), digest)?;
        if action == "install-package" {
            install(root, Path::new(archive), &package, digest)?;
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
            feature: FeatureMetadata {
                schema: 1,
                title: "Calendar".into(),
                summary: "Manage events, availability, and planning contexts.".into(),
                category: "Planning".into(),
                recommend_topics: vec!["calendar".into()],
                data_access: vec!["calendar entries".into()],
            },
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
        "platform":platform(),"requires":[],
        "feature":{"schema":1,"title":"Calendar","summary":"Manage events, availability, and planning contexts.",
            "category":"Planning","recommend_topics":["calendar"],"data_access":["calendar entries"]},
        "files":[
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
    fn place_copies_once_and_never_overwrites() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock")
            .as_nanos();
        let scratch =
            std::env::temp_dir().join(format!("sjel-place-{}-{nonce}", std::process::id()));
        let bundle = scratch.join("Sjel.app/Contents/Resources/runtime");
        fs::create_dir_all(bundle.join("target/release")).expect("test bundle");
        fs::write(
            bundle.join("distribution.json"),
            r#"{"schema":1,"version":"v1.2.3","platform":"p","core":["sjel-status","store"]}"#,
        )
        .expect("test identity");
        let tool = bundle.join("target/release/sjel-cli");
        fs::write(&tool, b"bin").expect("test binary");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).expect("test mode");
        let support = scratch.join("Application Support/Sjel");

        let (runtime, bytes) = place(&bundle, &support).expect("first placement");
        assert_eq!(runtime, support.join("runtime/v1.2.3"));
        assert!(bytes > 0);
        let mode = fs::metadata(runtime.join("target/release/sjel-cli"))
            .expect("copied")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
        let machine =
            fs::read_to_string(support.join("overlay/config/machine.toml")).expect("machine");
        assert!(machine.contains(r#"capabilities = ["sjel-status", "store"]"#));
        let local = fs::read_to_string(runtime.join("sjel.local.toml")).expect("local");
        assert!(local.contains(&format!(
            "overlay = \"{}\"",
            support.join("overlay").display()
        )));

        fs::write(support.join("overlay/config/machine.toml"), "edited").expect("user edit");
        let (_, again) = place(&bundle, &support).expect("second placement");
        assert_eq!(again, 0);
        assert_eq!(
            fs::read_to_string(support.join("overlay/config/machine.toml")).expect("kept"),
            "edited"
        );

        fs::write(bundle.join("Cargo.toml"), "").expect("checkout marker");
        assert!(place(&bundle, &support).is_err());
        fs::remove_dir_all(scratch).expect("test cleanup");
    }

    #[test]
    fn source_checkout_is_never_a_package_install_target() {
        assert!(install(
            Path::new("/nonexistent/sjel-source"),
            Path::new("unused"),
            &package(),
            "unused"
        )
        .is_err());
    }
}
