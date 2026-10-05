//! inventory — was ich besitze.
//!
//! Die Zeilen liegen in der einen geteilten SQLite-Datei unter `inventory_`. Diese Capability
//! ist die einzige, die sie schreibt; `interior` liest sie, weil es ohne sie keinen Plan
//! rechnen kann, und `trips` liest sie ueber die HTTP-Oberflaeche hier.
//!
//! Sie ist `autostart = true`, und das ist der Grund, aus dem es sie gibt: bis 2026-10-05 lagen
//! diese Zeilen in `interior`, und `interior` ist absichtlich on-demand. Ein Neustart nahm
//! damit jeder Packliste ihre Gewichte, bis jemand den Grundriss oeffnete (ISA F13).

use inventory::store::{State, Store};

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
{t} — was ich besitze

  {inventory}                    was da ist und was fehlt, mit Zustand und Preis
  {kategorien}                   die Zweige, die wirklich vorkommen, mit ihren Zahlen
  {wunsch} <url> [--label X]     einen Link als Wunsch eintragen (Titel und deklarierten
                                 Preis von der Seite; [--preis 79,90] [--category kleidung]
                                 [--kind piece] [--groesse M] [--farbe weiss]
                                 [--saison winter,uebergang] [--id kennung])
  {import}                       inventory/*.toml in die Tabellen (wiederholbar)
  {writeback}                    Slots in die markierte Region des Vaults schreiben
  {serve}                        HTTP-API fuer die Oberflaeche
"#,
        t = bold("inventory"),
        inventory = bold("inventory"),
        kategorien = bold("kategorien"),
        wunsch = bold("wunsch"),
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

/// Wo `inventory/*.toml` liegt — die Migrationsquelle (PRD B25), kein zweiter Bestand.
///
/// Zwei Orte, in dieser Reihenfolge: `data/inventory/inventory` ist der eigene seit 2026-10-05,
/// `data/interior/inventory` ist der historische. Ein Umzug, der die Dateien verschiebt, waere
/// eine Bewegung ohne Gewinn — gelesen werden sie einmal, danach sind die Tabellen die Wahrheit.
fn import_dir() -> Option<std::path::PathBuf> {
    let eigen = sjel_config::overlay_data_dir("inventory")?.join("inventory");
    if eigen.is_dir() {
        return Some(eigen);
    }
    let alt = sjel_config::overlay_data_dir("interior")?.join("inventory");
    alt.is_dir().then_some(alt)
}

fn open() -> Result<Store, String> {
    Store::open(&sjel_config::database_path())
        .map_err(|e| format!("Datenbank nicht erreichbar: {e}"))
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().map(String::as_str) else {
        usage()
    };

    let code = match cmd {
        "serve" => {
            inventory::api::serve().await;
            return;
        }
        "import" => inventory_import(&argv),
        "inventory" => inventory_show(),
        "kategorien" => kategorien_show(),
        "vault-writeback" => vault_writeback(),
        "wunsch" => wunsch(&argv).await,
        _ => usage(),
    };
    std::process::exit(code);
}

fn inventory_import(argv: &[String]) -> i32 {
    let Some(dir) = import_dir() else {
        eprintln!(
            "{}",
            red("keine inventory/*.toml gefunden — data/inventory/inventory oder data/interior/inventory")
        );
        return 2;
    };
    let store = match open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&e));
            return 2;
        }
    };
    let force = argv.iter().any(|a| a == "--force");
    match store.item_count() {
        Ok(n) if n > 0 && !force => {
            eprintln!(
                "\n  {}\n\n  Die Tabellen sind seit PRD Q64 die Wahrheit, nicht `inventory/*.toml`.\n  \
                 Ein Import wuerde jede Aenderung aus der Oberflaeche auf den Stand der Dateien\n  \
                 zuruecksetzen.\n\n  {}\n",
                red(&format!("{n} Eintraege stehen schon in der Datenbank.")),
                dim("Wenn genau das gemeint ist: inventory import --force")
            );
            return 2;
        }
        Err(e) => {
            eprintln!("{}", red(&format!("Datenbank nicht lesbar: {e}")));
            return 2;
        }
        _ => {}
    }
    if force {
        eprintln!(
            "  {}",
            dim("--force: die Dateien ueberschreiben die Tabellen")
        );
    }
    match inventory::import::inventory(&store, &dir) {
        Ok(b) => {
            println!(
                "\n  {} {} Stuecke, {} Bedarfe, {} Zustandswechsel\n  {}\n",
                green("importiert:"),
                b.pieces,
                b.slots,
                b.zustandswechsel,
                dim(&format!("aus {}", dir.display()))
            );
            0
        }
        Err(e) => {
            eprintln!("{}", red(&format!("Import fehlgeschlagen: {e}")));
            2
        }
    }
}

fn inventory_show() -> i32 {
    let store = match open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&e));
            return 2;
        }
    };
    let rows = match store.catalogue() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    if rows.is_empty() {
        println!(
            "\n  {}\n",
            yellow("leer — `inventory import` fuellt die Tabellen")
        );
        return 0;
    }
    let mut summe: i64 = 0;
    for state in [State::Owned, State::Wanted, State::Gone] {
        let gruppe: Vec<_> = rows
            .values()
            .filter(|(_, s)| *s == Some(state))
            .map(|(i, _)| i)
            .collect();
        if gruppe.is_empty() {
            continue;
        }
        println!(
            "\n{}",
            bold(&format!("  {} ({})", state.as_str(), gruppe.len()))
        );
        for i in gruppe {
            if state == State::Wanted {
                summe += i.preis_cent.or(i.kosten_min_cent).unwrap_or(0);
            }
            let zweig = i.category.as_deref().unwrap_or("");
            let kleid = match (&i.groesse, &i.farbe) {
                (None, None) => String::new(),
                (Some(g), None) => g.clone(),
                (None, Some(f)) => f.clone(),
                (Some(g), Some(f)) => format!("{g} {f}"),
            };
            let geld = match (i.preis_cent, i.kosten_min_cent) {
                (Some(p), _) => format!("{:.2} €", p as f64 / 100.0),
                (None, Some(lo)) => format!("ab {:.2} €", lo as f64 / 100.0),
                _ => String::new(),
            };
            println!(
                "    {:<34} {:<12} {:<10} {}",
                i.id,
                dim(zweig),
                dim(&kleid),
                dim(&geld)
            );
        }
    }
    println!(
        "\n  {} {:.2} €\n  {}\n",
        bold("offener Bedarf:"),
        summe as f64 / 100.0,
        dim("untere Kante: Produktpreis, sonst das Minimum der Schaetzung.")
    );
    zweige(&store);
    0
}

fn kategorien_show() -> i32 {
    let store = match open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&e));
            return 2;
        }
    };
    if zweige(&store).is_empty() {
        println!("\n  {}\n", dim("kein Eintrag traegt einen Zweig"));
    }
    0
}

/// Die Zweige — gelesen und nicht deklariert, wie `budget::BEKANNTE_PRIORITAETEN`. Eine hier
/// abgeschriebene Liste waere die zweite Wahrheit ueber dieselbe Sache.
fn zweige(store: &Store) -> Vec<(String, i64)> {
    let zweige = store.kategorien().unwrap_or_default();
    if zweige.is_empty() {
        return zweige;
    }
    println!("{}", bold("  Zweige (category)"));
    for (name, anzahl) in &zweige {
        println!("    {name:<26} {anzahl:>4}");
    }
    let kollisionen = inventory::store::kollisionen(&zweige);
    if !kollisionen.is_empty() {
        println!("\n  {}", yellow(&bold("  zwei Schreibweisen, ein Zweig")));
        for gruppe in &kollisionen {
            println!("    · {}", gruppe.join("  |  "));
        }
        println!(
            "  {}",
            dim("Das Feld ist frei, also hat das niemand entschieden. Ein `PATCH` auf `category` raeumt es auf.")
        );
    }
    println!();
    zweige
}

/// `inventory wunsch <url>` — ein geteilter Link wird eine Wunschzeile.
async fn wunsch(argv: &[String]) -> i32 {
    // Die URL steht vorn und wird nicht gesucht. `--label Hemd` haette sonst `Hemd` als URL
    // gelesen, und der Fehler saehe aus wie ein Netzproblem.
    let Some(url) = argv.get(1).filter(|a| !a.starts_with("--")) else {
        eprintln!(
            "{}",
            red("welcher Link? `inventory wunsch <url>` — die URL steht vorn, die Flags dahinter")
        );
        return 2;
    };

    // Ein Link, der nicht antwortet, ist kein Grund, die Zeile nicht anzulegen: der Link ist
    // der Punkt und der Titel die Zugabe. Dann braucht es aber `--label`.
    // `spawn_blocking` und nicht `block_in_place`: die Threads, die `spawn_blocking` faehrt,
    // sind keine Runtime-Worker, und dort darf der blockierende Client stehen.
    let abruf = url.clone();
    let gelesen = match tokio::task::spawn_blocking(move || inventory::wunsch::holen(&abruf)).await
    {
        Ok(Ok(g)) => g,
        // Eine abgelehnte URL beendet den Aufruf. Sie ist eine Entscheidung und kein Umstand:
        // `file://` und eine Adresse in diesem Netz werden nicht "trotzdem" eingetragen, sonst
        // waere die Wache eine Warnung.
        Ok(Err(abruf @ inventory::wunsch::Abruf::Abgelehnt(_))) => {
            eprintln!("{}", red(&format!("abgelehnt: {abruf}")));
            return 2;
        }
        Ok(Err(nicht_erreicht)) => {
            eprintln!("  {}", yellow(&format!("nicht geholt: {nicht_erreicht}")));
            inventory::wunsch::Gelesen::default()
        }
        Err(e) => {
            eprintln!("  {}", yellow(&format!("nicht geholt: {e}")));
            inventory::wunsch::Gelesen::default()
        }
    };

    let label = flag(argv, "label").or_else(|| gelesen.titel.clone());
    let Some(label) = label else {
        eprintln!(
            "{}",
            red("kein Titel auf der Seite und kein --label — eine Zeile ohne Namen findet niemand wieder")
        );
        return 2;
    };

    let art = match flag(argv, "kind").as_deref() {
        None | Some("piece") => inventory::store::Kind::Piece,
        Some("slot") => inventory::store::Kind::Slot,
        Some("gear") => inventory::store::Kind::Gear,
        Some(andere) => {
            eprintln!(
                "{}",
                red(&format!("`{andere}` ist keine Art — piece, slot oder gear"))
            );
            return 2;
        }
    };

    // `--preis` schlaegt die Seite: wer den Preis im Laden gesehen hat, hat die bessere Zahl.
    // Ein unlesbarer Betrag ist ein Fehler und keine Null — dieselbe Regel wie in `wunsch.rs`.
    let preis_flag = flag(argv, "preis");
    let preis_cent = match &preis_flag {
        Some(roh) => match inventory::wunsch::betrag_cent(roh) {
            Some(c) => Some(c),
            None => {
                eprintln!(
                    "{}",
                    red(&format!("`{roh}` ist kein Betrag — 79,90 oder 79.90"))
                );
                return 2;
            }
        },
        None => gelesen.preis_cent,
    };

    let saison: Vec<String> = flag(argv, "saison")
        .map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let store = match open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&e));
            return 2;
        }
    };
    let katalog = match store.catalogue() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{}", red(&format!("Katalog laedt nicht: {e}")));
            return 2;
        }
    };
    // `--id` heisst genau diese Kennung. Ist sie belegt, wird sie nicht ueberschrieben: ein
    // zweites Hemd ist ein zweiter Eintrag, und ein ueberschriebener verliert seine Geschichte.
    let kennung = match flag(argv, "id") {
        Some(id) => {
            if katalog.contains_key(&id) {
                eprintln!(
                    "{}",
                    red(&format!(
                        "`{id}` gibt es schon — ohne --id nimmt der Aufruf die naechste freie Kennung"
                    ))
                );
                return 2;
            }
            id
        }
        None => inventory::wunsch::freie_kennung(&inventory::wunsch::kennung(&label), |k| {
            katalog.contains_key(k)
        }),
    };

    let category = flag(argv, "category");
    let groesse = flag(argv, "groesse");
    let farbe = flag(argv, "farbe");
    // Die Zeile wird hier gebaut und nicht in `wunsch.rs`: das Modul liest einen Link, diese
    // Funktion schreibt eine Zeile.
    let item = inventory::store::Item {
        id: kennung.clone(),
        kind: art,
        label: label.clone(),
        link: Some(url.clone()),
        preis_cent,
        category: category.clone(),
        groesse: groesse.clone(),
        farbe: farbe.clone(),
        saison: saison.clone(),
        // Die Herkunft steht in derselben Spalte wie bei einer Schaetzung: wer die Zeile
        // spaeter liest, soll sehen, dass die Zahl von einer Seite kommt und nicht von einem
        // Bandmass.
        quelle: Some(format!("Link {url}")),
        ..Default::default()
    };
    if let Err(e) = store.upsert_item(&item) {
        eprintln!("{}", red(&format!("nicht angelegt: {e}")));
        return 2;
    }
    if let Err(e) = store.record_state(
        &kennung,
        State::Wanted,
        Some("ueber `inventory wunsch` angelegt"),
    ) {
        eprintln!("{}", red(&format!("Zustand nicht geschrieben: {e}")));
        return 2;
    }

    println!(
        "\n  {} {}  {}\n",
        green("angelegt:"),
        bold(&kennung),
        dim("wanted")
    );
    println!("    label   {label}");
    match preis_cent {
        Some(c) => {
            let quelle = match (&preis_flag, gelesen.preis_aus.as_deref()) {
                (Some(_), _) => "(--preis)",
                (None, Some(aus)) => aus,
                (None, None) => "",
            };
            println!("    preis   {}  {}", euro(c), dim(quelle));
        }
        None => println!(
            "    preis   {}",
            yellow(
                "keiner gelesen — die Wunschsumme zaehlt ihn als 0; GET /api/wishlist nennt die Zahl"
            )
        ),
    }
    if let Some(c) = category {
        println!("    kategorie {c}");
    }
    if let Some(g) = groesse {
        println!("    groesse {g}");
    }
    if let Some(f) = farbe {
        println!("    farbe   {f}");
    }
    if !saison.is_empty() {
        println!("    saison  {}", saison.join(", "));
    }
    println!("    link    {url}");
    println!("\n  {}\n", dim("`inventory inventory` zeigt die Zeile"));
    0
}

/// Cent als Euro mit Komma. Die einzige Rechnung, die eine Anzeige fuehren darf.
fn euro(cent: i64) -> String {
    format!("{},{:02} €", cent / 100, (cent % 100).abs())
}

fn vault_writeback() -> i32 {
    let store = match open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&e));
            return 2;
        }
    };
    let rows = match store.catalogue() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    let Some(ergebnis) = inventory::obsidian::writeback(&rows) else {
        eprintln!(
            "{}",
            red("keine Vault-Wurzel erklaert: obsidian.root in <overlay>/config/inventory.json setzen")
        );
        return 2;
    };
    let report = match ergebnis {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 1;
        }
    };
    for pfad in &report.seeded {
        println!("  {} {pfad}", green("angelegt:"));
    }
    for pfad in &report.written {
        println!("  {} {pfad}", green("Region neu:"));
    }
    if !report.unchanged.is_empty() {
        println!(
            "  {}",
            dim(&format!("{} unveraendert", report.unchanged.len()))
        );
    }
    if !report.conflicts.is_empty() {
        eprintln!("  {} {}", red("Konflikte:"), report.conflicts.len());
        for k in &report.conflicts {
            eprintln!("    · {k}");
        }
        return 1;
    }
    0
}
