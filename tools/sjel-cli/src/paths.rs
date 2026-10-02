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
    caps_dir: PathBuf,
    overlay_caps_dir: Option<PathBuf>,
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
            overlay_caps_dir: non_empty_var("SJEL_OVERLAY_CAPS_DIR").map(PathBuf::from),
            caps_dir,
            root,
        })
    }

    /// machine.toml, when it exists on disk.
    pub fn machine_toml(&self) -> Option<&Path> {
        self.machine_toml.as_deref().filter(|p| p.is_file())
    }

    /// The service.toml that declares capability `name` — the rule of `axon_manifest_for` in
    /// tools/lib/paths.sh. A name declared in both the core and the overlay capability roots is
    /// two services sharing a name, and resolves to nothing rather than to either one.
    pub fn manifest_for(&self, name: &str) -> Option<PathBuf> {
        let core = self.caps_dir.join(name).join("service.toml");
        let overlay = self
            .overlay_caps_dir
            .as_ref()
            .map(|d| d.join(name).join("service.toml"));
        match (core.is_file(), overlay.filter(|p| p.is_file())) {
            (true, Some(_)) => None,
            (true, None) => Some(core),
            (false, Some(o)) => Some(o),
            // A spine component (today: dashboard/) carries its manifest at the repo root.
            (false, None) => {
                Some(self.root.join(name).join("service.toml")).filter(|p| p.is_file())
            }
        }
    }
}

fn non_empty_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}
