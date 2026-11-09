//! Cargo-backed entry points for capability CLIs.

use std::{
    path::Path,
    process::{Command, ExitCode},
};

use crate::paths::{duplicate_message, Manifest, Paths};

fn binary(manifest: &str, capability: &str) -> Result<String, String> {
    let table: toml::Table = toml::from_str(manifest).map_err(|e| e.to_string())?;
    let bins = table.get("bin").and_then(toml::Value::as_array);
    for candidate in [
        capability.to_owned(),
        format!("{capability}-cli"),
        format!("sjel-{capability}"),
    ] {
        if bins.is_some_and(|bins| {
            bins.iter().any(|bin| {
                bin.get("name").and_then(toml::Value::as_str) == Some(candidate.as_str())
            })
        }) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "'{capability}' declares no '{capability}', '{capability}-cli' or 'sjel-{capability}' CLI binary"
    ))
}

fn cargo_command(paths: &Paths, manifest: &Path, bin: &str, args: &[String]) -> Command {
    let mut command = Command::new("cargo");
    command
        .args(["run", "--locked", "--manifest-path"])
        .arg(manifest)
        .args(["--bin", bin, "--"])
        .args(args);
    if let Some(overlay) = &paths.overlay_root {
        command
            .env("SJEL_OVERLAY_ROOT", overlay)
            .env("SJEL_PERSONAL_ROOT", overlay);
    }
    if let Some(machine) = &paths.machine_toml {
        command.env("SJEL_MACHINE_TOML", machine);
    }
    command
}

/// Resolve a capability's declared CLI, build it through Cargo, and forward its arguments.
/// Resolution and build errors return a nonzero exit status without running the CLI.
pub fn run(root: &Path, capability: &str, args: &[String]) -> ExitCode {
    if capability.is_empty()
        || !capability
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return crate::fail(&format!(
            "sjel: unknown command '{capability}'; run 'sjel help'"
        ));
    }
    let resolve = || -> Result<_, String> {
        let paths = Paths::from_shell(root)?;
        let service = match paths.manifest(capability) {
            Manifest::Found(path) => path,
            Manifest::Duplicate(core, overlay) => {
                return Err(duplicate_message(capability, &core, &overlay));
            }
            Manifest::None => {
                return Err(format!("unknown command '{capability}'; run 'sjel help'"));
            }
        };
        let manifest = service.with_file_name("Cargo.toml");
        let body = std::fs::read_to_string(&manifest)
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let bin = binary(&body, capability)?;
        Ok(cargo_command(&paths, &manifest, &bin, args))
    };
    let mut command = match resolve() {
        Ok(command) => command,
        Err(error) => return crate::fail(&format!("sjel: {error}")),
    };
    if std::env::var("SJEL_ALLOW_LOCAL_BUILD").as_deref() != Ok("1") {
        let admitted = crate::runtime::run(root, &["admit".into(), "local-build".into()]);
        if admitted != ExitCode::SUCCESS {
            return admitted;
        }
    }
    crate::exec(&mut command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_only_the_capability_cli_and_never_its_server_or_helpers() {
        assert_eq!(
            binary(include_str!("../../projects/Cargo.toml"), "projects").unwrap(),
            "projects"
        );
        assert_eq!(
            binary(
                include_str!("../../operator-profile/Cargo.toml"),
                "operator-profile"
            )
            .unwrap(),
            "operator-profile"
        );
        assert_eq!(
            binary(include_str!("../../finance/Cargo.toml"), "finance").unwrap(),
            "finance-cli"
        );
        assert_eq!(
            binary(include_str!("../../comms/Cargo.toml"), "comms").unwrap(),
            "comms"
        );
        assert_eq!(
            binary(include_str!("../../import/Cargo.toml"), "import").unwrap(),
            "sjel-import"
        );
        assert!(binary("[[bin]]\nname = 'example-server'", "example").is_err());
        assert!(binary("not toml", "example").is_err());
    }

    #[test]
    fn cargo_receives_literal_arguments_and_the_resolved_deployment() {
        let paths = Paths {
            root: "/checkout".into(),
            overlay_root: Some("/deployment".into()),
            machine_toml: Some("/deployment/config/machine.toml".into()),
            caps_dir: "/checkout/capabilities".into(),
            overlay_caps_dir: None,
        };
        let command = cargo_command(
            &paths,
            Path::new("/checkout/crates/projects/Cargo.toml"),
            "projects",
            &[
                "new-note".into(),
                "A title; $(literal)".into(),
                "--summary".into(),
                "two\nlines".into(),
            ],
        );
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "run",
                "--locked",
                "--manifest-path",
                "/checkout/crates/projects/Cargo.toml",
                "--bin",
                "projects",
                "--",
                "new-note",
                "A title; $(literal)",
                "--summary",
                "two\nlines"
            ]
        );
        assert_eq!(command.get_current_dir(), None);
        let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(
            env[std::ffi::OsStr::new("SJEL_PERSONAL_ROOT")],
            Some(std::ffi::OsStr::new("/deployment"))
        );
        assert_eq!(
            env[std::ffi::OsStr::new("SJEL_MACHINE_TOML")],
            Some(std::ffi::OsStr::new("/deployment/config/machine.toml"))
        );
    }
}
