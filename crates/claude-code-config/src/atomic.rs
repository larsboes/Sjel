// tools/claude-code-config/src/atomic.rs — a settings file a reader never sees half-written.
//
// Carried over from the TypeScript tool this replaced, because the reason did not change: a
// truncated ~/.claude/settings.json is not a weaker floor, it is an unparseable one, and
// Claude Code starts without the rules. So no bare write. Temp file in the same directory
// (rename is atomic only within one filesystem), mode set at open rather than chmod'd after
// the rename, content synced before the rename publishes it, and the temp file removed if
// anything fails.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The mode of an existing file, so a write preserves what the file already carries.
pub fn existing_mode(path: &Path) -> Option<u32> {
    fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode() & 0o777)
}

pub fn write_atomic(target: &Path, contents: &str, mode: u32) -> std::io::Result<()> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.json");
    let tmp = temp_path(dir, name);

    let write = || -> std::io::Result<()> {
        // "wx": never follow a symlink into, or reuse, an existing path at this name.
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, target)?;
        // Best effort: makes the rename itself survive a power loss. Not every platform lets
        // you open a directory for fsync, and failing here would undo a write that succeeded.
        if let Ok(handle) = File::open(dir) {
            let _ = handle.sync_all();
        }
        Ok(())
    };

    match write() {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            Err(error)
        }
    }
}

fn temp_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!(".{name}.sjel-{}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sjel-claude-config-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn writes_the_content_and_leaves_no_temp_file() {
        let dir = scratch();
        let target = dir.join("settings.json");
        write_atomic(&target, "{\"a\":1}\n", 0o600).expect("write");
        assert_eq!(fs::read_to_string(&target).expect("read"), "{\"a\":1}\n");
        assert_eq!(entries(&dir), vec!["settings.json".to_string()]);
    }

    #[test]
    fn a_new_file_gets_the_mode_it_was_asked_for() {
        let dir = scratch();
        let target = dir.join("settings.json");
        write_atomic(&target, "{}", 0o600).expect("write");
        assert_eq!(existing_mode(&target), Some(0o600));
    }

    #[test]
    fn replacing_keeps_a_mode_the_caller_preserved() {
        let dir = scratch();
        let target = dir.join("settings.json");
        write_atomic(&target, "{}", 0o644).expect("first write");
        let mode = existing_mode(&target).expect("mode");
        write_atomic(&target, "{\"b\":2}", mode).expect("second write");
        assert_eq!(fs::read_to_string(&target).expect("read"), "{\"b\":2}");
        assert_eq!(existing_mode(&target), Some(0o644));
    }

    #[test]
    fn existing_mode_is_none_for_a_missing_file() {
        let dir = scratch();
        assert_eq!(existing_mode(&dir.join("nope.json")), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
