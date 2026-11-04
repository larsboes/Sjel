use sjel_containment::{
    classify_ring, provider_hosts, EgressPolicy, EgressProxy, Ring, SeatbeltProfile,
};
use sjel_inference::ReviewedProvidersList;
use std::path::PathBuf;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!(
            "Usage: sjel-contain <harness-bin> --provider <name> --project <dir> [-- <harness-args...>]"
        );
        return ExitCode::from(2);
    }

    let mut harness_bin = String::new();
    let mut provider = String::new();
    let mut project = String::new();
    let mut harness_args = Vec::new();

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

    let profile = SeatbeltProfile::new(&project_dir);
    let contained = sjel_containment::ContainedCommand::new(&harness_bin, profile)
        .args(&harness_args)
        .with_proxy_port(proxy_port);

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
