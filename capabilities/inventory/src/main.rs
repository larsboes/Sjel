//! inventory: what the household owns. The only writer of the `inventory_` tables; `interior`
//! reads them to compute plans, and `trips` reads them over HTTP.

use sjel_inventory::intake::{self, FetchError, Page};
use sjel_inventory::store::{Item, Kind, State, Store};

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
  {import}                       inventory/*.toml into the tables (repeatable)
  {writeback}                    write slots into their marked vault region
  {serve}                        HTTP API for the dashboard
"#,
        t = bold("inventory"),
        inventory = bold("inventory"),
        categories = bold("categories"),
        add = bold("add"),
        import = bold("import"),
        writeback = bold("vault-writeback"),
        serve = bold("serve"),
    );
    std::process::exit(2)
}

fn flag(argv: &[String], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == &format!("--{name}"))
        .and_then(|i| argv.get(i + 1).cloned())
}

/// Where `inventory/*.toml` lives: this capability's own data dir, else interior's, where
/// existing machines keep it.
fn import_dir() -> Option<std::path::PathBuf> {
    let own = sjel_config::overlay_data_dir("inventory")?.join("inventory");
    if own.is_dir() {
        return Some(own);
    }
    let old = sjel_config::overlay_data_dir("interior")?.join("inventory");
    old.is_dir().then_some(old)
}

fn open() -> Result<Store, String> {
    Store::open(&sjel_config::database_path()).map_err(|e| format!("database not reachable: {e}"))
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

    let code = match cmd {
        "serve" => {
            sjel_inventory::api::serve().await;
            return;
        }
        "import" => import(&argv),
        "inventory" => show(),
        "categories" => categories_show(),
        "vault-writeback" => vault_writeback(),
        "add" => add(&argv).await,
        _ => usage(),
    };
    std::process::exit(code);
}

fn import(argv: &[String]) -> i32 {
    let Some(dir) = import_dir() else {
        eprintln!(
            "{}",
            red("no inventory/*.toml found in data/inventory/inventory or data/interior/inventory")
        );
        return 2;
    };
    let store = or_exit!(open());
    let force = argv.iter().any(|a| a == "--force");
    match store.item_count() {
        Ok(n) if n > 0 && !force => {
            eprintln!(
                "\n  {}\n\n  The tables are the truth, not `inventory/*.toml`. An import would reset\n  \
                 every change made in the dashboard to the state of the files.\n\n  {}\n",
                red(&format!("{n} entries are already in the database.")),
                dim("If that is what you mean: inventory import --force")
            );
            return 2;
        }
        Err(e) => {
            eprintln!("{}", red(&format!("database not readable: {e}")));
            return 2;
        }
        _ => {}
    }
    if force {
        eprintln!("  {}", dim("--force: the files overwrite the tables"));
    }
    let report = or_exit!(sjel_inventory::import::inventory(&store, &dir));
    println!(
        "\n  {} {} pieces, {} needs, {} state changes\n  {}\n",
        green("imported:"),
        report.pieces,
        report.slots,
        report.state_changes,
        dim(&format!("from {}", dir.display()))
    );
    0
}

fn show() -> i32 {
    let store = or_exit!(open());
    let rows = or_exit!(store.catalogue());
    if rows.is_empty() {
        println!("\n  {}\n", yellow("empty; `inventory import` fills the tables"));
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
        println!("\n{}", bold(&format!("  {} ({})", state.as_str(), group.len())));
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
    print_categories(&store);
    0
}

fn categories_show() -> i32 {
    let store = or_exit!(open());
    if print_categories(&store).is_empty() {
        println!("\n  {}\n", dim("no entry has a category"));
    }
    0
}

fn print_categories(store: &Store) -> Vec<(String, i64)> {
    let categories = store.categories().unwrap_or_default();
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
            dim("The field is free text, so nobody decided this. A PATCH on `category` cleans it up.")
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
            eprintln!("{}", red(&format!("`{other}` is no kind: piece, slot or gear")));
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

    let store = or_exit!(open());
    let catalogue = or_exit!(store.catalogue());
    // An explicit id that is taken is refused: a second shirt is a second entry, and an
    // overwritten one loses its history.
    let id = match flag(argv, "id") {
        Some(id) if catalogue.contains_key(&id) => {
            eprintln!(
                "{}",
                red(&format!("`{id}` exists already; without --id the next free id is used"))
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
    if let Err(e) = store.upsert_item(&item) {
        eprintln!("{}", red(&format!("not created: {e}")));
        return 2;
    }
    if let Err(e) = store.record_state(&id, State::Wanted, Some("created with `inventory add`")) {
        eprintln!("{}", red(&format!("state not written: {e}")));
        return 2;
    }

    println!("\n  {} {}  {}\n", green("created:"), bold(&id), dim("wanted"));
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

fn vault_writeback() -> i32 {
    let store = or_exit!(open());
    let rows = or_exit!(store.catalogue());
    let Some(outcome) = sjel_inventory::obsidian::writeback(&rows) else {
        eprintln!(
            "{}",
            red("no vault root declared: set obsidian.root in <overlay>/config/inventory.json")
        );
        return 2;
    };
    let report = match outcome {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 1;
        }
    };
    for path in &report.seeded {
        println!("  {} {path}", green("created:"));
    }
    for path in &report.written {
        println!("  {} {path}", green("region rewritten:"));
    }
    if !report.unchanged.is_empty() {
        println!("  {}", dim(&format!("{} unchanged", report.unchanged.len())));
    }
    if !report.conflicts.is_empty() {
        eprintln!("  {} {}", red("conflicts:"), report.conflicts.len());
        for k in &report.conflicts {
            eprintln!("    · {k}");
        }
        return 1;
    }
    0
}
