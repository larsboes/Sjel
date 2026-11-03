//! The macOS privacy grants Sjel needs from the person, and where each one is given.
//!
//! The shell runs under launchd through the signed `sjel-fda-launcher`
//! (`tools/fda-launcher`, `full_disk_access` in `capabilities/sjel-status/service.toml`), so
//! every process it starts, the storage report included, reads guarded folders such as the
//! Photos library and the Music folder under that one grant. Full Disk Access never prompts:
//! the person adds the launcher with + in System Settings.

use std::path::PathBuf;
use std::process::Command;

pub struct Grant {
    pub title: &'static str,
    pub why: &'static str,
    /// Relative to the overlay root: the binary the grant is for.
    pub binary: &'static str,
    /// The System Settings pane that holds this grant.
    pub pane: &'static str,
}

pub const GRANTS: [Grant; 1] = [Grant {
    title: "Full Disk Access for sjel-fda-launcher",
    why: "The Sjel shell runs under this launcher. One grant lets the storage report measure the \
          Photos library and the Music folder without asking again. In System Settings, click +, \
          then choose the launcher that Show in Finder selects.",
    binary: "bin/sjel-fda-launcher",
    pane: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
}];

/// The binary the grant is for, when the overlay root is known and the launcher is installed.
pub fn installed(grant: &Grant) -> Option<PathBuf> {
    let root = std::env::var_os("SJEL_PERSONAL_ROOT")?;
    let path = PathBuf::from(root).join(grant.binary);
    path.is_file().then_some(path)
}

pub fn reveal(path: &PathBuf) {
    let _ = Command::new("/usr/bin/open").arg("-R").arg(path).spawn();
}

pub fn open_pane(grant: &Grant) {
    let _ = Command::new("/usr/bin/open").arg(grant.pane).spawn();
}
