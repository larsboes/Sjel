//! Inventory CLI and HTTP service.

mod compare_cli;

use sjel_inventory::client::{self, Catalogue};
use sjel_inventory::intake::{self, FetchError, Page};
use sjel_inventory::store::{Item, Kind, State};

fn bold(s: &str) -> String {
    format!("\x1b[1m{s}\x1b[0m")
}
fn dim(s: &str) -> String {
    format!("\x1b[2m{s}\x1b[0m")
}
fn red(s: &str) -> String {
    format!("\x1b[31m{s}\x1b[0m")
}
fn green(s: &str) -> String {
    format!("\x1b[32m{s}\x1b[0m")
}
fn yellow(s: &str) -> String {
    format!("\x1b[33m{s}\x1b[0m")
}

fn usage() -> ! {
    eprintln!(
        r#"
{t}: what the household owns

  {inventory}                    what is owned and wanted, with state and price
  {categories}                   the categories in use, with counts
  {add} <url> [--label X]        add a link as a wish (title and declared price from the page;
                                 [--price 79,90] [--category kleidung] [--kind piece]
                                 [--size M] [--color white] [--seasons winter,spring]
                                 [--id some-id])
  compare [group]                wish groups: needs, candidates, latest price, specs
  specs <id> k=v[:unit]...       replace a wish's numeric specs (memory_gb=32)
  needs <group> k=v[:unit]...    replace a wish group's needs (memory_gb=29)
  alternatives <group> yes|no    one member gets bought: the budget counts the cheapest
  price <id> <amount>            record a dated price [--source X] [--config Y] [--on DATE]
  prices refresh                 read each candidate's geizhals product page once (daily job)
  {import}                       inventory/*.toml into the tables (repeatable)
  snapshot                       export a verified snapshot from the native owner
  {serve}                        HTTP API for the dashboard
"#,
        t = bold("inventory"),
        inventory = bold("inventory"),
        categories = bold("categories"),
        add = bold("add"),
        import = bold("import"),
        serve = bold("serve"),
    );
    std::process::exit(2)
}

fn flag(argv: &[String], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == &format!("--{name}"))
        .and_then(|i| argv.get(i + 1).cloned())
}

macro_rules! or_exit {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}", red(&e.to_string()));
                return 2;
            }
        }
    };
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().map(String::as_str) else {
        usage()
    };

    #[cfg(feature = "turso-wal")]
    if cmd == "serve-wal-preview" {
        std::process::exit(serve_wal_preview(&argv).await);
    }

    if cmd != "serve" && owner_url_needed() {
        eprintln!("Set SJEL_INVENTORY_URL to the service that owns the configured database.");
        std::process::exit(2);
    }

    let code = match cmd {
        "serve" => {
            if let Err(error) = sjel_inventory::api::serve().await {
                eprintln!("{}", red(&error.to_string()));
                std::process::exit(2);
            }
            return;
        }
        "import" => import(&argv).await,
        "snapshot" if argv.len() == 1 => snapshot().await,
        "inventory" => show().await,
        "categories" => categories_show().await,
        "add" => add(&argv).await,
        "prices" | "price" | "specs" | "needs" | "alternatives" | "compare" => {
            compare_cli::run(cmd, &argv).await
        }
        _ => usage(),
    };
    std::process::exit(code);
}

#[cfg(feature = "turso-wal")]
async fn serve_wal_preview(argv: &[String]) -> i32 {
    let (path, port, allow_writes) = match argv {
        [_, path, port] => (path, port, false),
        [_, path, port, flag] if flag == "--allow-writes" => (path, port, true),
        _ => {
            eprintln!(
                "usage: inventory serve-wal-preview <owner copy> <loopback port> [--allow-writes]"
            );
            return 2;
        }
    };
    let port = or_exit!(port.parse::<std::num::NonZeroU16>()).get();
    or_exit!(
        sjel_inventory::api::wal_preview::serve(std::path::Path::new(path), port, allow_writes)
            .await
    );
    0
}

fn owner_url_needed() -> bool {
    if sjel_config::env_var("SJEL_INVENTORY_URL").is_ok_and(|value| !value.trim().is_empty()) {
        return false;
    }
    let owner_override =
        sjel_config::env_var("SJEL_INVENTORY_DB_PATH").is_ok_and(|value| !value.trim().is_empty());
    let shared_override = sjel_config::env_var("SJEL_DB_PATH").is_ok_and(|value| {
        !value.trim().is_empty()
            && Some(sjel_config::expand_tilde(&value))
                != sjel_config::overlay_data_dir("sjel").map(|dir| dir.join("sjel.db"))
    });
    owner_override || shared_override
}

async fn snapshot() -> i32 {
    let result = or_exit!(
        tokio::task::spawn_blocking(|| { client::snapshot().map_err(|error| error.to_string()) })
            .await
    );
    or_exit!(result);
    0
}

async fn import(argv: &[String]) -> i32 {
    let force = argv.iter().any(|a| a == "--force");
    if force {
        eprintln!("  {}", dim("--force: the files overwrite the tables"));
    }
    let report = or_exit!(
        sjel_server::blocking(
            move || client::import_overlay(force).map_err(|error| error.to_string())
        )
        .await
    );
    println!(
        "\n  {} {} pieces, {} needs, {} state changes\n",
        green("imported:"),
        report.pieces,
        report.slots,
        report.state_changes
    );
    0
}

async fn owner_catalogue() -> Result<Catalogue, String> {
    sjel_server::blocking(|| client::catalogue().map_err(|error| error.to_string())).await
}

async fn show() -> i32 {
    let rows = or_exit!(owner_catalogue().await);
    if rows.is_empty() {
        println!(
            "\n  {}\n",
            yellow("empty; `inventory import` fills the tables")
        );
        return 0;
    }
    let mut open_total: i64 = 0;
    for state in [State::Owned, State::Wanted, State::Gone] {
        let group: Vec<_> = rows
            .values()
            .filter(|(_, s)| *s == Some(state))
            .map(|(i, _)| i)
            .collect();
        if group.is_empty() {
            continue;
        }
        println!(
            "\n{}",
            bold(&format!("  {} ({})", state.as_str(), group.len()))
        );
        for i in group {
            if state == State::Wanted {
                open_total += i.price_cents.or(i.cost_min_cents).unwrap_or(0);
            }
            let category = i.category.as_deref().unwrap_or("");
            let clothing = [i.size.as_deref(), i.color.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            let money = match (i.price_cents, i.cost_min_cents) {
                (Some(p), _) => euro(p),
                (None, Some(lo)) => format!("from {}", euro(lo)),
                _ => String::new(),
            };
            println!(
                "    {:<34} {:<12} {:<10} {}",
                i.id,
                dim(category),
                dim(&clothing),
                dim(&money)
            );
        }
    }
    println!(
        "\n  {} {}\n  {}\n",
        bold("open wishlist:"),
        euro(open_total),
        dim("low end: the product price, else the minimum of the estimate.")
    );
    print_categories(&rows);
    0
}

async fn categories_show() -> i32 {
    let rows = or_exit!(owner_catalogue().await);
    if print_categories(&rows).is_empty() {
        println!("\n  {}\n", dim("no entry has a category"));
    }
    0
}

fn print_categories(rows: &Catalogue) -> Vec<(String, i64)> {
    let mut counts = std::collections::BTreeMap::new();
    for (item, _) in rows.values() {
        if let Some(category) = item
            .category
            .as_deref()
            .map(|value| value.trim_matches(' '))
            .filter(|value| !value.is_empty())
        {
            *counts.entry(category.to_owned()).or_insert(0) += 1;
        }
    }
    let mut categories: Vec<_> = counts.into_iter().collect();
    categories.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if categories.is_empty() {
        return categories;
    }
    println!("{}", bold("  categories"));
    for (name, count) in &categories {
        println!("    {name:<26} {count:>4}");
    }
    let collisions = sjel_inventory::store::collisions(&categories);
    if !collisions.is_empty() {
        println!("\n  {}", yellow(&bold("  two spellings, one category")));
        for group in &collisions {
            println!("    · {}", group.join("  |  "));
        }
        println!(
            "  {}",
            dim(
                "The field is free text, so nobody decided this. A PATCH on `category` cleans it up."
            )
        );
    }
    println!();
    categories
}

/// `inventory add <url>`: a shared link becomes a wanted row.
async fn add(argv: &[String]) -> i32 {
    // The URL comes first and is not searched for, or `--label Shirt` would read `Shirt` as one.
    let Some(url) = argv.get(1).filter(|a| !a.starts_with("--")) else {
        eprintln!(
            "{}",
            red("which link? `inventory add <url>`: the URL first, the flags after it")
        );
        return 2;
    };

    let target = url.clone();
    // An unreachable page still creates the row: the link is the point, the title a bonus.
    let page = match tokio::task::spawn_blocking(move || intake::fetch(&target)).await {
        Ok(Ok(page)) => page,
        Ok(Err(refused @ FetchError::Refused(_))) => {
            eprintln!("{}", red(&format!("refused: {refused}")));
            return 2;
        }
        Ok(Err(error)) => {
            eprintln!("  {}", yellow(&format!("not fetched: {error}")));
            Page::default()
        }
        Err(error) => {
            eprintln!("  {}", yellow(&format!("not fetched: {error}")));
            Page::default()
        }
    };

    let Some(label) = flag(argv, "label").or_else(|| page.title.clone()) else {
        eprintln!(
            "{}",
            red("no title on the page and no --label; a row without a name is never found again")
        );
        return 2;
    };

    let kind = match flag(argv, "kind").as_deref() {
        None | Some("piece") => Kind::Piece,
        Some("slot") => Kind::Slot,
        Some("gear") => Kind::Gear,
        Some(other) => {
            eprintln!(
                "{}",
                red(&format!("`{other}` is no kind: piece, slot or gear"))
            );
            return 2;
        }
    };

    // A price seen in the shop beats the page. An unreadable amount is an error, not zero.
    let price_flag = flag(argv, "price");
    let price_cents = match &price_flag {
        Some(raw) => match intake::cents_from(raw) {
            Some(c) => Some(c),
            None => {
                eprintln!("{}", red(&format!("`{raw}` is no amount: 79,90 or 79.90")));
                return 2;
            }
        },
        None => page.price_cents,
    };

    let seasons: Vec<String> = flag(argv, "seasons")
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let catalogue = or_exit!(owner_catalogue().await);
    // An explicit id that is taken is refused: a second shirt is a second entry, and an
    // overwritten one loses its history.
    let id = match flag(argv, "id") {
        Some(id) if catalogue.contains_key(&id) => {
            eprintln!(
                "{}",
                red(&format!(
                    "`{id}` exists already; without --id the next free id is used"
                ))
            );
            return 2;
        }
        Some(id) => id,
        None => intake::free_id(&intake::slug(&label), |k| catalogue.contains_key(k)),
    };

    let item = Item {
        id: id.clone(),
        kind,
        label: label.clone(),
        link: Some(url.clone()),
        price_cents,
        category: flag(argv, "category"),
        size: flag(argv, "size"),
        color: flag(argv, "color"),
        seasons: seasons.clone(),
        // Same column as for a measured piece, so a reader sees the number came from a page.
        source: Some(format!("link {url}")),
        ..Default::default()
    };
    let request = item.clone();
    if let Err(e) = sjel_server::blocking(move || {
        client::create_wish(request).map_err(|error| error.to_string())
    })
    .await
    {
        eprintln!("{}", red(&format!("not created: {e}")));
        return 2;
    }

    println!(
        "\n  {} {}  {}\n",
        green("created:"),
        bold(&id),
        dim("wanted")
    );
    println!("    label     {label}");
    match price_cents {
        Some(c) => {
            let from = match (&price_flag, page.price_from.as_deref()) {
                (Some(_), _) => "(--price)",
                (None, Some(tag)) => tag,
                (None, None) => "",
            };
            println!("    price     {}  {}", euro(c), dim(from));
        }
        None => println!(
            "    price     {}",
            yellow("none read; the wishlist total counts it as 0 and GET /api/wishlist says so")
        ),
    }
    for (name, value) in [
        ("category", &item.category),
        ("size", &item.size),
        ("color", &item.color),
    ] {
        if let Some(v) = value {
            println!("    {name:<9} {v}");
        }
    }
    if !seasons.is_empty() {
        println!("    seasons   {}", seasons.join(", "));
    }
    println!("    link      {url}");
    println!("\n  {}\n", dim("`inventory inventory` shows the row"));
    0
}

fn euro(cents: i64) -> String {
    format!("{},{:02} €", cents / 100, (cents % 100).abs())
}
