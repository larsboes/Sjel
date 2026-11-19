//! `inventory prices|price|specs|needs|compare`: the wish comparison from the command line.
//!
//! `prices refresh` is what the daily job runs. It reads each candidate's `link` when that link
//! is a geizhals product page (`intake::refresh_source`) and records the page's lowest offer.
//! Every other price is a manual `price` entry with its own source label.

use std::time::Duration;

use inventory_api::{PriceRecorded, RecordPriceRequest};
use sjel_inventory::store::compare::{parse_spec, GroupComparison};
use sjel_inventory::{client, intake};

/// Between two page reads. geizhals asks crawlers it names for one second; this stays above it.
const PAUSE: Duration = Duration::from_secs(3);

fn flag(argv: &[String], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == &format!("--{name}"))
        .and_then(|i| argv.get(i + 1).cloned())
}

fn euros(cents: i64) -> String {
    format!("{}.{:02} €", cents / 100, cents % 100)
}

pub async fn run(cmd: &str, argv: &[String]) -> i32 {
    match cmd {
        "prices" => prices(argv).await,
        "price" => price(argv).await,
        "specs" => numbers(argv, false).await,
        "needs" => numbers(argv, true).await,
        "alternatives" => alternatives(argv).await,
        _ => compare(argv).await,
    }
}

async fn comparisons() -> Result<Vec<GroupComparison>, String> {
    sjel_server::blocking(|| client::comparisons().map_err(|error| error.to_string())).await
}

async fn record_price(id: &str, request: RecordPriceRequest) -> Result<PriceRecorded, String> {
    let id = id.to_owned();
    sjel_server::blocking(move || {
        client::record_price(&id, &request).map_err(|error| error.to_string())
    })
    .await
}

/// `prices refresh`: one pass over every candidate in every group, then exit.
/// Exit 1 when a page was due and could not be read, so the scheduler shows the failure.
async fn prices(argv: &[String]) -> i32 {
    if argv.get(1).map(String::as_str) != Some("refresh") {
        eprintln!("usage: inventory prices refresh");
        return 2;
    }
    let groups = match comparisons().await {
        Ok(groups) => groups,
        Err(e) => {
            eprintln!("cannot read the wish groups: {e}");
            return 2;
        }
    };
    let mut seen = std::collections::BTreeSet::new();
    let (mut read, mut failed) = (0, 0);
    for candidate in groups.iter().flat_map(|g| &g.candidates) {
        let Some(link) = candidate.link.clone() else {
            continue;
        };
        if !seen.insert(candidate.id.clone()) || intake::refresh_source(&link).is_err() {
            continue;
        }
        if read + failed > 0 {
            tokio::time::sleep(PAUSE).await;
        }
        let target = link.clone();
        let page = tokio::task::spawn_blocking(move || intake::fetch_for_refresh(&target)).await;
        let cents = match page {
            Ok(Ok(page)) => page.price_cents,
            Ok(Err(e)) => {
                eprintln!("{}: {e}", candidate.id);
                None
            }
            Err(e) => {
                eprintln!("{}: {e}", candidate.id);
                None
            }
        };
        let Some(cents) = cents else {
            eprintln!("{}: no price on {link}", candidate.id);
            failed += 1;
            continue;
        };
        read += 1;
        let request = RecordPriceRequest {
            price_cents: cents,
            source: link,
            configuration: String::new(),
            observed_at: None,
        };
        match record_price(&candidate.id, request).await {
            Ok(r) if r.alert => println!(
                "ALERT {}: {} is at or below the limit",
                candidate.id,
                euros(cents)
            ),
            Ok(r) if r.added => println!("{}: {}", candidate.id, euros(cents)),
            Ok(_) => println!(
                "{}: {} (already recorded today)",
                candidate.id,
                euros(cents)
            ),
            Err(e) => {
                eprintln!("{}: {e}", candidate.id);
                failed += 1;
            }
        }
    }
    println!("{read} read, {failed} failed");
    i32::from(failed > 0)
}

/// `price <id> <amount> [--source X] [--config Y] [--on YYYY-MM-DD]`: a manual observation.
async fn price(argv: &[String]) -> i32 {
    let (Some(id), Some(amount)) = (argv.get(1), argv.get(2)) else {
        eprintln!(
            "usage: inventory price <id> <amount> [--source X] [--config Y] [--on YYYY-MM-DD]"
        );
        return 2;
    };
    let Some(cents) = intake::cents_from(amount) else {
        eprintln!("`{amount}` is not a price");
        return 2;
    };
    let source = flag(argv, "source").unwrap_or_else(|| "manual".into());
    let request = RecordPriceRequest {
        price_cents: cents,
        source: source.clone(),
        configuration: flag(argv, "config").unwrap_or_default(),
        observed_at: flag(argv, "on"),
    };
    match record_price(id, request).await {
        Ok(r) if r.alert => {
            println!("ALERT {id}: {} is at or below the limit", euros(cents));
            0
        }
        Ok(r) if r.added => {
            println!("{id}: {} from {source}", euros(cents));
            0
        }
        Ok(_) => {
            println!("{id}: {source} has a price for that day already; nothing written");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `specs <id> key=value[:unit]...` and `needs <group> key=value[:unit]...`. The list replaces
/// the stored one; no pairs clears it.
async fn numbers(argv: &[String], needs: bool) -> i32 {
    let Some(owner) = argv.get(1) else {
        eprintln!("usage: inventory {} <id> key=value[:unit]...", argv[0]);
        return 2;
    };
    let mut parsed = Vec::new();
    for arg in &argv[2..] {
        match parse_spec(arg) {
            Ok(spec) => parsed.push(spec),
            Err(e) => {
                eprintln!("{e}");
                return 2;
            }
        }
    }
    let count = parsed.len();
    let id = owner.clone();
    let result = sjel_server::blocking(move || {
        if needs {
            client::set_needs(&id, parsed)
        } else {
            client::set_specs(&id, parsed)
        }
        .map_err(|error| error.to_string())
    })
    .await;
    match result {
        Ok(()) => {
            println!("{owner}: {count} value(s)");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `alternatives <group> yes|no`: one member of the group gets bought, so a budget counts only
/// the cheapest.
async fn alternatives(argv: &[String]) -> i32 {
    let (Some(group), Some(mark)) = (argv.get(1), argv.get(2)) else {
        eprintln!("usage: inventory alternatives <group> yes|no");
        return 2;
    };
    let on = match mark.as_str() {
        "yes" => true,
        "no" => false,
        _ => {
            eprintln!("usage: inventory alternatives <group> yes|no");
            return 2;
        }
    };
    let id = group.clone();
    match sjel_server::blocking(move || {
        client::set_alternatives(&id, on).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(()) => {
            println!(
                "{group}: {}",
                if on {
                    "alternatives, the cheapest counts"
                } else {
                    "every member counts"
                }
            );
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

/// `compare [group]`: needs, then one line per candidate with its latest price and specs.
async fn compare(argv: &[String]) -> i32 {
    let groups = match comparisons().await {
        Ok(groups) => groups,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let wanted = argv.get(1);
    for group in groups.iter().filter(|g| wanted.is_none_or(|w| &g.id == w)) {
        print_group(group);
    }
    0
}

fn print_group(group: &GroupComparison) {
    println!("\n{} ({})", group.title, group.id);
    if !group.needs.is_empty() {
        let needs: Vec<String> = group
            .needs
            .iter()
            .map(|n| format!("{}={}{}", n.key, n.value, n.unit))
            .collect();
        println!("  needs: {}", needs.join(", "));
    }
    for c in &group.candidates {
        let latest = c
            .observations
            .last()
            .map(|o| format!("{} on {}", euros(o.price_cents), &o.observed_at[..10]))
            .unwrap_or_else(|| "no price".into());
        let specs: Vec<String> = c
            .specs
            .iter()
            .map(|s| format!("{}={}", s.key, s.value))
            .collect();
        let limit = c.cost_max_cents.map(euros).unwrap_or_else(|| "—".into());
        println!(
            "  {:<28} {:<24} limit {:<12} {}",
            c.id,
            latest,
            limit,
            specs.join(" ")
        );
    }
}
