//! Open the `export.xml` inside whatever the operator hands over: the zip the Health app shares,
//! the unpacked `apple_health_export` folder, or the XML itself. A zip is streamed through the
//! system `unzip`, so a multi-gigabyte export is never unpacked to disk.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// The main export file. `export_cda.xml` beside it is the clinical document and is not read.
fn is_export_xml(name: &str) -> bool {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.eq_ignore_ascii_case("export.xml")
}

pub struct Opened {
    pub reader: Box<dyn BufRead>,
    child: Option<Child>,
}

impl Opened {
    /// Wait for `unzip`, if one runs, and report its failure.
    ///
    /// # Errors
    /// `unzip` exited with an error.
    pub fn finish(mut self) -> Result<(), String> {
        if let Some(child) = self.child.as_mut() {
            let status = child
                .wait()
                .map_err(|e| format!("unzip did not finish: {e}"))?;
            if !status.success() {
                return Err(format!("unzip exited with {status}"));
            }
        }
        Ok(())
    }
}

/// # Errors
/// The path is not an export, or it cannot be read.
pub fn open(path: &Path) -> Result<Opened, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "{} is a symlink; pass the file itself",
            path.display()
        ));
    }
    if meta.is_dir() {
        let xml =
            find_in_dir(path).ok_or_else(|| format!("no export.xml under {}", path.display()))?;
        return open(&xml);
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "xml" => {
            let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Opened {
                reader: Box::new(BufReader::with_capacity(1 << 20, file)),
                child: None,
            })
        }
        "zip" => open_zip(path),
        _ => Err(format!(
            "{}: expected the Health export .zip, its folder, or export.xml",
            path.display()
        )),
    }
}

fn find_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            let kind = entry.file_type().ok()?;
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_export_xml)
            {
                return Some(path);
            }
        }
    }
    None
}

fn open_zip(path: &Path) -> Result<Opened, String> {
    let listing = Command::new("unzip")
        .arg("-Z1")
        .arg(path)
        .output()
        .map_err(|e| format!("cannot run unzip: {e}"))?;
    if !listing.status.success() {
        return Err(format!("{}: not a readable zip", path.display()));
    }
    let names = String::from_utf8_lossy(&listing.stdout);
    let entry = names
        .lines()
        .find(|name| is_export_xml(name))
        .ok_or_else(|| format!("{}: holds no export.xml", path.display()))?
        .to_owned();
    let mut child = Command::new("unzip")
        .arg("-p")
        .arg(path)
        .arg(&entry)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run unzip: {e}"))?;
    let stdout = child.stdout.take().ok_or("unzip gave no output stream")?;
    Ok(Opened {
        reader: Box::new(BufReader::with_capacity(1 << 20, stdout)),
        child: Some(child),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_main_export_is_read() {
        assert!(is_export_xml("apple_health_export/export.xml"));
        assert!(is_export_xml("apple_health_export/Export.xml"));
        assert!(!is_export_xml("apple_health_export/export_cda.xml"));
    }

    #[test]
    fn a_zip_and_a_folder_open_the_same_stream() {
        let dir = std::env::temp_dir().join(format!("sjel-health-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("apple_health_export")).unwrap();
        std::fs::write(
            dir.join("apple_health_export/export.xml"),
            "<HealthData>\n</HealthData>\n",
        )
        .unwrap();
        std::fs::write(dir.join("apple_health_export/export_cda.xml"), "cda").unwrap();
        let mut lines = String::new();
        let mut opened = open(&dir).unwrap();
        std::io::Read::read_to_string(&mut opened.reader, &mut lines).unwrap();
        assert!(lines.starts_with("<HealthData>"));
        let zipped = Command::new("zip")
            .current_dir(&dir)
            .args(["-qr", "export.zip", "apple_health_export"])
            .status();
        if zipped.is_ok_and(|s| s.success()) {
            let opened = open(&dir.join("export.zip")).unwrap();
            let mut text = String::new();
            let mut reader = opened.reader;
            std::io::Read::read_to_string(&mut reader, &mut text).unwrap();
            assert!(text.starts_with("<HealthData>"));
        }
        assert!(open(&dir.join("notes.txt")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
