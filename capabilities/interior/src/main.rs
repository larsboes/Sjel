//! interior — Raumplanung.
//!
//! Welche Wohnung gemeint ist, entscheidet `SJEL_INTERIOR_FLAT` oder `--flat`; liegt genau
//! eine unter `data/interior/flats/`, ist es die. Kein Name steht hier im Quelltext — er stand
//! es bis 2026-08-30, und `tests/containment.rs` sorgt dafuer, dass er nicht zurueckkommt.
//!
//! Liest das gemessene Modell aus dem privaten Overlay, prueft Layouts gegen die
//! Raeumungsregeln und sucht Positionen, die sie erfuellen. `check` beendet sich mit einem
//! Fehlercode, wenn eine harte Regel verletzt ist, und taugt damit als Gate.
//!
//! `search --out` schreibt den besten Treffer als echtes Layout auf die Platte. Das ist kein
//! Komfort: bis zum 2026-08-30 lebten Suchergebnisse ausschliesslich in einer Terminalausgabe
//! und waren mit der Sitzung weg.

use interior::clearance::{check_layout, CheckResult};
use interior::model::{default_flat, Layout, Model};
use interior::plan;
use interior::search::{search, Spec};
use std::collections::BTreeMap;

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
{t} — Raumplanung

  {model}                        gemessener Raum, und was daran noch geraten ist
  {roomplan} [--capture D]       was in der RoomPlan-Aufnahme steht, gegen room.toml
                                 [--write]  Entwurf als draft.json neben die Aufnahme
                                 [--json]   dieselbe Auskunft als JSON
  {placements} [--set id=x,y[,r]]  wo die Stuecke WIRKLICH stehen, und was noch keinen
                                 Platz hat [--plan f] [--json]
  {layouts}                      Layouts auf der Platte
  {check} <layout> [--json]      gegen rules.toml pruefen (Exit 1 bei hartem Verstoss)
  {toleranz} <layout> [--json]   bis zu welchem Messfehler das Verdikt haelt
  {sonne} <layout> [--json]      wann im Jahr welches Stueck in der Sonne steht
  {einbringung} <layout>         kommt jedes Stueck durch die Tuer und an seinen Platz
                                 [--b 120 --t 60] stattdessen ein gedachtes Stueck
  {compose} --pieces a,b,c       eine Wohnung von Grund auf stellen (Strahlsuche)
  {search} <layout> --move a,b   Positionen rastern, bis nichts mehr verletzt ist
                                 [--step 20] [--limit 6] [--band id=x0,x1,y0,y1]
                                 [--out <name>]  besten Treffer als Layout schreiben
  {plan} [layout...] [--out f]   Plaene als HTML mit Verdikt; ohne Layouts: alle
  {deklaration} [--json]         wer noch am Namen gemessen wird, und was es kostet
  {kaufen} [--json]              welcher Bedarf zuerst, und wann er erreicht ist
  {serve}                        HTTP-API fuer die Oberflaeche

Was da ist und was fehlt, steht nicht mehr hier: die Zeilen sind `capabilities/inventory`
(ISA F13). `inventory inventory`, `inventory wunsch`, `inventory import` und
`inventory vault-writeback` sind dieselben Verben unter ihrem neuen Namen.
"#,
        t = bold("interior"),
        model = bold("model"),
        roomplan = bold("roomplan"),
        placements = bold("placements"),
        layouts = bold("layouts"),
        plan = bold("plan"),
        deklaration = bold("deklaration"),
        kaufen = bold("kaufen"),
        check = bold("check"),
        toleranz = bold("toleranz"),
        einbringung = bold("einbringung"),
        sonne = bold("sonne"),
        search = bold("search"),
        compose = bold("compose"),
        serve = bold("serve"),
    );
    std::process::exit(1)
}

fn flag(argv: &[String], name: &str) -> Option<String> {
    argv.iter()
        .position(|a| a == &format!("--{name}"))
        .and_then(|i| argv.get(i + 1).cloned())
}

fn report(r: &CheckResult) {
    let status = if r.pass {
        green("BESTANDEN")
    } else {
        red("DURCHGEFALLEN")
    };
    println!("\n{}  {}\n", bold(&r.layout), status);
    if !r.hard.is_empty() {
        println!(
            "{}",
            red(&bold(&format!("  {} harte Verstoesse", r.hard.len())))
        );
        for v in &r.hard {
            println!("    {} {}  {}", red("x"), bold(&v.rule), v.message);
        }
        println!();
    }
    if !r.soft.is_empty() {
        println!(
            "{}",
            yellow(&bold(&format!("  {} weiche Warnungen", r.soft.len())))
        );
        for v in &r.soft {
            println!("    {} {}  {}", yellow("!"), bold(&v.rule), v.message);
        }
        println!();
    }
    if r.hard.is_empty() && r.soft.is_empty() {
        println!("{}\n", green("  keine Verstoesse"));
    }
    let m = &r.metrics;
    println!("{}", bold("  Flaeche"));
    println!(
        "    Raum {:.2} m²   belegt {:.2} m²   frei {:.2} m²",
        m.room_area_m2, m.occupied_area_m2, m.free_area_m2
    );
    for c in &m.corridors {
        match c.width_cm {
            Some(w) => println!("    {} → {}: {} cm", c.from, c.to, w),
            None => println!("    {} → {}: {}", c.from, c.to, red("kein Weg")),
        }
    }
    if !r.uncertainties.is_empty() {
        println!("\n{}", yellow(&bold("  gemessen? nein.")));
        for u in &r.uncertainties {
            println!("    · {} — {}", u.label, u.fields.join(", "));
        }
        println!("{}", dim("    Jede Zahl oben erbt diese Schaetzungen."));
    }
    println!();
}

fn write_layout(model: &Model, name: &str, layout: &Layout, note: &str) -> std::io::Result<()> {
    let mut s = String::new();
    s.push_str(&format!("# {note}\n#\n# Von `interior search` erzeugt, nicht von Hand gesetzt. Jede Position hat die volle\n# Raeumungspruefung bestanden; die Begruendung steht im Suchbericht daneben.\n\n"));
    s.push_str(&format!("name = \"{}\"\n\n", layout.name));
    for it in &layout.items {
        s.push_str("[[item]]\n");
        s.push_str(&format!("ref = \"{}\"\n", it.reference));
        s.push_str(&format!("x = {}\ny = {}\nrot = {}\n", it.x, it.y, it.rot));
        if let Some(sz) = it.size {
            s.push_str(&format!("size = [{}, {}]\n", sz[0], sz[1]));
        }
        s.push('\n');
    }
    std::fs::write(model.layouts_dir().join(format!("{name}.toml")), s)
}

#[tokio::main]
async fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().map(String::as_str) else {
        usage()
    };

    if cmd == "serve" {
        interior_serve().await;
        return;
    }

    let flat = match flag(&argv, "flat").map(Ok).unwrap_or_else(default_flat) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            std::process::exit(2);
        }
    };
    let model = match Model::load(&flat) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{}", red(&format!("Modell laedt nicht: {e}")));
            std::process::exit(2);
        }
    };

    match cmd {
        "roomplan" => {
            std::process::exit(roomplan_show(&model, &argv));
        }
        "placements" => {
            std::process::exit(placements_show(&model, &argv));
        }
        "model" => {
            println!("\n{}\n", bold(&model.room.flat.name));
            println!(
                "  planbar    {:.2} m²  {}",
                model.room.area_m2(),
                dim("(aus dem Polygon gerechnet)")
            );
            println!(
                "  Hoehe      {}",
                if model.room.hauptraum.hoehe == 0 {
                    red("nicht gemessen")
                } else {
                    model.room.hauptraum.hoehe.to_string()
                }
            );
            println!(
                "  Waende     {}",
                model
                    .room
                    .waende
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "  Oeffnungen {}",
                model
                    .room
                    .oeffnungen
                    .iter()
                    .map(|o| o.id.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!("  Katalog    {} Eintraege", model.catalogue.len());
            if !model.room.todo.offen.is_empty() {
                println!("\n{}", yellow(&bold("  noch zu messen")));
                for t in &model.room.todo.offen {
                    println!("    · {t}");
                }
            }
            let unc = model.uncertainties();
            if !unc.is_empty() {
                println!(
                    "\n{}",
                    yellow(&bold(&format!("  geschaetzte Masse ({})", unc.len())))
                );
                for (_, label, fields) in unc {
                    println!("    · {label} — {}", fields.join(", "));
                }
            }
            println!();
        }
        "layouts" => {
            for n in model.layout_names().unwrap_or_default() {
                println!("  {n}");
            }
        }
        "check" => {
            let Some(name) = argv.get(1) else { usage() };
            let layout = match model.load_layout(name) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            let r = match check_layout(&model, &layout) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            if argv.contains(&"--json".to_string()) {
                println!("{}", serde_json::to_string_pretty(&r).unwrap());
            } else {
                report(&r);
            }
            std::process::exit(if r.pass { 0 } else { 1 });
        }
        "einbringung" => std::process::exit(cmd_einbringung(&model, &argv)),
        "sonne" => std::process::exit(cmd_sonne(&model, &argv)),
        "deklaration" => std::process::exit(cmd_deklaration(&model, &argv)),
        "kaufen" => std::process::exit(cmd_kaufen(&model, &argv)),
        "toleranz" => {
            let Some(name) = argv.get(1) else { usage() };
            let layout = match model.load_layout(name) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            let r = match interior::toleranz::robustheit(&model, &layout) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            if argv.contains(&"--json".to_string()) {
                println!("{}", serde_json::to_string_pretty(&r).unwrap());
            } else {
                toleranz_report(&r);
            }
            std::process::exit(if r.nominal_pass { 0 } else { 1 });
        }
        "compose" => std::process::exit(cmd_compose(&model, &argv)),
        "search" => {
            let Some(name) = argv.get(1) else { usage() };
            let move_refs: Vec<String> = flag(&argv, "move")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if move_refs.is_empty() {
                eprintln!(
                    "  --move <id>[,<id>] fehlt — ohne bewegliche Moebel gibt es nichts zu suchen"
                );
                std::process::exit(1);
            }
            let mut bands: BTreeMap<String, [i32; 4]> = BTreeMap::new();
            for (i, a) in argv.iter().enumerate() {
                if a != "--band" {
                    continue;
                }
                let Some(spec) = argv.get(i + 1) else {
                    continue;
                };
                let Some((id, nums)) = spec.split_once('=') else {
                    continue;
                };
                let v: Vec<i32> = nums
                    .split(',')
                    .filter_map(|n| n.trim().parse().ok())
                    .collect();
                if v.len() == 4 {
                    bands.insert(id.into(), [v[0], v[1], v[2], v[3]]);
                }
            }
            let spec = Spec {
                move_refs,
                step: flag(&argv, "step")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(20),
                bands,
                limit: flag(&argv, "limit")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(6),
            };
            let base = match model.load_layout(name) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            let rep = match search(&model, &base, &spec) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            println!(
                "\n{}  {}\n",
                bold(&format!("Suche in \"{}\"", rep.base)),
                dim(&format!(
                    "beweglich: {} · Raster {} cm",
                    rep.moved.join(", "),
                    rep.step
                ))
            );
            if rep.hits.is_empty() {
                println!("  {}", red("keine verstossfreie Position gefunden"));
                println!(
                    "{}",
                    dim(&format!(
                        "  Das ist ein Ergebnis, kein Fehler: bei Raster {} cm existiert keine.",
                        rep.step
                    ))
                );
                println!(
                    "{}",
                    dim("  Feiner suchen (--step 10) oder ein Moebel mehr freigeben (--move).")
                );
            } else {
                for h in &rep.hits {
                    let tag = if h.soft == 0 {
                        green("0 Warnungen")
                    } else {
                        yellow(&format!("{} Warnungen", h.soft))
                    };
                    let front = if h.pareto { "★ " } else { "  " };
                    println!(
                        "  {front}{}  {}",
                        tag,
                        dim(&format!(
                            "Reserve {} · Wandkontakt {} cm · engste Route {} cm",
                            h.engste_reserve_cm
                                .map(|r| format!("{r} cm"))
                                .unwrap_or_else(|| "—".into()),
                            h.wandkontakt_cm,
                            h.bottleneck_cm
                        ))
                    );
                    println!(
                        "    {}",
                        h.places
                            .iter()
                            .map(|(k, p)| format!("{k} {},{}", p[0], p[1]))
                            .collect::<Vec<_>>()
                            .join("  ")
                    );
                }
                if let Some(out) = flag(&argv, "out") {
                    let best = &rep.hits[0];
                    let mut items = base.items.clone();
                    for it in items.iter_mut() {
                        if let Some(p) = best.places.get(&it.reference) {
                            it.x = p[0];
                            it.y = p[1];
                        }
                    }
                    let l = Layout {
                        name: out.clone(),
                        items,
                        id: out.clone(),
                    };
                    let note = format!("Bester Treffer der Suche in \"{}\": {} Warnungen, {} cm Wandkontakt, engste Route {} cm, knappste harte Reserve {}.", rep.base, best.soft, best.wandkontakt_cm, best.bottleneck_cm, best.engste_reserve_cm.map(|r| format!("{r} cm")).unwrap_or_else(|| "nicht gemessen".into()));
                    match write_layout(&model, &out, &l, &note) {
                        Ok(()) => println!(
                            "\n  {} {}",
                            green("geschrieben:"),
                            model.layouts_dir().join(format!("{out}.toml")).display()
                        ),
                        Err(e) => eprintln!("\n  {} {e}", red("Schreiben fehlgeschlagen:")),
                    }
                }
            }
            println!(
                "{}",
                dim(&format!(
                    "\n  {} Kandidaten nach Vorfilter, {} voll geprueft, {} Treffer in {:.1} s\n",
                    rep.candidates_after_filter,
                    rep.fully_checked,
                    rep.hits.len(),
                    rep.elapsed_ms as f64 / 1000.0
                ))
            );
        }
        "plan" => {
            let names: Vec<String> = argv[1..]
                .iter()
                .take_while(|a| !a.starts_with("--"))
                .cloned()
                .collect();
            let names = if names.is_empty() {
                model.layout_names().unwrap_or_default()
            } else {
                names
            };
            let mut layouts = Vec::new();
            for n in &names {
                match model.load_layout(n) {
                    Ok(l) => layouts.push(l),
                    Err(e) => {
                        eprintln!("{}", red(&e.to_string()));
                        std::process::exit(2);
                    }
                }
            }
            let html = match plan::page(&model, &layouts, plan::Herkunft::Vorschlag) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            let out = flag(&argv, "out").unwrap_or_else(|| "plaene.html".into());
            match std::fs::write(&out, &html) {
                Ok(()) => println!(
                    "  {} {out}  ({} Plaene, {:.0} KB)",
                    green("geschrieben:"),
                    layouts.len(),
                    html.len() as f64 / 1024.0
                ),
                Err(e) => {
                    eprintln!("{}", red(&format!("Schreiben fehlgeschlagen: {e}")));
                    std::process::exit(2);
                }
            }
        }
        _ => usage(),
    }
}

async fn interior_serve() {
    // Der Port kommt aus service.toml und wird als SJEL_PORT gesetzt; 8092 ist der Wert, unter
    // dem die Vorgaengerfassung erreichbar war, damit vorhandene Lesezeichen weiter stimmen.
    let port: u16 = sjel_config::env_var("SJEL_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8092);
    let flat = match default_flat() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    interior::api::serve(&flat, port).await;
}

/// Eine Wohnung von Grund auf stellen lassen.
///
/// `search` verschiebt in einem bestehenden Layout; das hier stellt eins. Der Unterschied ist
/// nicht die Groesse, sondern das Verfahren: das volle Produkt ueber sechs Stuecke waere rund
/// 10^15 Kombinationen, also laeuft eine Strahlsuche Stueck fuer Stueck. Was sie findet, ist
/// gerechnet und wiederholbar; das globale Optimum ist es nicht garantiert, und `search.rs`
/// sagt warum.
fn cmd_compose(model: &Model, argv: &[String]) -> i32 {
    let refs: Vec<String> = match flag(argv, "pieces") {
        Some(v) => v.split(',').map(|s| s.trim().to_string()).collect(),
        None => {
            eprintln!(
                "{}",
                red("--pieces fehlt: interior compose --pieces a,b,c [--step 25] [--beam 60] [--limit 5] [--out name]")
            );
            return 2;
        }
    };
    let spec = interior::search::ComposeSpec {
        refs: refs.clone(),
        step: flag(argv, "step")
            .and_then(|v| v.parse().ok())
            .unwrap_or(25),
        beam: flag(argv, "beam")
            .and_then(|v| v.parse().ok())
            .unwrap_or(60),
        rotations: flag(argv, "rot")
            .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
            .unwrap_or_else(|| vec![0, 90]),
        limit: flag(argv, "limit")
            .and_then(|v| v.parse().ok())
            .unwrap_or(5),
    };
    let t0 = std::time::Instant::now();
    let out = match interior::search::compose(model, &spec) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    println!(
        "\n{}  {} Stuecke, Schritt {} cm, Strahl {}, {:.1} s\n",
        bold("compose"),
        refs.len(),
        spec.step,
        spec.beam,
        t0.elapsed().as_secs_f64()
    );
    if out.is_empty() {
        println!("{}\n", red("  keine vollstaendige Aufstellung gefunden"));
        return 1;
    }
    for (n, c) in out.iter().enumerate() {
        let status = if c.pass {
            green("BESTANDEN")
        } else {
            red("DURCHGEFALLEN")
        };
        println!(
            "  {} {}{}  {} weiche, Reserve {}, Engpass {} cm, Wand {} cm, frei {:.2} m²",
            bold(&format!("#{}", n + 1)),
            if c.pareto { "★ " } else { "" },
            status,
            c.soft.len(),
            c.engste_reserve_cm
                .map(|r| format!("{r} cm"))
                .unwrap_or_else(|| "—".into()),
            c.bottleneck_cm,
            c.wandkontakt_cm,
            c.free_m2
        );
        if !c.hard.is_empty() {
            println!("      {} {}", red("hart:"), c.hard.join(", "));
        }
        if !c.soft.is_empty() {
            println!("      {} {}", yellow("weich:"), c.soft.join(", "));
        }
        for (r, p, rot) in &c.places {
            println!("      {r:26} x={:<4} y={:<4} rot={rot}", p[0], p[1]);
        }
        println!();
    }
    // Nur der beste, und nur auf Verlangen: eine Suche, die ungefragt Dateien anlegt, ist eine
    // Suche, der man beim Ausprobieren nicht trauen kann.
    if let Some(name) = flag(argv, "out") {
        // Welchen Rang schreiben. Standard ist der erste, aber die Rangfolge kennt keine Regel
        // gegen frei stehende Moebel — sie hat nur `wandkontakt_cm` als Nachrang. Wer die Liste
        // gelesen hat, darf einen anderen nehmen, ohne die Suche neu zu starten.
        let rang: usize = flag(argv, "rank").and_then(|v| v.parse().ok()).unwrap_or(1);
        let Some(best) = out.get(rang.saturating_sub(1)) else {
            eprintln!(
                "{}",
                red(&format!(
                    "--rank {rang} gibt es nicht, es sind {}",
                    out.len()
                ))
            );
            return 2;
        };
        let layout = Layout {
            name: format!("{name} (compose)"),
            id: name.clone(),
            items: best
                .places
                .iter()
                .map(|(r, p, rot)| interior::model::PlacedItem {
                    reference: r.clone(),
                    x: p[0],
                    y: p[1],
                    rot: *rot,
                    size: None,
                    kind: None,
                })
                .collect(),
        };
        // Der Satz ueber die Raeumungspruefung steht hier und nicht mehr in `layout_io`: er ist
        // wahr fuer `compose`, das jede Position geprueft hat, und falsch fuer einen leeren
        // Plan, den die API anlegt.
        let notiz = format!(
            "Von `interior compose` gestellt am {}: {} Stuecke, Schritt {} cm, Strahl {}.\n\
             \n\
             Von einer Maschine gesetzt, nicht von Hand. Jede Position hat die volle\n\
             Raeumungspruefung durchlaufen; was sie ergeben hat, steht im Verdikt daneben und\n\
             nicht hier.",
            interior::layout_io::heute(),
            refs.len(),
            spec.step,
            spec.beam
        );
        match interior::layout_io::create(model, &name, &layout, &notiz) {
            Ok(()) => println!("  {} {name}\n", green("geschrieben:")),
            Err(e) => {
                eprintln!("{}", red(&e.to_string()));
                return 2;
            }
        }
    }
    0
}

/// Was ein Verdikt aushaelt, in Worten.
fn toleranz_report(r: &interior::toleranz::Robustheit) {
    use interior::toleranz::Haltbarkeit;
    println!("\n{}", bold(&r.layout));
    match &r.haelt {
        Haltbarkeit::FaelltDurch => println!(
            "  {}",
            red("faellt schon bei den eingetragenen Massen durch — ein Messfehler ist hier nicht die Frage")
        ),
        Haltbarkeit::NichtsGeraten => println!(
            "  {}",
            dim("kein Mass in diesem Layout ist als geschaetzt gefuehrt — es gibt nichts zu variieren")
        ),
        Haltbarkeit::UeberHorizont { horizont_cm } => println!(
            "  {}",
            green(&format!(
                "haelt auch {horizont_cm} cm Messfehler noch aus (weiter wurde nicht gesucht)"
            ))
        ),
        Haltbarkeit::Bis { cm } => {
            println!("  {}", green(&format!("haelt bis {cm} cm Messfehler")));
            println!(
                "  {}",
                yellow(&format!(
                    "bei {} cm reisst zuerst: {}",
                    cm + 1,
                    r.kippt_an.join(", ")
                ))
            );
        }
    }
    if let Some(cm) = r.engste_reserve_cm {
        println!("  {}", dim(&format!("knappste harte Messung: {cm} cm")));
    }
    for u in &r.geraten {
        println!(
            "  {}",
            dim(&format!(
                "geschaetzt: {} — {}",
                u.label,
                u.fields.join(", ")
            ))
        );
    }
    for n in &r.nicht_variiert {
        println!(
            "  {}",
            dim(&format!(
                "`{n}` traegt sein Mass im Layout (size:) und wurde nicht variiert"
            ))
        );
    }
    println!();
}

/// Kommt es herein? Fuer jedes Stueck eines Layouts, oder fuer ein gedachtes.
fn cmd_einbringung(model: &Model, argv: &[String]) -> i32 {
    use interior::einbringung::{
        durch_die_tuer, einbringung, weg_zum_platz, Einbringung, Tuerpass,
    };

    let zeile = |e: &Einbringung| {
        let tuer = match &e.tuer {
            Tuerpass::Passt { luft_cm, tuer_cm } => {
                green(&format!("durch die {tuer_cm} cm Tuer, {luft_cm} cm Luft"))
            }
            Tuerpass::PasstNicht { fehlen_cm, tuer_cm } => red(&format!(
                "passt NICHT durch die {tuer_cm} cm Tuer — {fehlen_cm} cm zu breit"
            )),
            Tuerpass::ZerlegtGetragen { fehlen_cm, tuer_cm } => yellow(&format!(
                "{fehlen_cm} cm breiter als die {tuer_cm} cm Tuer, kommt laut Zeile zerlegt herein"
            )),
            Tuerpass::KeinEingang => yellow("keine Tuer als Eingang deklariert"),
        };
        let weg = if e.erreichbar {
            green(&format!(
                "Weg frei ({} Schritte{})",
                e.schritte.unwrap_or(0),
                if e.dreht { ", mit Drehung" } else { "" }
            ))
        } else {
            red("kein Weg bis zum Platz")
        };
        println!(
            "  {:26} {:>3}×{:<3}  {tuer}  {weg}",
            bold(&e.reference),
            e.b,
            e.t
        );
        if let Some(g) = &e.grund {
            println!("      {}", dim(g));
        }
    };

    // Ein gedachtes Stueck: die Frage vor dem Kauf, ohne dass es schon irgendwo steht.
    if let (Some(b), Some(t)) = (
        flag(argv, "b").and_then(|v| v.parse::<i32>().ok()),
        flag(argv, "t").and_then(|v| v.parse::<i32>().ok()),
    ) {
        let zerlegbar = argv.contains(&"--zerlegbar".to_string());
        println!("\n{}  {b}×{t} cm\n", bold("einbringung"));
        match durch_die_tuer(model, b, t, zerlegbar) {
            Tuerpass::Passt { luft_cm, tuer_cm } => {
                println!(
                    "  {}",
                    green(&format!(
                        "passt hochkant durch die {tuer_cm} cm Tuer, {luft_cm} cm Luft"
                    ))
                );
                0
            }
            Tuerpass::PasstNicht { fehlen_cm, tuer_cm } => {
                println!(
                    "  {}",
                    red(&format!(
                        "passt nicht: die schmale Seite ist {fehlen_cm} cm breiter als die {tuer_cm} cm Tuer"
                    ))
                );
                println!(
                    "  {}",
                    dim(
                        "kommt es zerlegt? dann `--zerlegbar`, und in der Zeile `zerlegbar = true`"
                    )
                );
                1
            }
            Tuerpass::ZerlegtGetragen { fehlen_cm, tuer_cm } => {
                println!(
                    "  {}",
                    yellow(&format!(
                        "{fehlen_cm} cm breiter als die {tuer_cm} cm Tuer — zerlegt erklaert, also herein"
                    ))
                );
                0
            }
            Tuerpass::KeinEingang => {
                println!(
                    "  {}",
                    yellow("keine Oeffnung ist als Eingang deklariert (`eingang = true`)")
                );
                1
            }
        }
    } else {
        let Some(name) = argv.get(1) else { usage() };
        let layout = match model.load_layout(name) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{}", red(&e.to_string()));
                return 2;
            }
        };
        println!("\n{}  \"{}\"\n", bold("einbringung"), layout.name);
        let mut schlecht = 0;
        for it in &layout.items {
            match einbringung(model, &layout, &it.reference) {
                Ok(e) => {
                    if !e.erreichbar || matches!(e.tuer, Tuerpass::PasstNicht { .. }) {
                        // `ZerlegtGetragen` zaehlt hier bewusst nicht: die Wohnung hat eine
                        // Antwort darauf, und ein Exit-Code, der bei einer beantworteten
                        // Frage rot wird, wird abgeschaltet.
                        schlecht += 1;
                    }
                    zeile(&e);
                }
                Err(e) => eprintln!("  {} {e}", red(&it.reference)),
            }
        }
        // Verhindert, dass `weg_zum_platz` als tote Ausfuhr gilt, und ist die Zeile, die den
        // Unterschied zwischen "steht dort" und "kommt dorthin" ueberhaupt sichtbar macht.
        let _ = weg_zum_platz;
        println!();
        i32::from(schlecht > 0)
    }
}

/// Wann im Jahr welches Stueck in der Sonne steht.
fn cmd_sonne(model: &Model, argv: &[String]) -> i32 {
    let Some(name) = argv.get(1) else { usage() };
    let layout = match model.load_layout(name) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    let b = match interior::sonne::bericht(model, &layout) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    if argv.contains(&"--json".to_string()) {
        println!("{}", serde_json::to_string_pretty(&b).unwrap());
        return 0;
    }

    println!("\n{}  \"{}\"\n", bold("sonne"), b.layout);
    // Eine Zeile je Tag, eine Spalte je Stunde. Ein Jahr passt so auf vier Zeilen, und die
    // Form zeigt, was eine Liste verbirgt: die Sonne wandert.
    for (tag, _) in interior::sonne::TAGE {
        let stunden: Vec<&interior::sonne::Sonnenstunde> =
            b.stunden.iter().filter(|s| s.tag == *tag).collect();
        let kopf: String = stunden
            .iter()
            .map(|s| format!("{:>3}", s.stunde_lokal))
            .collect();
        let zeile: String = stunden
            .iter()
            .map(|s| {
                if s.hoehe_grad <= 0.0 {
                    "  ·".to_string()
                } else if s.getroffen.is_empty() {
                    "  -".to_string()
                } else {
                    format!("{:>3}", s.getroffen.len())
                }
            })
            .collect();
        // Erst auffuellen, dann faerben: die Escape-Sequenzen zaehlen sonst als Breite mit,
        // und die Spalten wandern je nach Laenge des Tagesnamens.
        println!("  {}{}", bold(&format!("{tag:<16}")), dim(&kopf));
        println!("  {:<16}{}", "", yellow(&zeile));
    }
    println!(
        "\n  {}",
        dim("· unter dem Horizont · - kein Stueck in der Sonne · Zahl = getroffene Stuecke")
    );
    if !b.treffer_je_stueck.is_empty() {
        println!("\n  {}", bold("Stunden in direkter Sonne"));
        for (k, n) in &b.treffer_je_stueck {
            println!(
                "    {k:26} {n:>3} von {}",
                interior::sonne::gepruefte_stunden()
            );
        }
    }
    for o in &b.ohne_glashoehen {
        println!(
            "  {}",
            yellow(&format!(
                "`{o}` hat keine Glashoehen (glas_von_cm/glas_bis_cm) und wirft hier kein Licht"
            ))
        );
    }
    println!();
    0
}

/// Wer wird noch am Namen gemessen — und was aendert sich, wenn er sich erklaert.
fn cmd_deklaration(model: &Model, argv: &[String]) -> i32 {
    let stand = match interior::deklaration::uebersicht(model) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    if argv.contains(&"--json".to_string()) {
        println!("{}", serde_json::to_string_pretty(&stand).unwrap());
        return 0;
    }

    let erklaert = stand.iter().filter(|s| s.erklaert).count();
    println!(
        "\n{}  {} von {} Eintraegen erklaeren sich selbst\n",
        bold("deklaration"),
        erklaert,
        stand.len()
    );
    for s in &stand {
        if s.erklaert {
            println!("  {} {}", green("erklaert "), bold(&s.id));
            continue;
        }
        let Some(v) = &s.vorschlag else {
            println!(
                "  {} {}  {}",
                dim("am Namen"),
                bold(&s.id),
                dim(&format!(
                    "als {} eingestuft, und daraus folgt keine Schwelle",
                    s.geraten_als
                ))
            );
            continue;
        };
        let f = s.folgen.as_ref();
        let wirkung = match f {
            Some(f) if f.geaendert.is_empty() => green("aendert kein Verdikt"),
            Some(f) => yellow(&format!("aendert: {}", f.geaendert.join(", "))),
            None => dim("nicht gerechnet"),
        };
        println!(
            "  {} {}  {}  {wirkung}",
            yellow("am Namen"),
            bold(&s.id),
            dim(&format!("als {} eingestuft", s.geraten_als))
        );
        for zeile in v.toml.lines() {
            println!("      {}", dim(zeile));
        }
        if !s.in_layouts.is_empty() {
            println!(
                "      {}",
                dim(&format!("steht in {} Layouts", s.in_layouts.len()))
            );
        }
    }
    println!();
    0
}

/// Welcher Bedarf zuerst, und wann er erreicht ist.
fn cmd_kaufen(model: &Model, argv: &[String]) -> i32 {
    // Ohne Saldo laeuft die Reihenfolge trotzdem: sie ist eine Ordnung und keine Zeitachse,
    // und die Zeitachse ist der Zusatz. Ein `finance`, das hier nichts geschrieben hat, ist
    // kein Grund, die Frage nach dem Was-zuerst unbeantwortet zu lassen.
    let saldo = interior::store::Store::open(&sjel_config::database_path())
        .ok()
        .and_then(|s| s.borrow_connection().ok())
        .and_then(|conn| interior::budget::monatssaldo(&conn).ok().flatten());
    let r = match interior::budget::kaufreihenfolge(model, saldo) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    if argv.contains(&"--json".to_string()) {
        println!("{}", serde_json::to_string_pretty(&r).unwrap());
        return 0;
    }

    println!("\n{}\n", bold("Kaufreihenfolge"));
    if r.posten.is_empty() {
        println!("  {}\n", dim("kein Bedarf mit Preis offen"));
    }
    for (n, p) in r.posten.iter().enumerate() {
        // Zwei Gruende fuer keine Monatszahl, und sie duerfen nicht dasselbe Wort bekommen:
        // „es gibt keinen Saldo" ist eine fehlende Messung, „der Saldo ist nicht positiv" ist
        // eine Antwort. Die erste Fassung schrieb beidem „ohne Saldo" hin, an einer Wohnung,
        // deren Median gemessen -269,98 € betraegt.
        let wann = match (p.erreichbar_nach_monaten, &r.saldo) {
            (Some(m), _) => green(&format!("nach {m:.1} Monaten")),
            (None, Some(_)) => dim("aus diesem Saldo nicht ansparbar"),
            (None, None) => dim("kein Monatssaldo gemessen"),
        };
        println!(
            "  {:>2}. {:28} {:>9}  {:>10} kumuliert  {wann}",
            n + 1,
            bold(&p.label),
            euro(p.preis_cent.unwrap_or(0)),
            euro(p.kumuliert_cent)
        );
        if !p.in_layouts.is_empty() {
            println!(
                "      {}",
                dim(&format!("eingeplant in {}", p.in_layouts.join(", ")))
            );
        }
    }
    if !r.ohne_preis.is_empty() {
        println!(
            "\n  {}",
            yellow(&format!(
                "{} Bedarfe ohne Preis — sie stehen in keiner Reihenfolge",
                r.ohne_preis.len()
            ))
        );
        for p in &r.ohne_preis {
            println!("      {}", dim(&p.label));
        }
    }
    if !r.unbekannte_prioritaeten.is_empty() {
        println!(
            "\n  {}",
            yellow(&format!(
                "unbekannte Prioritaet, hinten einsortiert und hier genannt: {}",
                r.unbekannte_prioritaeten.join(", ")
            ))
        );
    }
    match &r.saldo {
        Some(s) if s.median_cent > 0 => println!(
            "\n  {}",
            dim(&format!(
                "Median des Monatssaldos {} ueber {} Monate ({} bis {})",
                euro(s.median_cent),
                s.monate,
                s.von,
                s.bis
            ))
        ),
        Some(s) => println!(
            "\n  {}",
            yellow(&format!(
                "Median des Monatssaldos {} — daraus laesst sich nichts ansparen, also gibt es keine Monatszahl",
                euro(s.median_cent)
            ))
        ),
        None => println!("\n  {}", dim("kein Monatssaldo verfuegbar (finance hat hier nichts geschrieben)")),
    }
    println!();
    0
}

/// Was in einer RoomPlan-Aufnahme steht, und wie sie sich zu `room.toml` verhaelt.
///
/// Schreibt nichts, solange `--write` nicht dasteht. Ein Abgleich, der von selbst schreibt,
/// waere der Scan als Messung — und die Regel dieses Projekts ist die andere.
///
/// Zwischen den Oeffnungen des Scans und denen aus `room.toml` wird **keine Identitaet
/// behauptet**. Beide Listen stehen nebeneinander, und die Zuordnung ist eine Entscheidung.
fn roomplan_show(model: &Model, argv: &[String]) -> i32 {
    use interior::roomplan;
    let flat = model.room.flat.id.clone();
    let verzeichnis = match flag(argv, "capture") {
        Some(datum) => match roomplan::captures_root(&flat) {
            Ok(root) => root.join(datum),
            Err(e) => {
                eprintln!("{}", red(&e.to_string()));
                return 2;
            }
        },
        None => match roomplan::latest_capture(&flat) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}", red(&e.to_string()));
                return 2;
            }
        },
    };
    let scan = match roomplan::read_usdz(&verzeichnis.join("captured-room.usdz")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    let d = roomplan::vergleiche(&scan, model);

    if argv.iter().any(|a| a == "--json") {
        let inhalt = serde_json::json!({
            "capture": verzeichnis.display().to_string(),
            "sha256": scan.sha256,
            "byte_length": scan.byte_length,
            "mesh_assets": scan.mesh_assets,
            "room_groups": scan.room_groups,
            "capture_finished_at": scan.capture_finished_at,
            "kategorien": scan.kategorien(),
            "scan_wandhoehe_cm": d.scan_wandhoehe_cm,
            "scan_wandstaerke_cm": scan.wandstaerke_m().map(|m| m * 100.0),
            "modell_hoehe_cm": d.modell_hoehe_cm,
            "scan_wandlaenge_cm": d.scan_wandlaenge_cm,
            "scan_oeffnungen": d.scan_oeffnungen.iter().map(|(id, cat, b, h)| serde_json::json!({
                "id": id, "category": cat, "breite_cm": b, "hoehe_cm": h
            })).collect::<Vec<_>>(),
            "modell_oeffnungen": d.modell_oeffnungen.iter().map(|(id, b)| serde_json::json!({
                "id": id, "breite_cm": b
            })).collect::<Vec<_>>(),
            "scan_grundflaeche_m2": d.scan_grundflaeche_m2,
            "modell_innenflaeche_m2": d.modell_innenflaeche_m2,
            "scan_objekte": d.scan_objekte.iter().map(|(n, m)| serde_json::json!({
                "element": n, "cm": m
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&inhalt).unwrap_or_default()
        );
        return 0;
    }

    println!("\n{}\n", bold("RoomPlan-Aufnahme"));
    println!("  Aufnahme   {}", verzeichnis.display());
    println!(
        "  USDZ       {} Bytes, sha256 {}",
        scan.byte_length,
        &scan.sha256[..16.min(scan.sha256.len())]
    );
    println!(
        "  Inhalt     {} Maschen, {} Raumgruppen {}",
        scan.mesh_assets,
        scan.room_groups,
        dim("(die Gruppen nennen Mittelpunkte und keine Mitgliedschaft)")
    );
    println!(
        "  Aufnahme   {}",
        match &scan.capture_finished_at {
            Some(t) => format!("{t} {}", dim("(aus dem Archiv, ohne Zone)")),
            None => dim("ohne Zeitstempel").to_string(),
        }
    );

    println!("\n  {}", bold("Wandhoehe"));
    match d.scan_wandhoehe_cm {
        Some(cm) => {
            println!("    {:.2} cm   {}", cm, dim("alle Waende einig"));
            if d.modell_hoehe_cm == 0 {
                println!(
                    "    room.toml fuehrt 0 {} — {}",
                    dim("= nicht gemessen"),
                    green("diese offene Frage beantwortet der Scan")
                );
            } else {
                println!(
                    "    room.toml fuehrt {} cm   Abweichung {:+.2} cm",
                    d.modell_hoehe_cm,
                    cm - d.modell_hoehe_cm as f64
                );
            }
        }
        None => println!(
            "    {}",
            red("die Waende widersprechen sich, also keine Zahl")
        ),
    }
    match scan.wandstaerke_m() {
        Some(m) => println!(
            "  Wandstaerke {:.2} cm   {}",
            m * 100.0,
            dim("alle gleich, also eine Konstante und keine Messung — geht nicht in den Vergleich ein")
        ),
        None => println!(
            "  Wandstaerke {}",
            dim("die Waende unterscheiden sich, also gemessen")
        ),
    }
    println!(
        "  Waende      {}, Gesamtlaenge {:.1} cm",
        scan.flaechen().filter(|e| e.category == "Wall").count(),
        d.scan_wandlaenge_cm
    );
    println!(
        "              {}",
        dim("der Scan trennt Waende an ihren Oeffnungen, room.toml nicht")
    );

    println!("\n  {}", bold("Oeffnungen im Scan"));
    for (id, cat, b, h) in &d.scan_oeffnungen {
        println!("    {:<9} {:<24} {:>7.1} x{:>6.1} cm", id, cat, b, h);
    }
    println!("\n  {}", bold("Oeffnungen in room.toml"));
    for (id, b) in &d.modell_oeffnungen {
        println!("    {:<26} breit {:>5} cm", id, b);
    }
    println!(
        "\n    {}",
        yellow("zwischen den Listen wird keine Identitaet behauptet:")
    );
    println!(
        "    {}",
        dim("welcher Scan-Eintrag welche Oeffnung ist, ist eine Entscheidung und keine Rechnung")
    );

    println!("\n  {}", bold("Bodenflaeche"));
    match d.scan_grundflaeche_m2 {
        Some(m2) => {
            let delta = m2 - d.modell_innenflaeche_m2;
            let prozent = if d.modell_innenflaeche_m2 > 0.0 {
                delta / d.modell_innenflaeche_m2 * 100.0
            } else {
                0.0
            };
            println!(
                "    Scan        {:.2} m²  {}",
                m2,
                dim("obere Seite der Bodenplatte, aus ihren Dreiecken")
            );
            println!(
                "    room.toml   {:.2} m²  {}",
                d.modell_innenflaeche_m2,
                dim("Hauptraum + Bad, aus dem Polygon gerechnet")
            );
            println!("    Abweichung  {:+.2} m²  ({:+.1} %)", delta, prozent);
        }
        None => println!("    {}", dim("der Scan traegt keine Bodenplatte")),
    }

    println!("\n  {}", bold("Moebel und Geraete im Scan"));
    for (name, m) in &d.scan_objekte {
        println!(
            "    {:<34} {:>6.1} x{:>6.1} x{:>6.1} cm",
            name, m[0], m[1], m[2]
        );
    }

    if argv.iter().any(|a| a == "--write") {
        match roomplan::write_draft(&scan, &flat, &verzeichnis, &civil_date::today()) {
            Ok(pfad) => println!("\n  {} {}", green("Entwurf geschrieben:"), pfad.display()),
            Err(e) => {
                eprintln!("{}", red(&format!("Entwurf nicht geschrieben: {e}")));
                return 1;
            }
        }
    } else {
        println!(
            "\n  {}",
            dim("nichts geschrieben. Mit --write entsteht der Entwurf als draft.json neben der Aufnahme.")
        );
    }
    println!();
    0
}

/// Wo die Stuecke WIRKLICH stehen — und was noch keinen Platz hat.
///
/// `interior_placement` stand seit B25 leer, weil nichts sie schrieb. Das hier ist der Weg
/// hinein: `--set bett_bestand=x,y[,rot]` schreibt eine Position. Ohne `--set` wird nur gelesen,
/// und ohne Fundstelle wird nicht geschrieben. Eine Platzierung ist eine Zeile und kein
/// Vorschlag — das ist der Unterschied, den die Tabelle traegt.
///
/// `--plan <datei>` legt einen Plan des Ist-Zustands daneben. Er wird aus denselben
/// [`PlacedItem`]-Zeilen gebaut wie ein Layout, damit die Zeichnung nicht zwei Fassungen
/// desselben Stuecks kennt: ein Stueck an einer Stelle ist dieselbe Form, ob es nun steht oder
/// vorgeschlagen ist, und nur der Zustand unterscheidet die beiden.
fn placements_show(model: &Model, argv: &[String]) -> i32 {
    let flat = model.room.flat.id.clone();
    let st = match interior::store::Store::open(&sjel_config::database_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&format!("Datenbank nicht erreichbar: {e}")));
            return 2;
        }
    };

    // Erst schreiben, dann lesen: die Ausgabe soll den Zustand NACH den Argumenten zeigen und
    // nicht den davor. Ein Bericht, der die Zeile noch nicht kennt, die gerade geschrieben wurde,
    // liest sich wie ein Schreibfehler.
    let mut geschrieben = 0usize;
    let mut i = 0usize;
    while i < argv.len() {
        if argv[i] == "--set" {
            let Some(roh) = argv.get(i + 1) else {
                eprintln!("{}", red("--set braucht id=x,y[,rot]"));
                return 2;
            };
            let Some((id, x, y, rot)) = parse_platzierung(roh) else {
                eprintln!("{}", red(&format!("--set {roh} ist nicht id=x,y[,rot]")));
                return 2;
            };
            if !model.catalogue.contains_key(&id) {
                eprintln!(
                    "{}",
                    red(&format!(
                        "`{id}` steht nicht im Katalog — nichts geschrieben"
                    ))
                );
                return 2;
            }
            if let Err(e) = st.place(&interior::store::Placement {
                item_id: id,
                flat: flat.clone(),
                x,
                y,
                rot,
            }) {
                eprintln!("{}", red(&format!("nicht geschrieben: {e}")));
                return 2;
            }
            geschrieben += 1;
            i += 2;
            continue;
        }
        i += 1;
    }

    let platziert = match st.placements(&flat) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", red(&format!("Platzierungen laden nicht: {e}")));
            return 2;
        }
    };
    let nach_id: std::collections::BTreeMap<&str, &interior::store::Placement> =
        platziert.iter().map(|p| (p.item_id.as_str(), p)).collect();

    // Nur Besitz: ein Wunsch hat keinen Platz, er hat eine Begruendung.
    let mut besessen: Vec<(&String, &interior::store::Item)> = model
        .catalogue
        .iter()
        .filter(|(id, _)| model.states.get(*id).copied() == Some(interior::store::State::Owned))
        .collect();
    besessen.sort_by(|a, b| a.0.cmp(b.0));

    let mut gesetzt: Vec<(&String, &interior::store::Placement)> = Vec::new();
    let mut ohne: Vec<(&String, &interior::store::Item)> = Vec::new();
    for (id, item) in &besessen {
        match nach_id.get(id.as_str()) {
            Some(p) => gesetzt.push((id, p)),
            None => ohne.push((id, item)),
        }
    }
    // Eine Platzierung fuer ein Stueck, das nicht mehr im Besitz ist, ist keine Kleinigkeit —
    // sie zeigt auf etwas, das nicht mehr da ist, und keine Liste oben haette sie genannt.
    let verwaist: Vec<&&str> = nach_id
        .iter()
        .filter(|(id, _)| !besessen.iter().any(|(b, _)| b.as_str() == **id))
        .map(|(id, _)| id)
        .collect();

    if argv.iter().any(|a| a == "--json") {
        let wert = serde_json::json!({
            "flat": flat,
            "geschrieben": geschrieben,
            "besessen": besessen.len(),
            "platziert": gesetzt.iter().map(|(id, p)| serde_json::json!({
                "id": id, "x": p.x, "y": p.y, "rot": p.rot
            })).collect::<Vec<_>>(),
            "ohne_platz": ohne.iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>(),
            "verwaist": verwaist,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&wert).unwrap_or_default()
        );
        return 0;
    }

    println!("\n{}\n", bold("Ist-Zustand"));
    println!(
        "  besessen   {}   {}",
        besessen.len(),
        dim("Plaetze sind eine Zeile je Stueck; ein Layout ist ein Vorschlag und eine Datei")
    );
    println!("  gesetzt    {}", bold(&gesetzt.len().to_string()));
    for (id, p) in &gesetzt {
        println!(
            "    {:<28} x={:<5} y={:<5} rot={:<4} {}",
            id,
            p.x,
            p.y,
            p.rot,
            dim(&model.catalogue[*id].label)
        );
    }
    println!(
        "\n  {} {}",
        bold(&format!("ohne Platz {}", ohne.len())),
        dim("(davon weiss der Plan noch nichts)")
    );
    for (id, item) in &ohne {
        println!("    {:<28} {}", id, dim(&item.label));
    }
    if !verwaist.is_empty() {
        println!(
            "\n  {}",
            yellow(&format!(
                "Platzierungen ohne Stueck im Besitz: {} — sie zeigen auf etwas, das nicht mehr da ist",
                verwaist.iter().map(|s| **s).collect::<Vec<_>>().join(", ")
            ))
        );
    }

    if let Some(datei) = flag(argv, "plan") {
        let layout = interior::model::Layout {
            name: "ist-zustand".to_string(),
            id: "ist-zustand".to_string(),
            items: gesetzt
                .iter()
                .map(|(id, p)| interior::model::PlacedItem {
                    reference: (*id).clone(),
                    x: p.x,
                    y: p.y,
                    rot: p.rot,
                    size: None,
                    kind: None,
                })
                .collect(),
        };
        match plan::page(
            model,
            std::slice::from_ref(&layout),
            plan::Herkunft::IstZustand,
        ) {
            Ok(html) => match std::fs::write(&datei, &html) {
                Ok(()) => println!("\n  {} {}", green("Ist-Plan geschrieben:"), datei),
                Err(e) => {
                    eprintln!("{}", red(&format!("{datei}: {e}")));
                    return 1;
                }
            },
            Err(e) => {
                eprintln!("{}", red(&format!("Plan nicht gebaut: {e}")));
                return 1;
            }
        }
    }
    println!();
    0
}

/// `id=x,y` oder `id=x,y,rot`. Die Kennung darf kein `=` enthalten, weil `=` das Feld trennt.
fn parse_platzierung(roh: &str) -> Option<(String, i32, i32, i32)> {
    let (id, rest) = roh.split_once('=')?;
    if id.is_empty() {
        return None;
    }
    let teile: Vec<&str> = rest.split(',').map(str::trim).collect();
    if teile.len() < 2 || teile.len() > 3 {
        return None;
    }
    let x = teile[0].parse::<i32>().ok()?;
    let y = teile[1].parse::<i32>().ok()?;
    let rot = match teile.get(2) {
        Some(r) => r.parse::<i32>().ok()?,
        None => 0,
    };
    Some((id.to_string(), x, y, rot))
}

/// Cent als Euro mit Komma. Die einzige Rechnung, die eine Anzeige fuehren darf.
fn euro(cent: i64) -> String {
    format!("{},{:02} €", cent / 100, (cent % 100).abs())
}
