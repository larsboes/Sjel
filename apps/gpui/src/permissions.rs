//! Locate the managed helper that macOS can grant Full Disk Access to.

use std::path::{Path, PathBuf};

pub(crate) const FULL_DISK_ACCESS_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";
const HELPER_PATH: &str = include_str!("../../../tools/fda-launcher/home-relative-path");

#[derive(Default)]
pub(crate) enum HelperStatus {
    #[default]
    Checking,
    Installed(PathBuf),
    Unavailable,
}

impl HelperStatus {
    /// Check for the installed file without following a replacement symlink.
    pub(crate) fn detect() -> Self {
        let helper = std::env::var_os("HOME").and_then(|home| installed_under(Path::new(&home)));
        helper.map_or(Self::Unavailable, Self::Installed)
    }
}

fn installed_under(home: &Path) -> Option<PathBuf> {
    if !home.is_absolute() {
        return None;
    }
    let path = home.join(HELPER_PATH.trim_end());
    std::fs::symlink_metadata(&path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|_| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_resolves_under_application_support() {
        let home = Path::new("/test home");
        assert_eq!(
            home.join(HELPER_PATH.trim_end()),
            Path::new("/test home/Library/Application Support/Sjel/helpers/sjel-fda-launcher"),
        );
        assert_eq!(installed_under(Path::new("")), None);
    }
}
