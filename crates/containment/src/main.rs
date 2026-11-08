use sjel_containment::{
    classify_ring, provider_hosts, EgressPolicy, EgressProxy, Ring, SeatbeltProfile,
};
use sjel_inference::ReviewedProvidersList;
use std::path::PathBuf;
use std::process::ExitCode;

/// The seeded demo node, the only Sjel a contained run reaches. The port is tools/demo-shots'.
fn demo_endpoint() -> String {
    let port = std::env::var("SJEL_DEMO_SHOTS_PORT").unwrap_or_else(|_| "18082".into());
    format!("http://127.0.0.1:{port}")
}

/// The demo node's throwaway inbound token, written by tools/demo-up. Synthetic data only.
fn demo_token() -> Option<String> {
    let root = PathBuf::from(std::env::var_os("SJEL_ROOT")?);
    let token = std::fs::read_to_string(root.join("demo/state/comms-api-secret")).ok()?;
    Some(token.trim().to_owned()).filter(|t| !t.is_empty())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let own = args.iter().take_while(|a| *a != "--");
    if args.is_empty() || own.into_iter().any(|a| a == "--help" || a == "-h") {
        eprintln!(
            "Usage: sjel-contain <harness-bin> --provider <name> --project <dir> [--read <dir>]... [--write <dir>]... [--keep-env <NAME>]... [--env <NAME=VALUE>]... [--credential <provider>]... [-- <harness-args...>]"
        );
        return ExitCode::from(2);
    }

    let mut harness_bin = String::new();
    let mut provider = String::new();
    let mut project = String::new();
    let mut harness_args = Vec::new();
    let mut read_dirs: Vec<PathBuf> = Vec::new();
    let mut write_dirs: Vec<PathBuf> = Vec::new();
    let mut keep_env: Vec<String> = Vec::new();
    let mut set_env: Vec<String> = Vec::new();
    let mut credentials: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        if args[i] == "--" {
            harness_args.extend(args[i + 1..].iter().cloned());
            break;
        } else if args[i] == "--provider" && i + 1 < args.len() {
            provider = args[i + 1].clone();
            i += 2;
        } else if args[i] == "--project" && i + 1 < args.len() {
            project = args[i + 1].clone();
            i += 2;
        } else if args[i] == "--read" && i + 1 < args.len() {
            read_dirs.push(PathBuf::from(&args[i + 1]));
            i += 2;
        } else if args[i] == "--write" && i + 1 < args.len() {
            write_dirs.push(PathBuf::from(&args[i + 1]));
            i += 2;
        } else if args[i] == "--keep-env" && i + 1 < args.len() {
            keep_env.push(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--env" && i + 1 < args.len() {
            set_env.push(args[i + 1].clone());
            i += 2;
        } else if args[i] == "--credential" && i + 1 < args.len() {
            credentials.push(args[i + 1].clone());
            i += 2;
        } else if harness_bin.is_empty() {
            harness_bin = args[i].clone();
            i += 1;
        } else {
            harness_args.push(args[i].clone());
            i += 1;
        }
    }

    if harness_bin.is_empty() || project.is_empty() {
        eprintln!("Error: missing harness binary or --project directory");
        return ExitCode::from(2);
    }

    let project_dir = PathBuf::from(&project);
    if !project_dir.exists() {
        eprintln!(
            "Error: project directory does not exist: {}",
            project_dir.display()
        );
        return ExitCode::from(2);
    }

    // `--env` names a value the caller supplies, never one inherited from the environment.
    let mut env_pairs: Vec<(String, String)> = Vec::new();
    for assignment in &set_env {
        match assignment.split_once('=') {
            Some((name, value)) if !name.is_empty() && !name.contains(char::is_whitespace) => {
                env_pairs.push((name.to_owned(), value.to_owned()));
            }
            _ => {
                eprintln!("Error: --env takes NAME=VALUE, not {assignment:?}");
                return ExitCode::from(2);
            }
        }
    }

    let providers = ReviewedProvidersList::load();
    let ring = classify_ring(&providers, &provider, &civil_date::today());
    let hosts = provider_hosts(&providers, &provider);

    println!("[containment] harness={harness_bin} provider={provider} ring={ring:?}");

    if ring == Ring::Trusted {
        // Run harness directly
        let mut cmd = std::process::Command::new(&harness_bin);
        cmd.args(&harness_args);
        cmd.current_dir(&project_dir);
        let status = cmd.status();
        return match status {
            Ok(s) => ExitCode::from(s.code().unwrap_or(0) as u8),
            Err(e) => {
                eprintln!("Failed to execute harness: {e}");
                ExitCode::from(1)
            }
        };
    }

    // Contained run: resolve the named provider keys first, so a missing one stops the run.
    let mut resolved: Vec<(String, String)> = Vec::new();
    for slug in &credentials {
        match sjel_containment::resolve_credential(slug) {
            Ok(pair) => resolved.push(pair),
            Err(message) => {
                eprintln!("[containment] --credential {slug}: {message}");
                return ExitCode::from(2);
            }
        }
    }

    // Contained run: start egress proxy
    if hosts.is_empty() {
        eprintln!(
            "[containment] {provider} lists no hosts in providers.toml: its API is unreachable"
        );
    }
    let policy = hosts
        .into_iter()
        .fold(EgressPolicy::new(), EgressPolicy::with_provider_host);
    let proxy = match EgressProxy::bind(policy).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to bind egress proxy: {e}");
            return ExitCode::from(1);
        }
    };
    let proxy_port = proxy.local_addr().port();
    tokio::spawn(async move {
        let _ = proxy.run().await;
    });

    let profile = read_dirs.iter().fold(
        write_dirs
            .iter()
            .fold(SeatbeltProfile::new(&project_dir), |p, d| {
                p.with_read_write_path(d)
            }),
        |p, d| p.with_read_path(d),
    );
    let mut contained = sjel_containment::ContainedCommand::new(&harness_bin, profile)
        .args(&harness_args)
        .with_proxy_port(proxy_port)
        .with_dev_node(&demo_endpoint());
    // The provider's own credential, named on the command line; every other one stays out.
    for name in &keep_env {
        if let Ok(value) = std::env::var(name) {
            contained.env.insert(name.clone(), value);
        }
    }
    // A value the caller named on the command line, for a harness that reads its own home.
    for (name, value) in env_pairs {
        contained.env.insert(name, value);
    }
    // A key from the private catalogue, resolved here and handed to the child as one variable.
    for (name, value) in resolved {
        println!("[containment] credential {name} resolved from the catalogue");
        contained.env.insert(name, value);
    }
    match demo_token() {
        Some(token) => {
            contained.env.insert("SJEL_DEMO_TOKEN".into(), token);
        }
        None => eprintln!("[containment] no demo node token: run `sjel demo up` for dev data"),
    }

    let mut cmd = contained.build_command();
    cmd.current_dir(&project_dir);

    match cmd.status() {
        Ok(s) => ExitCode::from(s.code().unwrap_or(0) as u8),
        Err(e) => {
            eprintln!("Failed to spawn contained command under sandbox-exec: {e}");
            ExitCode::from(1)
        }
    }
}
