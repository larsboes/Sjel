use sjel_containment::{
    classify_ring, provider_hosts, EgressPolicy, EgressProxy, Ring, SeatbeltProfile, TrustedProfile,
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

/// The one harness that may run without the trusted profile, for debugging
/// (`docs/plans/containment.md` §1).
const UNWALLED_HARNESS: &str = "pi";

/// The overlay root, from the environment `tools/lib/paths.sh` exports.
fn overlay_root() -> Option<PathBuf> {
    std::env::var_os("SJEL_OVERLAY_ROOT")
        .or_else(|| std::env::var_os("SJEL_PERSONAL_ROOT"))
        .map(PathBuf::from)
}

/// The current UTC time as `YYYY-MM-DDTHH:MM:SSZ`.
fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (day, time) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        civil_date::iso_of_unix_day(day),
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

/// Appends one line per unwalled run to [`sjel_config::UNWALLED_RUNS_LOG`]: UTC time, harness,
/// provider and project, tab-separated.
fn record_unwalled(
    overlay: &std::path::Path,
    harness: &str,
    provider: &str,
    project: &std::path::Path,
) -> std::io::Result<()> {
    use std::io::Write;
    let path = overlay.join(sjel_config::UNWALLED_RUNS_LOG);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(
        log,
        "{}\t{harness}\t{provider}\t{}",
        now_utc(),
        project.display()
    )
}

/// A trusted-ring run: the harness keeps the network and the filesystem, except the overlay's
/// `data/` and `secrets/` and the home credentials no harness signs in with. `walled: false`
/// skips that profile for debugging. `SJEL_AGENT=1` makes Sjel's own tools inside the run
/// present the agent token, which the gate pseudonymizes, walled or not.
fn run_trusted(
    harness_bin: &str,
    harness_args: &[String],
    project_dir: &std::path::Path,
    env_pairs: Vec<(String, String)>,
    walled: bool,
) -> ExitCode {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let overlay = overlay_root();
    let mut cmd = if walled {
        if overlay.is_none() {
            eprintln!("[containment] no overlay is set: only the home credentials are closed");
        }
        let profile = TrustedProfile::new(home.as_deref(), overlay.as_deref());
        let mut cmd = std::process::Command::new("/usr/bin/sandbox-exec");
        cmd.arg("-p").arg(profile.render()).arg(harness_bin);
        cmd
    } else {
        eprintln!("[containment] UNWALLED: this run can read the overlay's data/ and secrets/");
        std::process::Command::new(harness_bin)
    };
    cmd.args(harness_args);
    cmd.current_dir(project_dir).env("SJEL_AGENT", "1");
    // The token the run's Sjel tools present: `sjel-agent-token.<harness>`, else the default.
    if std::env::var_os("SJEL_AGENT_NAME").is_none() {
        if let Some(name) = std::path::Path::new(harness_bin)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        {
            cmd.env("SJEL_AGENT_NAME", name);
        }
    }
    for (name, value) in env_pairs {
        cmd.env(name, value);
    }
    match cmd.status() {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("Failed to start the harness: {e}");
            ExitCode::from(1)
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let own = args.iter().take_while(|a| *a != "--");
    if args.is_empty() || own.into_iter().any(|a| a == "--help" || a == "-h") {
        eprintln!(
            "Usage: sjel-contain <harness-bin> --provider <name> --project <dir> [--unwalled] [--read <dir>]... [--write <dir>]... [--keep-env <NAME>]... [--env <NAME=VALUE>]... [--credential <provider>|<entry>=<VAR>]... [-- <harness-args...>]\n       sjel-contain --ring-of <provider>"
        );
        return ExitCode::from(2);
    }

    let providers = ReviewedProvidersList::load();
    if let [flag, provider] = args.as_slice() {
        if flag == "--ring-of" {
            let ring = classify_ring(&providers, provider, &civil_date::today());
            println!(
                "{}",
                if ring == Ring::Trusted {
                    "trusted"
                } else {
                    "contained"
                }
            );
            return ExitCode::SUCCESS;
        }
    }

    let mut harness_bin = String::new();
    let mut provider = String::new();
    let mut unwalled = false;
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
        } else if args[i] == "--unwalled" {
            unwalled = true;
            i += 1;
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

    let ring = classify_ring(&providers, &provider, &civil_date::today());
    let hosts = provider_hosts(&providers, &provider);

    println!("[containment] harness={harness_bin} provider={provider} ring={ring:?}");

    if unwalled {
        let harness_name = std::path::Path::new(&harness_bin)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if harness_name != UNWALLED_HARNESS {
            eprintln!(
                "[containment] --unwalled is for {UNWALLED_HARNESS} only; {harness_name} always runs walled"
            );
            return ExitCode::from(2);
        }
        if ring != Ring::Trusted {
            eprintln!(
                "[containment] --unwalled is for trusted providers only; {provider} always runs contained"
            );
            return ExitCode::from(2);
        }
        let Some(overlay) = overlay_root() else {
            eprintln!("[containment] --unwalled needs the overlay set, to record the run");
            return ExitCode::from(2);
        };
        if let Err(e) = record_unwalled(&overlay, &harness_bin, &provider, &project_dir) {
            eprintln!("[containment] --unwalled: the run could not be recorded: {e}");
            return ExitCode::from(2);
        }
    }

    if ring == Ring::Trusted {
        return run_trusted(
            &harness_bin,
            &harness_args,
            &project_dir,
            env_pairs,
            !unwalled,
        );
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

    let mut profile = read_dirs.iter().fold(
        write_dirs
            .iter()
            .fold(SeatbeltProfile::new(&project_dir), |p, d| {
                p.with_read_write_path(d)
            }),
        |p, d| p.with_read_path(d),
    );
    // The machine's TLS roots, read here because a runtime inside the profile finds none in the
    // Keychain's trust settings; the run may read the bundle, and it goes when the child exits.
    let mut roots_path: Option<PathBuf> = None;
    if let Some(bundle) = sjel_containment::export_roots() {
        let dir = std::env::temp_dir();
        if let Some(path) = sjel_containment::write_bundle(&bundle, &dir) {
            profile = profile.with_read_path(&path);
            roots_path = Some(path);
        }
    }

    let mut contained = sjel_containment::ContainedCommand::new(&harness_bin, profile)
        .args(&harness_args)
        .with_proxy_port(proxy_port)
        .with_dev_node(&demo_endpoint());
    if let Some(path) = &roots_path {
        contained = contained.with_ca_bundle(path);
    }
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
    // The machine's TLS roots, read here because the run cannot read the Keychain's trust
    // settings for itself; removed when the child exits.
    match demo_token() {
        Some(token) => {
            contained.env.insert("SJEL_DEMO_TOKEN".into(), token);
        }
        None => eprintln!("[containment] no demo node token: run `sjel demo up` for dev data"),
    }

    let mut cmd = contained.build_command();
    cmd.current_dir(&project_dir);

    let status = cmd.status();
    if let Some(path) = roots_path {
        let _ = std::fs::remove_file(path);
    }
    match status {
        Ok(s) => ExitCode::from(s.code().unwrap_or(0) as u8),
        Err(e) => {
            eprintln!("Failed to spawn contained command under sandbox-exec: {e}");
            ExitCode::from(1)
        }
    }
}
