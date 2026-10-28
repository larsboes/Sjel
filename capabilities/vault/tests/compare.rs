use std::fs;
use std::process::Command;

#[test]
fn compares_only_unchanged_notes_and_counts_newly_broken_occurrences() {
    let root = std::env::temp_dir().join(format!("sjel-vault-compare-{}", std::process::id()));
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("Hub.md"), "[[Gone]] [[Gone]] [[Already dead]]").unwrap();
    fs::write(
        destination.join("Hub.md"),
        "[[Gone]] [[Gone]] [[Already dead]]",
    )
    .unwrap();
    fs::write(source.join("Gone.md"), "present").unwrap();
    fs::write(source.join("Changed.md"), "[[Gone]]").unwrap();
    fs::write(destination.join("Changed.md"), "[[Gone]] edited").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_vault"))
        .args(["compare", "--root"])
        .arg(&source)
        .arg("--against")
        .arg(&destination)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["unchanged_notes"], 1);
    assert_eq!(result["destination_notes_excluded"], 1);
    assert_eq!(result["newly_unresolved_occurrences"], 2);
    assert!(result.get("newly_unresolved").is_none());
    let listed = Command::new(env!("CARGO_BIN_EXE_vault"))
        .args(["compare", "--root"])
        .arg(&source)
        .arg("--against")
        .arg(&destination)
        .args(["--json", "--list"])
        .output()
        .unwrap();
    assert!(listed.status.success());
    let detail: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(detail["newly_unresolved"].as_array().unwrap().len(), 2);
    assert_eq!(detail["newly_unresolved"][0]["target"], "Gone");
    fs::remove_dir_all(root).unwrap();
}
