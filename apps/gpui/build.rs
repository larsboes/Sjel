use std::path::{Path, PathBuf};
use std::process::Command;

fn output(command: &mut Command) -> Option<String> {
    let result = command.output().ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8(result.stdout).ok())
        .flatten()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn build_stamp(manifest: &Path) {
    println!("cargo:rerun-if-env-changed=SJEL_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=SJEL_BUILD_TIME");

    let commit = std::env::var("SJEL_BUILD_COMMIT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            output(
                Command::new("git")
                    .current_dir(manifest)
                    .args(["rev-parse", "--short", "HEAD"]),
            )
        })
        .unwrap_or_else(|| "dev".into());
    let time = std::env::var("SJEL_BUILD_TIME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| output(Command::new("date").args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])))
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=SJEL_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=SJEL_BUILD_TIME={time}");
}

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    build_stamp(&manifest);
}
