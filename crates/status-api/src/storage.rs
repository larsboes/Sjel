//! Read-only directory discovery shared by the storage tool and Systems clients.

use serde::{Deserialize, Serialize};

pub const ROUTE_STATUS_STORAGE_EXPLORER: &str = "/api/sjel-status/storage-explorer";

/// One immediate child directory, charged by a single allocation walk.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageDirectory {
    pub path: String,
    pub bytes: u64,
}

/// An allocation snapshot, not an estimate of recoverable space.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageDirectorySnapshot {
    pub root: String,
    pub measured_at: u64,
    pub bytes: u64,
    pub directories: Vec<StorageDirectory>,
    pub unlisted_bytes: u64,
    pub omitted_directories: usize,
    pub partial: bool,
    pub error: Option<String>,
}

/// Cached on-demand discovery. Reading this state never starts a scan.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StorageExplorer {
    pub root: String,
    pub snapshot: Option<StorageDirectorySnapshot>,
    pub scanning: bool,
    pub error: Option<String>,
    pub previous_bytes: Option<u64>,
    pub previous_measured_at: Option<u64>,
}

impl StorageExplorer {
    /// A signed change between complete snapshots of the same directory.
    pub fn growth_bytes(&self) -> Option<i128> {
        let snapshot = self.snapshot.as_ref().filter(|snapshot| {
            !snapshot.partial && snapshot.error.is_none() && snapshot.root == self.root
        })?;
        Some(i128::from(snapshot.bytes) - i128::from(self.previous_bytes?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_gap_is_separate_from_directory_allocation() {
        let report = crate::StorageReport {
            disk: crate::Disk {
                total: 100 * 1024 * 1024,
                used: 60 * 1024 * 1024,
                free: 30 * 1024 * 1024,
            },
            ..crate::StorageReport::default()
        };
        assert_eq!(
            report.view().other_allocation_label.as_deref(),
            Some("10 MB")
        );
        let balanced = crate::StorageReport {
            disk: crate::Disk {
                free: 40 * 1024 * 1024,
                ..report.disk
            },
            ..crate::StorageReport::default()
        };
        assert!(balanced.view().other_allocation_label.is_none());
    }

    #[test]
    fn growth_requires_two_complete_readings() {
        let mut view = StorageExplorer {
            root: "/data".into(),
            previous_bytes: Some(100),
            snapshot: Some(StorageDirectorySnapshot {
                root: "/data".into(),
                measured_at: 20,
                bytes: 60,
                directories: vec![],
                unlisted_bytes: 60,
                omitted_directories: 0,
                partial: false,
                error: None,
            }),
            ..StorageExplorer::default()
        };
        assert_eq!(view.growth_bytes(), Some(-40));
        view.snapshot.as_mut().unwrap().partial = true;
        assert_eq!(view.growth_bytes(), None);
        view.snapshot.as_mut().unwrap().partial = false;
        view.root = "/other".into();
        assert_eq!(view.growth_bytes(), None);
        view.root = "/data".into();
        view.snapshot.as_mut().unwrap().error = Some("incomplete reading".into());
        assert_eq!(view.growth_bytes(), None);
        view.snapshot.as_mut().unwrap().error = None;
        view.previous_bytes = None;
        assert_eq!(view.growth_bytes(), None);
    }
}
