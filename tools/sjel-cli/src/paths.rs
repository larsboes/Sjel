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
