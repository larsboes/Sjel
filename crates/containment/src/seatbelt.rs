use std::path::{Path, PathBuf};

/// System locations a contained process may read: the dynamic linker, frameworks, shells,
/// command-line tools and Homebrew. Nothing under a user's home.
const SYSTEM_READ: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/System",
    "/Library/Developer",
    "/Library/Apple",
    "/Library/Preferences",
    "/private/etc",
    "/private/var/db/timezone",
    "/private/var/db/xcode_select_link",
    "/private/var/select",
    "/opt/homebrew",
    "/Applications/Xcode.app",
    "/dev",
];

/// Toolchain directories under the home directory, read-only. `~/.cargo` is limited to the
/// binaries and the download caches: `credentials.toml` stays out of reach.
const HOME_TOOLCHAIN_READ: &[&str] = &[
    ".rustup",
    ".cargo/bin",
    ".cargo/registry",
    ".cargo/git",
    ".bun/bin",
    ".bun/install/cache",
    ".local/bin",
];

/// Builder for macOS sandbox-exec Seatbelt profiles. Reads and writes are denied by default;
/// each allowed path is listed.
#[derive(Debug, Clone)]
pub struct SeatbeltProfile {
    pub project_dir: PathBuf,
    pub read_paths: Vec<PathBuf>,
    pub read_write_paths: Vec<PathBuf>,
    pub loopback_only: bool,
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A Seatbelt string literal. Seatbelt strings take C escapes, so `"` and `\` are escaped.
fn quoted(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
}

impl SeatbeltProfile {
    /// The project read-write, system paths and the home toolchains read-only.
    pub fn new(project_dir: impl AsRef<Path>) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut read_paths: Vec<PathBuf> = SYSTEM_READ.iter().map(PathBuf::from).collect();
        if let Some(home) = &home {
            read_paths.extend(HOME_TOOLCHAIN_READ.iter().map(|rel| home.join(rel)));
        }
        Self {
            project_dir: canonical(project_dir.as_ref()),
            read_paths: read_paths
                .into_iter()
                .filter(|p| p.exists())
                .map(|p| canonical(&p))
                .collect(),
            read_write_paths: Vec::new(),
            loopback_only: true,
        }
    }

    pub fn with_read_path(mut self, path: impl AsRef<Path>) -> Self {
        self.read_paths.push(canonical(path.as_ref()));
        self
    }

    pub fn with_read_write_path(mut self, path: impl AsRef<Path>) -> Self {
        self.read_write_paths.push(canonical(path.as_ref()));
        self
    }

    /// Every ancestor of a granted path, from the root down, deduplicated: the directories a
    /// path walk passes through on its way to a path this run may read.
    pub fn ancestors(&self) -> Vec<PathBuf> {
        let mut seen = std::collections::BTreeSet::new();
        for path in std::iter::once(&self.project_dir)
            .chain(&self.read_paths)
            .chain(&self.read_write_paths)
        {
            for ancestor in path.ancestors().skip(1) {
                if ancestor != Path::new("/") {
                    seen.insert(ancestor.to_path_buf());
                }
            }
        }
        seen.into_iter().collect()
    }

    pub fn render(&self) -> String {
        let mut out = String::from("(version 1)\n(deny default)\n\n");

        out.push_str(
            "(allow process-exec process-fork signal sysctl-read mach-lookup ipc-posix-shm)\n",
        );
        // The Keychain answers through these services; a contained run reads no credential.
        out.push_str("(deny mach-lookup (global-name \"com.apple.SecurityServer\") (global-name \"com.apple.securityd.xpc\"))\n");
        // Metadata lets path resolution walk through parents. Listing a directory or reading a
        // file needs file-read-data, which only the paths below get.
        out.push_str("(allow file-read-metadata)\n");
        out.push_str("(allow file-read-data (literal \"/\"))\n");
        out.push_str("(allow file-write-data (literal \"/dev/null\") (regex #\"^/dev/tty\"))\n");

        for path in &self.read_paths {
            out.push_str(&format!("(allow file-read* (subpath {}))\n", quoted(path)));
        }
        for path in std::iter::once(&self.project_dir).chain(&self.read_write_paths) {
            out.push_str(&format!(
                "(allow file-read* file-write* (subpath {}))\n",
                quoted(path)
            ));
        }
        // A reader that opens each path component itself is refused at the first ancestor
        // without this: metadata answers stat, not an open. A directory literal admits the
        // names in it and no file inside it.
        for ancestor in self.ancestors() {
            out.push_str(&format!(
                "(allow file-read-data (literal {}))\n",
                quoted(&ancestor)
            ));
        }
        out.push('\n');

        if self.loopback_only {
            // Outbound traffic leaves only through the loopback egress proxy.
            out.push_str("(allow network-outbound (literal \"/private/var/run/mDNSResponder\") (remote ip \"localhost:*\"))\n");
            out.push_str("(allow network-inbound network-bind (local ip \"localhost:*\"))\n");
        } else {
            out.push_str("(allow network-outbound network-inbound network-bind)\n");
        }
        out
    }
}
