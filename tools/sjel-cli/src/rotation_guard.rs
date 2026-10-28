use std::fs::{File, OpenOptions};
use std::path::PathBuf;

use crate::paths::Paths;

/// Held by both sqld starters and rotation writers. Never unlink the lock file.
pub(crate) struct Guard {
    _file: File,
    marker: PathBuf,
}

pub(crate) fn marker_path(paths: &Paths) -> Result<PathBuf, String> {
    let overlay = paths
        .overlay_root
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or("sqld rotation: overlay root is not configured")?;
    Ok(overlay.join("config/sqld-rotation-inhibit"))
}

pub(crate) fn acquire(paths: &Paths) -> Result<Guard, String> {
    let marker = marker_path(paths)?;
    let lock = marker.with_file_name("sqld-rotation.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .map_err(|e| format!("sqld rotation: cannot open lock {}: {e}", lock.display()))?;
    file.try_lock()
        .map_err(|e| format!("sqld rotation: cannot acquire lock {}: {e}", lock.display()))?;
    Ok(Guard {
        _file: file,
        marker,
    })
}

impl Guard {
    pub(crate) fn marker(&self) -> &std::path::Path {
        &self.marker
    }

    pub(crate) fn marker_is_present(&self) -> Result<bool, String> {
        match std::fs::symlink_metadata(&self.marker) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let dir = self.marker.parent().expect("marker has a parent");
                let metadata = std::fs::metadata(dir)
                    .map_err(|e| format!("sqld rotation: cannot access {}: {e}", dir.display()))?;
                if !metadata.is_dir() {
                    return Err(format!(
                        "sqld rotation: {} is not a directory",
                        dir.display()
                    ));
                }
                Ok(false)
            }
            Err(e) => Err(format!(
                "sqld rotation: cannot check {}: {e}",
                self.marker.display()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_lock_survives_on_disk_and_releases_on_drop() {
        let root = std::env::temp_dir().join(format!("sjel-rotation-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("config")).unwrap();
        let paths = Paths {
            root: root.clone(),
            caps_dir: root.join("capabilities"),
            overlay_root: Some(root.clone()),
            machine_toml: None,
            overlay_caps_dir: None,
        };
        let guard = acquire(&paths).unwrap();
        assert!(!guard.marker_is_present().unwrap());
        assert!(acquire(&paths).is_err());
        drop(guard);
        assert!(root.join("config/sqld-rotation.lock").is_file());
        let guard = acquire(&paths).unwrap();
        std::fs::rename(root.join("config"), root.join("moved-config")).unwrap();
        assert!(guard.marker_is_present().is_err());
        drop(guard);
        assert!(acquire(&paths).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
