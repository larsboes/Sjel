//! The places binary: the HTTP server by default (`service.toml` runs it bare),
//! one-shot backfills as subcommands (README "Backfills"). Backfills are CLI
//! verbs rather than routes because each runs once, prints counts and exits —
//! a route would be a standing surface for a non-standing job.

mod server;

fn usage() -> ! {
    eprintln!(
        "usage: places-server [server \
         | backfill <amex|cities|stations|takeout|travelers|vault> \
         | climate fetch [--place <id>] [--force]]"
    );
    std::process::exit(2);
}

/// `climate fetch` is a top-level verb rather than a `backfill` arm.
///
/// A backfill, as the module comment above defines it, runs once, prints counts
/// and exits. This one is re-runnable per place — it is the refresh path a
/// permanent cache needs — and it takes flags the `backfill <name>` dispatch has
/// no room for.
///
/// Without `--place` it covers `kind = 'city'` only. A venue's climate is its
/// city's climate, so fetching all 183 registry rows would egress 134 venue
/// coordinates for an identical answer, at one provider request each; the read
/// side resolves a venue from its city row anyway. `kind = 'address'` is refused
/// by `fetch_normals` itself (ISA PLC-15), whichever path names it.
fn climate(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.first().map(String::as_str) != Some("fetch") {
        usage();
    }
    let mut place_id: Option<String> = None;
    let mut force = false;
    let mut rest = args[1..].iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--force" => force = true,
            "--place" => place_id = Some(rest.next().cloned().unwrap_or_else(|| usage())),
            _ => usage(),
        }
    }

    let config = places::config::Config::load();
    let store = places::store::PlacesStore::open(&config.database_path)?;
    let today = places::today();
    let url = places::climate::archive_url();

    let targets: Vec<places::store::Place> = match &place_id {
        Some(id) => vec![store.place(id)?.ok_or_else(|| format!("no place {id}"))?],
        None => store
            .search_places(None, Some("city"))?
            .into_iter()
            .filter(|place| place.latitude.is_some() && place.longitude.is_some())
            .collect(),
    };

    let (mut fetched, mut cached, mut failed) = (0_usize, 0_usize, 0_usize);
    for place in &targets {
        match places::climate::fetch_normals(&store, place, &url, &today, force) {
            Ok(outcome) if outcome.cached => cached += 1,
            Ok(outcome) => {
                fetched += 1;
                println!("{}: {} months", place.id, outcome.months);
            }
            Err(error) => {
                failed += 1;
                // The message is already stripped of the request URL, which is
                // the coordinate pair (climate.rs, ISA PLC-13).
                eprintln!("{}: {error}", place.id);
            }
        }
    }
    println!(
        "climate fetch: {} considered, {fetched} fetched, {cached} served from cache, {failed} failed",
        targets.len()
    );
    if failed > 0 && fetched == 0 {
        return Err("every climate fetch failed".into());
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("server") => {
            tokio::runtime::Runtime::new()
                .expect("tokio runtime could not start")
                .block_on(server::serve());
        }
        Some("backfill") => {
            let config = places::config::Config::load();
            let store = match places::store::PlacesStore::open(&config.database_path) {
                Ok(store) => store,
                Err(error) => {
                    eprintln!("places backfill: cannot open store: {error}");
                    std::process::exit(1);
                }
            };
            let today = places::today();
            let result = match args.get(1).map(String::as_str) {
                Some("amex") => places::backfill::amex(&store, &today),
                Some("cities") => places::backfill::cities(&store, &today),
                Some("stations") => places::backfill::stations(&store, &today),
                Some("takeout") => places::backfill::takeout(&store, &today),
                Some("travelers") => places::backfill::travelers(&store, &today),
                Some("vault") => places::backfill::vault(&store, &today),
                _ => usage(),
            };
            if let Err(error) = result {
                eprintln!("places backfill failed: {error}");
                std::process::exit(1);
            }
            // A backfill may have written new person names; link the exact ones (D11). Best
            // effort: with entities down they stay open, and the server retries at start.
            match places::people::link_exact(&store) {
                Ok(0) => {}
                Ok(n) => eprintln!("places: linked {n} person name(s) to people"),
                Err(error) => eprintln!("places: person names not linked: {error}"),
            }
        }
        Some("climate") => {
            if let Err(error) = climate(&args[1..]) {
                eprintln!("places climate failed: {error}");
                std::process::exit(1);
            }
        }
        Some(_) => usage(),
    }
}
