//! `sjel app` — manage native Sjel macOS applications (Desktop GPUI & Menu Bar).

use std::path::Path;
use std::process::{Command, ExitCode};

pub const USAGE: &str = r#"Usage: sjel app [command] [target]

Manage native macOS application bundles (build, codesign, install, status, open).

Commands:
  update [desktop|mac|all]       Build, sign and install the app bundle (default: desktop)
  install [desktop|mac|all]      Alias for update
  status                         Check installation status and bundle paths
  open [desktop|mac]             Launch or bring the app forward (default: desktop)

Targets:
  desktop                        Sjel Desktop.app (GPUI Zed framework, dev.sjel.desktop)
  mac                            Sjel.app (Menu bar app, dev.sjel.mac)
  all                            Both desktop and menu bar apps
"#;

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let cmd = args.first().map(String::as_str).unwrap_or("status");
    let target = args.get(1).map(String::as_str).unwrap_or("desktop");

    match cmd {
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        "update" | "install" => update(root, target),
        "status" => status(root),
        "open" => open(target),
        other => {
            eprintln!("sjel app: unknown command '{other}'");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn update(root: &Path, target: &str) -> ExitCode {
    let mut ok = true;
    if target == "desktop" || target == "all" {
        println!("Building and installing Sjel Desktop.app (apps/gpui/install)...");
        let status = Command::new(root.join("apps/gpui/install"))
            .current_dir(root)
            .status();
        match status {
            Ok(s) if s.success() => {
                println!("✓ Sjel Desktop.app installed to ~/Applications/Sjel Desktop.app");
            }
            Ok(s) => {
                eprintln!(
                    "✗ apps/gpui/install failed with exit code {}",
                    s.code().unwrap_or(1)
                );
                ok = false;
            }
            Err(e) => {
                eprintln!("✗ cannot run apps/gpui/install: {e}");
                ok = false;
            }
        }
    }
    if target == "mac" || target == "all" {
        println!("Building and installing Sjel Menu Bar app (apps/mac/install)...");
        let status = Command::new(root.join("apps/mac/install"))
            .current_dir(root)
            .status();
        match status {
            Ok(s) if s.success() => {
                println!("✓ Sjel.app installed to ~/Applications/Sjel.app");
            }
            Ok(s) => {
                eprintln!(
                    "✗ apps/mac/install failed with exit code {}",
                    s.code().unwrap_or(1)
                );
                ok = false;
            }
            Err(e) => {
                eprintln!("✗ cannot run apps/mac/install: {e}");
                ok = false;
            }
        }
    }
    if !ok {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn open(target: &str) -> ExitCode {
    let bundle_id = match target {
        "desktop" => "dev.sjel.desktop",
        "mac" => "dev.sjel.mac",
        other => {
            eprintln!("sjel app open: unknown target '{other}' (expected 'desktop' or 'mac')");
            return ExitCode::from(2);
        }
    };
    match Command::new("open").arg("-b").arg(bundle_id).status() {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(s) => ExitCode::from(s.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("cannot launch {bundle_id}: {e}");
            ExitCode::FAILURE
        }
    }
}

fn status(root: &Path) -> ExitCode {
    let home = std::env::var("HOME").unwrap_or_default();
    let home_path = Path::new(&home);

    let desktop_app = home_path.join("Applications/Sjel Desktop.app");
    let mac_app = home_path.join("Applications/Sjel.app");

    println!("Sjel Native Applications:");
    print_app_status(
        "Desktop App",
        &desktop_app,
        "dev.sjel.desktop",
        &root.join("apps/gpui"),
    );
    print_app_status(
        "Menu Bar App",
        &mac_app,
        "dev.sjel.mac",
        &root.join("apps/mac"),
    );

    ExitCode::SUCCESS
}

fn print_app_status(name: &str, app_path: &Path, bundle_id: &str, src_dir: &Path) {
    if !app_path.exists() {
        println!("  • {name} ({bundle_id}): not installed (run 'sjel app install')");
        return;
    }
    let mtime = app_path.metadata().and_then(|m| m.modified()).ok();
    let mtime_str = mtime
        .map(format_system_time)
        .unwrap_or_else(|| "unknown".into());

    println!("  • {name} ({bundle_id}):");
    println!("      Installed at: {}", app_path.display());
    println!("      Last updated: {mtime_str}");
    if src_dir.exists() {
        println!("      Source dir:   {}", src_dir.display());
    }
}

fn format_system_time(t: std::time::SystemTime) -> String {
    let d = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, day) = civil_date::unix_day_to_ymd(days);
    format!(
        "{y:04}-{m:02}-{day:02} {:02}:{:02}:{:02} UTC",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}
