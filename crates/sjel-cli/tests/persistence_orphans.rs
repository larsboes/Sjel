use std::{fs, os::unix::fs::symlink, path::Path, process::Command};

#[test]
fn doctor_distinguishes_enabled_orphaned_unmanaged_and_script_owned_units() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    for dir in ["tools", "crates", "services"] {
        symlink(checkout.join(dir), root.join(dir)).unwrap();
    }
    let home = root.join("home");
    let units = home.join("Library/LaunchAgents");
    let overlay = root.join("overlay");
    fs::create_dir_all(&units).unwrap();
    fs::create_dir_all(overlay.join("config")).unwrap();
    fs::write(
        overlay.join("config/machine.toml"),
        "os = 'macos'\ncontainer_runtime = 'docker'\ncapabilities = ['comms', 'macmon']\n",
    )
    .unwrap();
    for label in [
        "com.axon.comms",
        "com.axon.macmon",
        "com.axon.trips",
        "com.axon.nothing-declares-this",
        "com.sjel.tailscaled",
    ] {
        fs::write(units.join(format!("{label}.plist")), "<plist/>\n").unwrap();
    }

    let output = Command::new(root.join("tools/doctor"))
        .arg("--persistence")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", home)
        .env("SJEL_OVERLAY_ROOT", overlay)
        .env("SJEL_CLI_BIN", env!("CARGO_BIN_EXE_sjel-cli"))
        .current_dir(root)
        .output()
        .unwrap();
    let report = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        output.status.code(),
        Some(i32::from(report.contains("  ✗ "))),
        "{report}"
    );
    assert!(report.contains("Boot persistence"), "{report}");
    assert!(!report.contains("Host toolchain"), "{report}");
    assert!(!report.contains("persistence state unverified"), "{report}");
    for enabled in ["comms", "macmon"] {
        assert!(
            !report.contains(&format!("persistence is installed for '{enabled}'")),
            "{report}"
        );
    }
    assert!(
        report.contains("persistence is installed for 'trips'"),
        "{report}"
    );
    assert!(
        report.contains("'nothing-declares-this' is unmanaged:"),
        "{report}"
    );
    assert!(
        !report.contains("remove-persistence nothing-declares-this"),
        "{report}"
    );
    assert!(
        report.contains("'tailscaled' is installed by tools/setup-tailnet-shell.sh"),
        "{report}"
    );
    assert!(!report.contains("'tailscaled' is unmanaged"), "{report}");
    assert!(!include_str!("../src/doctor/checks.rs").contains("\"macmon\""));
}
