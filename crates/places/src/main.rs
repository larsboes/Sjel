//! Places HTTP server and local maintenance commands.

mod commands;
#[cfg(feature = "turso-wal")]
mod jobs;
mod server;

use places::{
    config::Config,
    store::{Fallible, PlacesStore},
};

fn command(args: &[String]) -> Fallible<()> {
    #[cfg(feature = "turso-wal")]
    {
        if args == ["snapshot"] {
            return jobs::snapshot();
        }
        if jobs::request(args, &mut std::io::stdout().lock())? {
            return Ok(());
        }
    }
    let config = Config::load();
    let store = PlacesStore::open(&config.database_path)?;
    commands::run(&store, args, &mut std::io::stdout().lock())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config::load();
    #[cfg(feature = "turso-wal")]
    if config.database_path == sjel_config::database_path() {
        eprintln!("Turso requires a separate Places owner database");
        std::process::exit(1);
    }
    if args.is_empty() || args == ["server"] {
        tokio::runtime::Runtime::new()
            .expect("tokio runtime could not start")
            .block_on(server::serve(config));
    } else if let Err(error) = command(&args) {
        eprintln!("{error}");
        std::process::exit(if error.to_string() == commands::USAGE {
            2
        } else {
            1
        });
    }
}
