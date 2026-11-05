use std::path::Path;
use std::process::Command;

use super::diagnostics::read_cargo_output;
use super::git::find_repo_root;
use super::types::BuildFailure;

/// Executes the install script.
///
/// The build runs with `--message-format=json-render-diagnostics`, so a failure comes back as a
/// record per error instead of one log: the panel can name the first one without a human reading
/// 200 lines. The rendering stays inside each record, so the log is still readable.
pub fn execute_rebuild(repo_root: &Path) -> Result<String, BuildFailure> {
    let install_script = repo_root.join("apps/gpui/install");
    if !install_script.exists() {
        return Err(BuildFailure::without_diagnostics(format!(
            "Installer script not found at {}",
            install_script.display()
        )));
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let path_env = format!(
        "{}/.local/bin:{}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        home, home
    );

    let output = Command::new("/bin/bash")
        .arg(&install_script)
        .current_dir(repo_root)
        .env("PATH", path_env)
        .env("SJEL_BUILD_MESSAGE_FORMAT", "json-render-diagnostics")
        .output()
        .map_err(|e| {
            BuildFailure::without_diagnostics(format!("Failed to spawn installer: {e}"))
        })?;

    if output.status.success() {
        return Ok("Sjel Desktop rebuilt and installed successfully!".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    // Cargo writes build records to stdout; anything else the script says goes to stderr.
    let read = read_cargo_output(&stdout);
    let first = read.errors.into_iter().next();
    let log = match (read.log.trim().is_empty(), stderr.trim().is_empty()) {
        (true, true) => "The installer failed without output.".to_owned(),
        (true, false) => stderr,
        (false, true) => read.log,
        (false, false) => format!("{}\n{stderr}", read.log),
    };
    Err(BuildFailure { first, log })
}

/// Starts the app on the other node. From the dev node it opens the installed app; from the
/// live node it runs `tools/demo-shots app`, which fails when the demo node is down.
pub fn launch_other_node() -> Result<(), String> {
    if sjel_gpui_core::environment::current().dev {
        let status = Command::new("/usr/bin/open")
            .args(["-b", "dev.sjel.desktop"])
            .status()
            .map_err(|e| format!("Could not open Sjel Desktop: {e}"))?;
        return if status.success() {
            Ok(())
        } else {
            Err("Sjel Desktop is not installed. Run apps/gpui/install.".to_owned())
        };
    }
    let root = find_repo_root().ok_or("Could not locate the Sjel checkout.")?;
    let home = std::env::var("HOME").unwrap_or_default();
    let output = Command::new(root.join("tools/demo-shots"))
        .arg("app")
        .current_dir(&root)
        .env(
            "PATH",
            format!("{home}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"),
        )
        .output()
        .map_err(|e| format!("Could not start the dev app: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(stderr
            .lines()
            .last()
            .unwrap_or("The dev app did not start.")
            .to_owned())
    }
}

/// Rebuilds and restarts the local shell outside its own authenticated API.
pub fn restart_local_shell(repo_root: &Path) -> Result<(), String> {
    let runner = repo_root.join("tools/service-runner.sh");
    if !runner.is_file() {
        return Err(format!("Service runner not found at {}", runner.display()));
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let path_env = format!(
        "{}/.local/bin:{}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        home, home
    );
    let output = Command::new(runner)
        .args(["restart", "sjel-status"])
        .current_dir(repo_root)
        .env("PATH", path_env)
        .output()
        .map_err(|error| format!("Could not start the Sjel service runner: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if detail.is_empty() {
        "The Sjel service runner could not restart the local shell.".into()
    } else {
        detail
    })
}
