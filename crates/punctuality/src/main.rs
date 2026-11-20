//! punctuality — ingest published DB stop history, read the resulting statistics.

use punctuality::{commands::flag, config::Config};

#[cfg(not(feature = "turso-wal"))]
fn store_command(args: &[String]) -> Result<String, Box<dyn std::error::Error>> {
    let config = Config::load();
    let store = punctuality::store::Store::open(&config.database_path)?;
    punctuality::commands::run(args, &config, &store)
}

#[cfg(feature = "turso-wal")]
fn store_command(args: &[String]) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::blocking::Client::builder()
        .unix_socket(sjel_server::owner_snapshot::socket_path("punctuality")?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(None)
        .build()?;
    let url = "http://127.0.0.1/commands";
    let response =
        sjel_server::InboundAuth::with_loopback_auth(client.post(url).json(args), url).send()?;
    let status = response.status();
    let result = response.text()?;
    if status.is_success() {
        Ok(result)
    } else {
        Err(format!("punctuality owner command returned {status}: {result}").into())
    }
}

const USAGE: &str = "\
Usage:
  punctuality ingest [--from YYYY-MM] [--to YYYY-MM]   download + incrementally aggregate monthly releases
  punctuality stats <station|eva> [--type ICE] [--min-n N]
  punctuality stations <needle>                        eva lookup by name
  punctuality ride --type ICE --number 611 --date YYYY-MM-DD [--eva 8000044]
                                                       one train's actual stops that day

Defaults: --from 2025-12 (first month covering every station, not just the largest ~100),
--to the newest published month, --min-n 30.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("ingest" | "stats" | "stations") => store_command(&args).map(|output| {
            if !output.is_empty() {
                println!("{}", output.trim_end());
            }
        }),
        Some("ride") => ride_cmd(&args),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("punctuality: {e}");
        std::process::exit(1);
    }
}

/// One train's actual stops on one day, straight from the cached monthly file.
///
/// Reads the columns `ingest` throws away. Deliberately CLI-only and
/// deliberately not stored: this is a lookup against files already on disk, and
/// giving it a table would create a second copy of DB's own published data.
fn ride_cmd(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::load();
    let train_type = flag(args, "--type").ok_or("ride needs --type (e.g. ICE)")?;
    let number = flag(args, "--number").ok_or("ride needs --number (e.g. 611)")?;
    let date = flag(args, "--date").ok_or("ride needs --date YYYY-MM-DD")?;
    let eva = flag(args, "--eva");

    let answer =
        punctuality::ride::find(&cfg.raw_dir, &train_type, &number, &date, eva.as_deref())?;
    if answer.stops.is_empty() && answer.unavailable.is_none() {
        eprintln!(
            "punctuality: no stops for {train_type} {number} on {date}. Cached months: {}",
            punctuality::ride::cached_months(&cfg.raw_dir).join(", ")
        );
    }
    println!("{}", serde_json::to_string_pretty(&answer)?);
    Ok(())
}
