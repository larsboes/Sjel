//! Locate the sealed runtime relative to the app, without a developer shell or absolute paths.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn layout(executable: &Path) -> Result<(PathBuf, PathBuf), String> {
    let contents = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("invalid app layout")?;
    if contents.file_name().is_none_or(|name| name != "Contents") {
        return Err("launcher must run from an application bundle".into());
    }
    Ok((
        contents.join("Helpers/Sjel Desktop"),
        contents.join("Resources/runtime"),
    ))
}

fn launch() -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let (native, runtime) = layout(&executable)?;
    if !native.is_file() || !runtime.join("distribution.json").is_file() {
        return Err(
            "the application bundle is incomplete; replace it with a complete release".into(),
        );
    }
    let mut paths = vec![runtime.clone(), runtime.join("target/release")];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    } else {
        paths.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    }
    let path = std::env::join_paths(paths).map_err(|error| error.to_string())?;
    let error = Command::new(native)
        .args(std::env::args_os().skip(1))
        .current_dir(&runtime)
        .env("SJEL_ROOT", &runtime)
        .env("SJEL_CLI_BIN", runtime.join("target/release/sjel-cli"))
        .env("PATH", path)
        .exec();
    Err(format!("cannot launch Sjel: {error}"))
}

fn main() -> ExitCode {
    match launch() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Sjel: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_paths_survive_spaces_and_moves() {
        let (native, runtime) = layout(Path::new(
            "/Applications/My Apps/Sjel.app/Contents/MacOS/Sjel",
        ))
        .unwrap();
        assert_eq!(
            native,
            Path::new("/Applications/My Apps/Sjel.app/Contents/Helpers/Sjel Desktop")
        );
        assert_eq!(
            runtime,
            Path::new("/Applications/My Apps/Sjel.app/Contents/Resources/runtime")
        );
        assert!(layout(Path::new("/tmp/developer-binary")).is_err());
    }
}
