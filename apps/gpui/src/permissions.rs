//! The macOS privacy grants Sjel needs from the person, and a way to ask for each.
//!
//! The storage report measures folders macOS guards (`storage-policy.toml` in the overlay).
//! When the shell runs under this app, macOS asks on the app's behalf. Asking from here, with
//! the same read the report makes, puts the dialog in front of the person when they choose to
//! answer it, not in the middle of a scheduled run.

use std::path::PathBuf;
use std::process::Command;

pub struct Grant {
    pub title: &'static str,
    pub why: &'static str,
    /// Relative to the home folder: what the storage report reads.
    pub path: &'static str,
    /// The System Settings pane that holds this grant.
    pub pane: &'static str,
}

pub const GRANTS: [Grant; 2] = [
    Grant {
        title: "Photos",
        why: "The storage report measures the Photos library. Sjel does not read or copy photos.",
        path: "Pictures/Photos Library.photoslibrary",
        pane: "x-apple.systempreferences:com.apple.preference.security?Privacy_Photos",
    },
    Grant {
        title: "Media & Apple Music",
        why: "The storage report measures the Music folder. Sjel does not read the library.",
        path: "Music",
        pane: "x-apple.systempreferences:com.apple.preference.security?Privacy_Media",
    },
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Not asked in this session. Opening the section asks nothing.
    #[default]
    Unchecked,
    Checking,
    Granted,
    /// macOS refused the read: denied now, or denied earlier and not asking again.
    Denied,
    /// The folder does not exist on this Mac, so nothing needs a grant.
    Absent,
}

/// Reads the folder the way the storage report does. Blocks while macOS shows its dialog.
pub fn request(grant: &Grant) -> Status {
    let Some(home) = std::env::var_os("HOME") else {
        return Status::Denied;
    };
    let path = PathBuf::from(home).join(grant.path);
    if !path.exists() {
        return Status::Absent;
    }
    match Command::new("/usr/bin/du").arg("-sk").arg(&path).output() {
        Ok(out) if String::from_utf8_lossy(&out.stderr).contains("Operation not permitted") => {
            Status::Denied
        }
        Ok(_) => Status::Granted,
        Err(_) => Status::Denied,
    }
}

pub fn open_pane(grant: &Grant) {
    let _ = Command::new("/usr/bin/open").arg(grant.pane).spawn();
}
