use documents::commands::USAGE;

fn main() {
    if let Err(error) = run() {
        eprintln!("documents: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("serve") if args.len() == 1 => documents::server::serve(),
        #[cfg(feature = "turso-wal")]
        Some("snapshot") if args.len() == 1 => documents::jobs::snapshot(),
        Some("watch") if args.len() == 2 => {
            resolve_paths(&mut args)?;
            println!(
                "watching {}; scans every 60 seconds; stop with Ctrl-C",
                args[1]
            );
            args[0] = "watch-scan".into();
            loop {
                if let Err(error) = run_command(&args) {
                    eprintln!("documents: {error}");
                }
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        Some("watch-scan") | None => Err(USAGE.into()),
        Some(_) => {
            resolve_paths(&mut args)?;
            run_command(&args)
        }
    }
}

fn resolve_paths(args: &mut [String]) -> Result<(), Box<dyn std::error::Error>> {
    let index = match args.first().map(String::as_str) {
        Some("intake" | "inbox" | "watch") => 1,
        Some("export") => 2,
        _ => return Ok(()),
    };
    if let Some(path) = args.get_mut(index).filter(|path| !path.starts_with("--")) {
        let absolute = std::path::absolute(&*path)?;
        *path = absolute
            .into_os_string()
            .into_string()
            .map_err(|_| "document CLI path must be valid UTF-8")?;
    }
    Ok(())
}

fn run_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "turso-wal")]
    if documents::jobs::request(args, &mut std::io::stdout())? {
        return Ok(());
    }
    documents::commands::run(&documents::open_default()?, args, &mut std::io::stdout())
}
