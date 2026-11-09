//! Inbox on the Mac: the triage page (`triage`). The phone's capture screen is NSH
//! (`crate::nsh_capture`).

pub mod capture;
pub mod triage;

pub use capture::capture_tag_pill;
