use std::path::{Path, PathBuf};

/// Builder for macOS sandbox-exec Seatbelt profiles.
#[derive(Debug, Clone)]
pub struct SeatbeltProfile {
    pub project_dir: PathBuf,
    pub additional_read_write_paths: Vec<PathBuf>,
    pub loopback_only: bool,
}

impl SeatbeltProfile {
    pub fn new(project_dir: impl AsRef<Path>) -> Self {
        let canonical_project = std::fs::canonicalize(project_dir.as_ref())
            .unwrap_or_else(|_| project_dir.as_ref().to_path_buf());
        Self {
            project_dir: canonical_project,
            additional_read_write_paths: Vec::new(),
            loopback_only: true,
        }
    }

    pub fn with_read_write_path(mut self, path: impl AsRef<Path>) -> Self {
        if let Ok(canon) = std::fs::canonicalize(path.as_ref()) {
            self.additional_read_write_paths.push(canon);
        } else {
            self.additional_read_write_paths.push(path.as_ref().to_path_buf());
        }
        self
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("(version 1)\n(deny default)\n\n");

        // Basic execution, IPC, and system information
        out.push_str(";; Basic execution, IPC, and system information\n");
        out.push_str("(allow process-exec)\n");
        out.push_str("(allow process-fork)\n");
        out.push_str("(allow sysctl-read)\n");
        out.push_str("(allow mach-lookup)\n");
        out.push_str("(allow signal (target self))\n\n");

        // Read access for dyld, system tools, and frameworks
        out.push_str(";; Read access for system runtimes, toolchains, and dynamic linkers\n");
        out.push_str("(allow file-read-data)\n");
        out.push_str("(allow file-read-metadata)\n\n");

        // Project read/write
        out.push_str(";; Project directory read/write access\n");
        out.push_str(&format!(
            "(allow file-read* file-write* (subpath \"{}\"))\n",
            self.project_dir.display()
        ));
        for path in &self.additional_read_write_paths {
            out.push_str(&format!(
                "(allow file-read* file-write* (subpath \"{}\"))\n",
                path.display()
            ));
        }
        out.push_str("\n");

        // Network: loopback only if requested
        if self.loopback_only {
            out.push_str(";; Network: loopback and resolver socket only\n");
            out.push_str("(allow network-outbound\n");
            out.push_str("    (literal \"/private/var/run/mDNSResponder\")\n");
            out.push_str("    (remote tcp \"localhost:*\")\n");
            out.push_str("    (remote udp \"localhost:*\")\n");
            out.push_str(")\n");
            out.push_str("(allow network-inbound\n");
            out.push_str("    (local tcp \"localhost:*\")\n");
            out.push_str("    (local udp \"localhost:*\")\n");
            out.push_str(")\n");
            out.push_str("(allow network-bind\n");
            out.push_str("    (local tcp \"localhost:*\")\n");
            out.push_str("    (local udp \"localhost:*\")\n");
            out.push_str(")\n");
        } else {
            out.push_str("(allow network-outbound)\n");
            out.push_str("(allow network-inbound)\n");
            out.push_str("(allow network-bind)\n");
        }

        out
    }
}
