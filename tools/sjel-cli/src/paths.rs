//! Repository and overlay locations, read from the environment `tools/lib/paths.sh` exports.
//!
//! Nothing here derives a location. `paths.sh` owns the axon.local.toml → axon.toml overlay
//! order and the machine.toml lookup; re-deriving either here would be a second copy of a rule
//! that has already drifted once between shell and TypeScript (`libs/overlay/overlay.ts`).

use std::path::{Path, PathBuf};

pub struct Paths {
    pub root: PathBuf,
    /// `SJEL_MACHINE_TOML`. Unset or absent on a fresh clone before `tools/install.sh` runs.
    pub machine_toml: Option<PathBuf>,
    /// `SJEL_OVERLAY_ROOT`, the active deployment overlay.
    pub overlay_root: Option<PathBuf>,
    pub caps_dir: PathBuf,
    pub overlay_caps_dir: Option<PathBuf>,
}

/// Where a capability's service.toml is, by the rule of `axon_manifest_for` in
/// tools/lib/paths.sh.
pub enum Manifest {
    Found(PathBuf),
    None,
    /// Declared in both the core and the overlay capability root: two different services
    /// sharing a name. It resolves to neither.
    Duplicate(PathBuf, PathBuf),
}

impl Paths {
    pub fn from_env() -> Result<Self, String> {
        let root = non_empty_var("SJEL_ROOT").ok_or(
            "SJEL_ROOT is unset — run this through its tools/ launcher, which sources tools/lib/paths.sh",
        )?;
        let root = PathBuf::from(root);
        let caps_dir = non_empty_var("SJEL_CAPS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("capabilities"));
        Ok(Self {
            machine_toml: non_empty_var("SJEL_MACHINE_TOML").map(PathBuf::from),
            overlay_root: non_empty_var("SJEL_OVERLAY_ROOT").map(PathBuf::from),
            overlay_caps_dir: non_empty_var("SJEL_OVERLAY_CAPS_DIR").map(PathBuf::from),
            caps_dir,
            root,
        })
    }

    /// The same values, resolved by sourcing tools/lib/paths.sh in a child bash, for the `sjel`
    /// launcher, which must not source it itself. paths.sh treats an EXPORTED
    /// SJEL_OVERLAY_ROOT as a per-invocation override and then skips axon.local.toml's
    /// `machine = "<name>"`, so exporting its results from `sjel` would hand every tool `sjel`
    /// execs (doctor, storage, update) a different machine.toml than it resolves on its own.
    pub fn from_shell(root: &Path) -> Result<Self, String> {
        let out = std::process::Command::new("bash")
            .args([
                "-c",
                r#". "$1/tools/lib/paths.sh" && printf '%s\0' "$SJEL_ROOT" "$SJEL_MACHINE_TOML" "$SJEL_OVERLAY_ROOT" "$SJEL_CAPS_DIR" "$SJEL_OVERLAY_CAPS_DIR""#,
                "_",
            ])
            .arg(root)
            .stderr(std::process::Stdio::inherit())
            .output()
            .map_err(|e| format!("cannot run bash to source tools/lib/paths.sh: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let v: Vec<&str> = text.split('\0').collect();
        if !out.status.success() || v.len() < 5 {
            return Err("tools/lib/paths.sh could not resolve this checkout's overlay".to_owned());
        }
        let opt = |s: &str| (!s.is_empty()).then(|| PathBuf::from(s));
        Ok(Self {
            root: PathBuf::from(v[0]),
            machine_toml: opt(v[1]),
            overlay_root: opt(v[2]),
            caps_dir: PathBuf::from(v[3]),
            overlay_caps_dir: opt(v[4]),
        })
    }

    /// machine.toml, when it exists on disk.
    pub fn machine_toml(&self) -> Option<&Path> {
        self.machine_toml.as_deref().filter(|p| p.is_file())
    }

    pub fn manifest(&self, name: &str) -> Manifest {
        let core = self.caps_dir.join(name).join("service.toml");
        let overlay = self
            .overlay_caps_dir
            .as_ref()
            .map(|d| d.join(name).join("service.toml"))
            .filter(|p| p.is_file());
        match (core.is_file(), overlay) {
            (true, Some(o)) => Manifest::Duplicate(core, o),
            (true, None) => Manifest::Found(core),
            (false, Some(o)) => Manifest::Found(o),
            // A spine component (today: dashboard/) carries its manifest at the repo root.
            (false, None) => {
                let spine = self.root.join(name).join("service.toml");
                if spine.is_file() {
                    Manifest::Found(spine)
                } else {
                    Manifest::None
                }
            }
        }
    }

    /// The manifest, or `None` for both "none" and "declared twice".
    pub fn manifest_for(&self, name: &str) -> Option<PathBuf> {
        match self.manifest(name) {
            Manifest::Found(p) => Some(p),
            Manifest::None | Manifest::Duplicate(..) => None,
        }
    }

    /// A capability directory from whichever root holds it. A capability may exist before it
    /// declares a service, so this asks "is it real", not "does it run".
    pub fn cap_dir(&self, name: &str) -> Option<PathBuf> {
        let core = self.caps_dir.join(name);
        if core.is_dir() {
            return Some(core);
        }
        self.overlay_caps_dir
            .as_ref()
            .map(|d| d.join(name))
            .filter(|d| d.is_dir())
    }
}

/// A capability's declared port, read from its own `service.toml`.
///
/// The port is interpolated straight into `http://127.0.0.1:${port}` by the scheduled jobs that
/// call another capability over HTTP, so the one thing this reader must not return is something
/// that is not a port. [`port_in_manifest`] is that check; this is the file lookup around it.
///
/// A name declared in both roots is an error rather than a guess, exactly as `manifest_for`
/// treats it: the whole point of the two-root rule is that a duplicate is a person's mistake.
pub fn manifest_port(paths: &Paths, capability: &str) -> Result<String, String> {
    match paths.manifest(capability) {
        Manifest::Found(path) => std::fs::read_to_string(&path)
            .map_err(|e| format!("{}: {e}", path.display()))
            .and_then(|body| port_in_manifest(&body).map_err(|e| format!("{} {e}", path.display()))),
        Manifest::Duplicate(core, overlay) => Err(duplicate_message(capability, &core, &overlay)),
        Manifest::None => Err(format!(
            "no {}",
            paths.caps_dir.join(capability).join("service.toml").display()
        )),
    }
}

/// The port a `service.toml` body declares, or the reason it declares none.
///
/// The digits check is what keeps `http://127.0.0.1:${port}` a loopback URL, and the bearer
/// token a caller puts on that request a credential that never leaves this machine. Measured:
/// `new URL("http://127.0.0.1:1@evil.example/x").host` is `evil.example`, because the last `@`
/// before the path ends the userinfo — so a manifest whose port reads `1@evil.example` moves the
/// host, and everything sent to it goes along. Ported from `portInManifest` in
/// `tools/sparpreis-watch.ts` (the reader `tools/feed-sweep.ts` carried a copy of), whose test is
/// the case list at the bottom of this file.
pub fn port_in_manifest(body: &str) -> Result<String, String> {
    let line = body
        .split('\n')
        .find(|line| is_port_assignment(line))
        .ok_or_else(|| "declares no port".to_owned())?;
    let port = first_quoted(line).unwrap_or("");
    if port.is_empty() {
        return Err("declares no port".to_owned());
    }
    let is_digits = port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit());
    match port.parse::<u32>() {
        Ok(n) if is_digits && (1..=65535).contains(&n) => Ok(port.to_owned()),
        _ => Err(format!("declares a port that is not a TCP port: {port}")),
    }
}

/// `^port\s*=` with no leading whitespace, as the TypeScript's regex anchors it.
fn is_port_assignment(line: &str) -> bool {
    match line.strip_prefix("port") {
        Some(rest) => rest.trim_start().starts_with('='),
        None => false,
    }
}

/// The first `"..."` on a line, as `line.match(/"([^"]*)"/)` reads it.
fn first_quoted(line: &str) -> Option<&str> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// The message `axon_manifest_for` prints for a name declared in both roots.
pub fn duplicate_message(name: &str, core: &Path, overlay: &Path) -> String {
    format!(
        "paths.sh: capability '{name}' is declared in both roots:\n  {}\n  {}\nRename one — they are two different services sharing a name.",
        core.display(),
        overlay.display()
    )
}

fn non_empty_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_port_a_manifest_declares() {
        assert_eq!(
            port_in_manifest("name = \"comms\"\nport = \"8099\"\n").unwrap(),
            "8099"
        );
    }

    #[test]
    fn refuses_a_manifest_with_no_port_line() {
        assert_eq!(
            port_in_manifest("name = \"comms\"\n").unwrap_err(),
            "declares no port"
        );
    }

    /// Measured with bun: `new URL("http://127.0.0.1:1@evil.example/x").host === "evil.example"`.
    /// The last `@` before the path ends the userinfo, so this value moves the host off loopback
    /// and takes the Authorization header with it.
    #[test]
    fn refuses_a_port_that_would_move_the_host_off_loopback() {
        assert_eq!(
            port_in_manifest("port = \"1@evil.example\"\n").unwrap_err(),
            "declares a port that is not a TCP port: 1@evil.example"
        );
        assert_eq!(
            reqwest::Url::parse("http://127.0.0.1:1@evil.example/feed")
                .unwrap()
                .host_str()
                .unwrap(),
            "evil.example"
        );
    }

    #[test]
    fn refuses_a_port_carrying_a_path_a_space_or_a_scheme() {
        for bad in ["8099/../x", "80 99", "https://evil.example", "-1", "80990"] {
            assert_eq!(
                port_in_manifest(&format!("port = \"{bad}\"\n")).unwrap_err(),
                format!("declares a port that is not a TCP port: {bad}")
            );
        }
    }
}
