//! interior: room planning.
//!
//! The flat is `SJEL_INTERIOR_FLAT` or `--flat`, else the only one under `data/interior/flats/`;
//! no flat name is in the source (`tests/containment.rs`). Reads the measured model from the private
//! overlay, checks layouts against the clearance rules and searches positions that meet them.
//! `check` exits with an error on a hard violation, so it works as a gate. `search --out` writes the
//! best hit as a real layout, so results outlive the session.

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
{t}: room planning

  {model}                        the measured room, and what in it is still guessed
  {roomplan} [--capture D]       what the RoomPlan capture holds, against room.toml
                                 [--write]  the draft as draft.json next to the capture
                                 [--json]   the same as JSON
  {placements} [--set id=x,y[,r]]  where the pieces REALLY stand, and what has no place yet
                                 [--plan f] [--json]
  {layouts}                      layouts on disk
  {check} <layout> [--json]      check against rules.toml (exit 1 on a hard violation)
  {tolerance} <layout> [--json]  up to which measurement error the verdict holds
  {sun} <layout> [--json]        when in the year which piece is in the sun
  {delivery} <layout>            does every piece get through the door to its place
                                 [--width 120 --depth 60] an imagined piece instead
  {compose} --pieces a,b,c       furnish a flat from scratch (beam search)
  {search} <layout> --move a,b   step positions until nothing is violated
                                 [--step 20] [--limit 6] [--band id=x0,x1,y0,y1]
                                 [--out <name>]  write the best hit as a layout
  {plan} [layout...] [--out f]   plans as HTML with verdicts; without layouts: all
  {declarations} [--json]        which items are still judged by name, and what it costs
  {purchase_order_cmd} [--json]  which need first, and when it is affordable
  {serve}                        HTTP API for the dashboard

What is owned and wanted lives in `capabilities/inventory`: `inventory inventory`,
`inventory add` and `inventory import`.
"#,
        t = bold("interior"),
        model = bold("model"),
        roomplan = bold("roomplan"),
        placements = bold("placements"),
        layouts = bold("layouts"),
        plan = bold("plan"),
        declarations = bold("declarations"),
        purchase_order_cmd = bold("purchase-order"),
        check = bold("check"),
        tolerance = bold("tolerance"),
        delivery = bold("delivery"),
        sun = bold("sun"),
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
            red(&bold(&format!("  {} hard violations", r.hard.len())))
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
        println!("{}\n", green("  no violations"));
    }
    let m = &r.metrics;
    println!("{}", bold("  Area"));
    println!(
        "    room {:.2} m²   occupied {:.2} m²   free {:.2} m²",
        m.room_area_m2, m.occupied_area_m2, m.free_area_m2
    );
    for c in &m.corridors {
        match c.width_cm {
            Some(w) => println!("    {} → {}: {} cm", c.from, c.to, w),
            None => println!("    {} → {}: {}", c.from, c.to, red("no path")),
        }
    }
    if !r.uncertainties.is_empty() {
        println!("\n{}", yellow(&bold("  measured? no.")));
        for u in &r.uncertainties {
            println!("    · {} — {}", u.label, u.fields.join(", "));
        }
        println!(
            "{}",
            dim("    Every number above inherits these estimates.")
        );
    }
    println!();
}

fn write_layout(model: &Model, name: &str, layout: &Layout, note: &str) -> std::io::Result<()> {
    let mut s = String::new();
    s.push_str(&format!("# {note}\n#\n# Generated by `interior search`, not placed by hand. Every position passed the full\n# clearance check; the reasoning is in the search report next to it.\n\n"));
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
            eprintln!("{}", red(&format!("model does not load: {e}")));
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
                dim("(computed from the polygon)")
            );
            println!(
                "  Height     {}",
                if model.room.main_room.height == 0 {
                    red("not measured")
                } else {
                    model.room.main_room.height.to_string()
                }
            );
            println!(
                "  Walls      {}",
                model
                    .room
                    .walls
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "  Oeffnungen {}",
                model
                    .room
                    .openings
                    .iter()
                    .map(|o| o.id.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!("  Catalogue  {} entries", model.catalogue.len());
            if !model.room.todo.open.is_empty() {
                println!("\n{}", yellow(&bold("  still to measure")));
                for t in &model.room.todo.open {
                    println!("    · {t}");
                }
            }
            let unc = model.uncertainties();
            if !unc.is_empty() {
                println!(
                    "\n{}",
                    yellow(&bold(&format!("  estimated sizes ({})", unc.len())))
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
        "delivery" | "einbringung" => std::process::exit(cmd_delivery(&model, &argv)),
        "sun" | "sonne" => std::process::exit(cmd_sun(&model, &argv)),
        "declarations" | "deklaration" => std::process::exit(cmd_declarations(&model, &argv)),
        "purchase-order" | "kaufen" => std::process::exit(cmd_purchase_order(&model, &argv)),
        "tolerance" | "toleranz" => {
            let Some(name) = argv.get(1) else { usage() };
            let layout = match model.load_layout(name) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            let r = match interior::tolerance::robustness(&model, &layout) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            if argv.contains(&"--json".to_string()) {
                println!("{}", serde_json::to_string_pretty(&r).unwrap());
            } else {
                tolerance_report(&r);
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
                    "  --move <id>[,<id>] is missing; without movable pieces there is nothing to search"
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
                bold(&format!("Search in \"{}\"", rep.base)),
                dim(&format!(
                    "beweglich: {} · Raster {} cm",
                    rep.moved.join(", "),
                    rep.step
                ))
            );
            if rep.hits.is_empty() {
                println!("  {}", red("no violation-free position found"));
                println!(
                    "{}",
                    dim(&format!(
                        "  That is a result, not an error: at a {} cm grid there is none.",
                        rep.step
                    ))
                );
                println!(
                    "{}",
                    dim("  Search finer (--step 10) or free one more piece (--move).")
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
                            h.tightest_reserve_cm
                                .map(|r| format!("{r} cm"))
                                .unwrap_or_else(|| "—".into()),
                            h.wall_contact_cm,
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
                    let note = format!("Best hit of the search in \"{}\": {} warnings, {} cm wall contact, tightest route {} cm, tightest hard reserve {}.", rep.base, best.soft, best.wall_contact_cm, best.bottleneck_cm, best.tightest_reserve_cm.map(|r| format!("{r} cm")).unwrap_or_else(|| "not measured".into()));
                    match write_layout(&model, &out, &l, &note) {
                        Ok(()) => println!(
                            "\n  {} {}",
                            green("written:"),
                            model.layouts_dir().join(format!("{out}.toml")).display()
                        ),
                        Err(e) => eprintln!("\n  {} {e}", red("Schreiben fehlgeschlagen:")),
                    }
                }
            }
            println!(
                "{}",
                dim(&format!(
                    "\n  {} candidates after the prefilter, {} fully checked, {} hits in {:.1} s\n",
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
            let html = match plan::page(&model, &layouts, plan::Origin::Proposal) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("{}", red(&e.to_string()));
                    std::process::exit(2);
                }
            };
            let out = flag(&argv, "out").unwrap_or_else(|| "plans.html".into());
            match std::fs::write(&out, &html) {
                Ok(()) => println!(
                    "  {} {out}  ({} plans, {:.0} KB)",
                    green("written:"),
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
    // The port comes from service.toml as SJEL_PORT; 8092 is the predecessor's port, so old
    // bookmarks still work.
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

/// Furnish a flat from scratch. `search` moves pieces in an existing layout; this places a new
/// one with a beam search, since the full product over six pieces is about 10^15 combinations. The
/// result is computed and repeatable, not guaranteed optimal; `search.rs` says why.
fn cmd_compose(model: &Model, argv: &[String]) -> i32 {
    let refs: Vec<String> = match flag(argv, "pieces") {
        Some(v) => v.split(',').map(|s| s.trim().to_string()).collect(),
        None => {
            eprintln!(
                "{}",
                red("--pieces is missing: interior compose --pieces a,b,c [--step 25] [--beam 60] [--limit 5] [--out name]")
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
        "\n{}  {} pieces, step {} cm, beam {}, {:.1} s\n",
        bold("compose"),
        refs.len(),
        spec.step,
        spec.beam,
        t0.elapsed().as_secs_f64()
    );
    if out.is_empty() {
        println!("{}\n", red("  no complete arrangement found"));
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
            c.tightest_reserve_cm
                .map(|r| format!("{r} cm"))
                .unwrap_or_else(|| "—".into()),
            c.bottleneck_cm,
            c.wall_contact_cm,
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
    // Only the best, and only on request: a search that writes files unasked cannot be tried safely.
    if let Some(name) = flag(argv, "out") {
        // Which rank to write; the first by default. The order has no rule against free-standing pieces,
        // only `wall_contact_cm` as a tie-breaker, so whoever read the list may pick another.
        let rank: usize = flag(argv, "rank").and_then(|v| v.parse().ok()).unwrap_or(1);
        let Some(best) = out.get(rank.saturating_sub(1)) else {
            eprintln!(
                "{}",
                red(&format!(
                    "--rank {rank} does not exist; there are {}",
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
        // The sentence about the clearance check belongs here: true for `compose`, false for an empty
        // plan the API creates.
        let note_text = format!(
            "Placed by `interior compose` on {}: {} pieces, step {} cm, beam {}.\n\
             \n\
             Placed by a machine, not by hand. Every position went through the full\n\
             clearance check; the result is in the verdict next to it, not here.",
            interior::layout_io::today(),
            refs.len(),
            spec.step,
            spec.beam
        );
        match interior::layout_io::create(model, &name, &layout, &note_text) {
            Ok(()) => println!("  {} {name}\n", green("written:")),
            Err(e) => {
                eprintln!("{}", red(&e.to_string()));
                return 2;
            }
        }
    }
    0
}

/// What a verdict survives, in words.
fn tolerance_report(r: &interior::tolerance::Robustness) {
    use interior::tolerance::Holds;
    println!("\n{}", bold(&r.layout));
    match &r.holds {
        Holds::Fails => {
            println!(
            "  {}",
            red("fails with the recorded sizes already; measurement error is not the question here")
        )
        }
        Holds::NothingGuessed => println!(
            "  {}",
            dim("no size in this layout is marked estimated; there is nothing to vary")
        ),
        Holds::BeyondHorizon { horizon_cm } => println!(
            "  {}",
            green(&format!(
                "holds even {horizon_cm} cm of measurement error (not searched further)"
            ))
        ),
        Holds::UpTo { cm } => {
            println!(
                "  {}",
                green(&format!("holds up to {cm} cm of measurement error"))
            );
            println!(
                "  {}",
                yellow(&format!(
                    "at {} cm the first to break: {}",
                    cm + 1,
                    r.tips_at.join(", ")
                ))
            );
        }
    }
    if let Some(cm) = r.tightest_reserve_cm {
        println!("  {}", dim(&format!("knappste harte Messung: {cm} cm")));
    }
    for u in &r.guessed {
        println!(
            "  {}",
            dim(&format!("estimated: {}: {}", u.label, u.fields.join(", ")))
        );
    }
    for n in &r.not_varied {
        println!(
            "  {}",
            dim(&format!(
                "`{n}` carries its size in the layout (size:) and was not varied"
            ))
        );
    }
    println!();
}

/// Does it get in? For every piece of a layout, or for an imagined one.
fn cmd_delivery(model: &Model, argv: &[String]) -> i32 {
    use interior::delivery::{delivery, path_to_place, through_the_door, Delivery, DoorFit};

    let line = |e: &Delivery| {
        let door = match &e.door {
            DoorFit::Fits { air_cm, door_cm } => {
                green(&format!("through the {door_cm} cm door, {air_cm} cm air"))
            }
            DoorFit::DoesNotFit {
                missing_cm,
                door_cm,
            } => red(&format!(
                "does NOT fit through the {door_cm} cm door: {missing_cm} cm too wide"
            )),
            DoorFit::CarriedApart {
                missing_cm,
                door_cm,
            } => yellow(&format!(
                "{missing_cm} cm wider than the {door_cm} cm door, comes apart according to its row"
            )),
            DoorFit::NoEntrance => yellow("no door declared as the entrance"),
        };
        let path_result = if e.affordable {
            green(&format!(
                "Weg frei ({} Schritte{})",
                e.steps.unwrap_or(0),
                if e.rotates { ", with rotation" } else { "" }
            ))
        } else {
            red("no path to the place")
        };
        println!(
            "  {:26} {:>3}×{:<3}  {door}  {path_result}",
            bold(&e.reference),
            e.width,
            e.depth
        );
        if let Some(g) = &e.reason {
            println!("      {}", dim(g));
        }
    };

    // An imagined piece: the question before buying.
    if let (Some(width), Some(depth)) = (
        flag(argv, "width")
            .or_else(|| flag(argv, "b"))
            .and_then(|v| v.parse::<i32>().ok()),
        flag(argv, "depth")
            .or_else(|| flag(argv, "t"))
            .and_then(|v| v.parse::<i32>().ok()),
    ) {
        let disassembles = argv
            .iter()
            .any(|a| a == "--disassembles" || a == "--zerlegbar");
        println!("\n{}  {width}×{depth} cm\n", bold("delivery"));
        match through_the_door(model, width, depth, disassembles) {
            DoorFit::Fits { air_cm, door_cm } => {
                println!(
                    "  {}",
                    green(&format!(
                        "fits upright through the {door_cm} cm door, {air_cm} cm air"
                    ))
                );
                0
            }
            DoorFit::DoesNotFit {
                missing_cm,
                door_cm,
            } => {
                println!(
                    "  {}",
                    red(&format!(
                        "does not fit: the narrow side is {missing_cm} cm wider than the {door_cm} cm door"
                    ))
                );
                println!(
                    "  {}",
                    dim(
                        "does it come apart? then `--disassembles`, and `disassembles = true` on the row"
                    )
                );
                1
            }
            DoorFit::CarriedApart {
                missing_cm,
                door_cm,
            } => {
                println!(
                    "  {}",
                    yellow(&format!(
                        "{missing_cm} cm wider than the {door_cm} cm door; declared to come apart, so it gets in"
                    ))
                );
                0
            }
            DoorFit::NoEntrance => {
                println!(
                    "  {}",
                    yellow("no opening is declared the entrance (`eingang = true`)")
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
        println!("\n{}  \"{}\"\n", bold("delivery"), layout.name);
        let mut worse = 0;
        for it in &layout.items {
            match delivery(model, &layout, &it.reference) {
                Ok(e) => {
                    if !e.affordable || matches!(e.door, DoorFit::DoesNotFit { .. }) {
                        // `CarriedApart` does not count here: the flat has an answer, and an exit code that turns red on
                        // an answered question gets switched off.
                        worse += 1;
                    }
                    line(&e);
                }
                Err(e) => eprintln!("  {} {e}", red(&it.reference)),
            }
        }
        // The line that shows the difference between "stands there" and "gets there".
        let _ = path_to_place;
        println!();
        i32::from(worse > 0)
    }
}

/// When in the year which piece is in the sun.
fn cmd_sun(model: &Model, argv: &[String]) -> i32 {
    let Some(name) = argv.get(1) else { usage() };
    let layout = match model.load_layout(name) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    let b = match interior::sun::report(model, &layout) {
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

    println!("\n{}  \"{}\"\n", bold("sun"), b.layout);
    // One line per day, one column per hour: a year fits in four lines, and the shape shows the sun
    // moving.
    for (tag, _) in interior::sun::DAYS {
        let hours: Vec<&interior::sun::SunHour> =
            b.hours.iter().filter(|s| s.day == *tag).collect();
        let header: String = hours
            .iter()
            .map(|s| format!("{:>3}", s.hour_local))
            .collect();
        let line: String = hours
            .iter()
            .map(|s| {
                if s.altitude_deg <= 0.0 {
                    "  ·".to_string()
                } else if s.hit.is_empty() {
                    "  -".to_string()
                } else {
                    format!("{:>3}", s.hit.len())
                }
            })
            .collect();
        // Pad first, then colour, or the escape sequences count as width.
        println!("  {}{}", bold(&format!("{tag:<16}")), dim(&header));
        println!("  {:<16}{}", "", yellow(&line));
    }
    println!(
        "\n  {}",
        dim("· below the horizon · - no piece in the sun · number = pieces hit")
    );
    if !b.hits_per_piece.is_empty() {
        println!("\n  {}", bold("Hours in direct sun"));
        for (k, n) in &b.hits_per_piece {
            println!("    {k:26} {n:>3} of {}", interior::sun::checked_hours());
        }
    }
    for o in &b.without_glass_heights {
        println!(
            "  {}",
            yellow(&format!(
                "`{o}` has no glass heights (glas_von_cm/glas_bis_cm) and throws no light here"
            ))
        );
    }
    println!();
    0
}

/// Which items are still judged by name, and what changes when they declare themselves.
fn cmd_declarations(model: &Model, argv: &[String]) -> i32 {
    let stand = match interior::declarations::overview(model) {
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

    let declared = stand.iter().filter(|s| s.declared).count();
    println!(
        "\n{}  {} of {} entries declare themselves\n",
        bold("declarations"),
        declared,
        stand.len()
    );
    for s in &stand {
        if s.declared {
            println!("  {} {}", green("declared "), bold(&s.id));
            continue;
        }
        let Some(v) = &s.proposal else {
            println!(
                "  {} {}  {}",
                dim("by name"),
                bold(&s.id),
                dim(&format!(
                    "classified as {}, which implies no threshold",
                    s.guessed_as
                ))
            );
            continue;
        };
        let f = s.consequences.as_ref();
        let effect = match f {
            Some(f) if f.changed.is_empty() => green("changes no verdict"),
            Some(f) => yellow(&format!("changes: {}", f.changed.join(", "))),
            None => dim("not computed"),
        };
        println!(
            "  {} {}  {}  {effect}",
            yellow("by name"),
            bold(&s.id),
            dim(&format!("classified as {}", s.guessed_as))
        );
        for line in v.toml.lines() {
            println!("      {}", dim(line));
        }
        if !s.in_layouts.is_empty() {
            println!(
                "      {}",
                dim(&format!("stands in {} layouts", s.in_layouts.len()))
            );
        }
    }
    println!();
    0
}

/// Which need first, and when it is affordable.
fn cmd_purchase_order(model: &Model, argv: &[String]) -> i32 {
    // The order runs without a balance too: it is an order, and the timeline is the extra.
    let balance = interior::store::Store::open(&sjel_config::database_path())
        .ok()
        .and_then(|s| s.borrow_connection().ok())
        .and_then(|conn| interior::budget::monthly_balance(&conn).ok().flatten());
    let r = match interior::budget::purchase_order(model, balance) {
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
    if r.lines.is_empty() {
        println!("  {}\n", dim("no priced need open"));
    }
    for (n, p) in r.lines.iter().enumerate() {
        // Two reasons for no month count, kept apart: no balance is a missing measurement, a balance
        // that is not positive is an answer.
        let when = match (p.affordable_after_months, &r.balance) {
            (Some(m), _) => green(&format!("after {m:.1} months")),
            (None, Some(_)) => dim("not affordable from this balance"),
            (None, None) => dim("no monthly balance measured"),
        };
        println!(
            "  {:>2}. {:28} {:>9}  {:>10} kumuliert  {when}",
            n + 1,
            bold(&p.label),
            euro(p.price_cents.unwrap_or(0)),
            euro(p.cumulative_cents)
        );
        if !p.in_layouts.is_empty() {
            println!(
                "      {}",
                dim(&format!("eingeplant in {}", p.in_layouts.join(", ")))
            );
        }
    }
    if !r.unpriced.is_empty() {
        println!(
            "\n  {}",
            yellow(&format!(
                "{} needs without a price; they are in no order",
                r.unpriced.len()
            ))
        );
        for p in &r.unpriced {
            println!("      {}", dim(&p.label));
        }
    }
    if !r.unknown_priorities.is_empty() {
        println!(
            "\n  {}",
            yellow(&format!(
                "unknown priority, sorted last and named here: {}",
                r.unknown_priorities.join(", ")
            ))
        );
    }
    match &r.balance {
        Some(s) if s.median_cents > 0 => println!(
            "\n  {}",
            dim(&format!(
                "Median monthly balance {} over {} months ({} to {})",
                euro(s.median_cents),
                s.months,
                s.from,
                s.to
            ))
        ),
        Some(s) => println!(
            "\n  {}",
            yellow(&format!(
                "Median monthly balance {}: nothing accrues from it, so there is no month count",
                euro(s.median_cents)
            ))
        ),
        None => println!(
            "\n  {}",
            dim("no monthly balance available (finance wrote nothing here)")
        ),
    }
    println!();
    0
}

/// What a RoomPlan capture holds, and how it relates to `room.toml`. Writes nothing without
/// `--write`, and claims no identity between the scan's openings and those in `room.toml`; the
/// matching is a decision.
fn roomplan_show(model: &Model, argv: &[String]) -> i32 {
    use interior::roomplan;
    let flat = model.room.flat.id.clone();
    let dir_path = match flag(argv, "capture") {
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
    let scan = match roomplan::read_usdz(&dir_path.join("captured-room.usdz")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&e.to_string()));
            return 2;
        }
    };
    let d = roomplan::compare(&scan, model);

    if argv.iter().any(|a| a == "--json") {
        let content = serde_json::json!({
            "capture": dir_path.display().to_string(),
            "sha256": scan.sha256,
            "byte_length": scan.byte_length,
            "mesh_assets": scan.mesh_assets,
            "room_groups": scan.room_groups,
            "capture_finished_at": scan.capture_finished_at,
            "categories": scan.categories(),
            "scan_wall_height_cm": d.scan_wall_height_cm,
            "scan_wall_thickness_cm": scan.wall_thickness_m().map(|m| m * 100.0),
            "model_height_cm": d.model_height_cm,
            "scan_wall_length_cm": d.scan_wall_length_cm,
            "scan_openings": d.scan_openings.iter().map(|(id, cat, b, h)| serde_json::json!({
                "id": id, "category": cat, "width_cm": b, "height_cm": h
            })).collect::<Vec<_>>(),
            "model_openings": d.model_openings.iter().map(|(id, b)| serde_json::json!({
                "id": id, "width_cm": b
            })).collect::<Vec<_>>(),
            "scan_floor_area_m2": d.scan_floor_area_m2,
            "model_inner_area_m2": d.model_inner_area_m2,
            "scan_objects": d.scan_objects.iter().map(|(n, m)| serde_json::json!({
                "element": n, "cm": m
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&content).unwrap_or_default()
        );
        return 0;
    }

    println!("\n{}\n", bold("RoomPlan-Aufnahme"));
    println!("  Aufnahme   {}", dir_path.display());
    println!(
        "  USDZ       {} Bytes, sha256 {}",
        scan.byte_length,
        &scan.sha256[..16.min(scan.sha256.len())]
    );
    println!(
        "  Inhalt     {} Maschen, {} Raumgruppen {}",
        scan.mesh_assets,
        scan.room_groups,
        dim("(the groups name centres, not members)")
    );
    println!(
        "  Aufnahme   {}",
        match &scan.capture_finished_at {
            Some(t) => format!("{t} {}", dim("(from the archive, no zone)")),
            None => dim("no timestamp").to_string(),
        }
    );

    println!("\n  {}", bold("Wall height"));
    match d.scan_wall_height_cm {
        Some(cm) => {
            println!("    {:.2} cm   {}", cm, dim("all walls agree"));
            if d.model_height_cm == 0 {
                println!(
                    "    room.toml states 0 {}: {}",
                    dim("= not measured"),
                    green("the scan answers this open question")
                );
            } else {
                println!(
                    "    room.toml states {} cm   deviation {:+.2} cm",
                    d.model_height_cm,
                    cm - d.model_height_cm as f64
                );
            }
        }
        None => println!(
            "    {}",
            red("the walls contradict each other, so no number")
        ),
    }
    match scan.wall_thickness_m() {
        Some(m) => println!(
            "  Wall thickness {:.2} cm   {}",
            m * 100.0,
            dim("all equal, so a constant and no measurement; not part of the comparison")
        ),
        None => println!("  Wall thickness {}", dim("the walls differ, so measured")),
    }
    println!(
        "  Walls       {}, total length {:.1} cm",
        scan.surfaces().filter(|e| e.category == "Wall").count(),
        d.scan_wall_length_cm
    );
    println!(
        "              {}",
        dim("the scan splits walls at their openings, room.toml does not")
    );

    println!("\n  {}", bold("Oeffnungen im Scan"));
    for (id, cat, b, h) in &d.scan_openings {
        println!("    {:<9} {:<24} {:>7.1} x{:>6.1} cm", id, cat, b, h);
    }
    println!("\n  {}", bold("Oeffnungen in room.toml"));
    for (id, b) in &d.model_openings {
        println!("    {:<26} breit {:>5} cm", id, b);
    }
    println!(
        "\n    {}",
        yellow("no identity is claimed between the lists:")
    );
    println!(
        "    {}",
        dim("which scan entry is which opening is a decision, not a computation")
    );

    println!("\n  {}", bold("Floor area"));
    match d.scan_floor_area_m2 {
        Some(m2) => {
            let delta = m2 - d.model_inner_area_m2;
            let percent = if d.model_inner_area_m2 > 0.0 {
                delta / d.model_inner_area_m2 * 100.0
            } else {
                0.0
            };
            println!(
                "    Scan        {:.2} m²  {}",
                m2,
                dim("upper side of the floor plate, from its triangles")
            );
            println!(
                "    room.toml   {:.2} m²  {}",
                d.model_inner_area_m2,
                dim("main room + bathroom, computed from the polygon")
            );
            println!("    Abweichung  {:+.2} m²  ({:+.1} %)", delta, percent);
        }
        None => println!("    {}", dim("the scan carries no floor plate")),
    }

    println!("\n  {}", bold("Furniture and appliances in the scan"));
    for (name, m) in &d.scan_objects {
        println!(
            "    {:<34} {:>6.1} x{:>6.1} x{:>6.1} cm",
            name, m[0], m[1], m[2]
        );
    }

    if argv.iter().any(|a| a == "--write") {
        match roomplan::write_draft(&scan, &flat, &dir_path, &civil_date::today()) {
            Ok(file_path) => println!("\n  {} {}", green("draft written:"), file_path.display()),
            Err(e) => {
                eprintln!("{}", red(&format!("draft not written: {e}")));
                return 1;
            }
        }
    } else {
        println!(
            "\n  {}",
            dim("nothing written. With --write the draft goes next to the capture as draft.json.")
        );
    }
    println!();
    0
}

/// Where the pieces REALLY stand, and what has no place yet.
///
/// `--set bett_bestand=x,y[,rot]` writes a position; without `--set` it only reads. A placement is
/// a row, not a proposal. `--plan <file>` writes a plan of the as-is state, built from the same
/// [`PlacedItem`] rows as a layout.
fn placements_show(model: &Model, argv: &[String]) -> i32 {
    let flat = model.room.flat.id.clone();
    let st = match interior::store::Store::open(&sjel_config::database_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", red(&format!("database not reachable: {e}")));
            return 2;
        }
    };

    // Write first, then read, so the output shows the state after the arguments.
    let mut written = 0usize;
    let mut i = 0usize;
    while i < argv.len() {
        if argv[i] == "--set" {
            let Some(raw) = argv.get(i + 1) else {
                eprintln!("{}", red("--set braucht id=x,y[,rot]"));
                return 2;
            };
            let Some((id, x, y, rot)) = parse_placement(raw) else {
                eprintln!("{}", red(&format!("--set {raw} is not id=x,y[,rot]")));
                return 2;
            };
            if !model.catalogue.contains_key(&id) {
                eprintln!(
                    "{}",
                    red(&format!("`{id}` is not in the catalogue; nothing written"))
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
                eprintln!("{}", red(&format!("not written: {e}")));
                return 2;
            }
            written += 1;
            i += 2;
            continue;
        }
        i += 1;
    }

    let placed = match st.placements(&flat) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", red(&format!("placements do not load: {e}")));
            return 2;
        }
    };
    let nach_id: std::collections::BTreeMap<&str, &interior::store::Placement> =
        placed.iter().map(|p| (p.item_id.as_str(), p)).collect();

    // Owned only: a wish has no place, it has a reason.
    let mut owned: Vec<(&String, &interior::store::Item)> = model
        .catalogue
        .iter()
        .filter(|(id, _)| model.states.get(*id).copied() == Some(interior::store::State::Owned))
        .collect();
    owned.sort_by(|a, b| a.0.cmp(b.0));

    let mut set: Vec<(&String, &interior::store::Placement)> = Vec::new();
    let mut without: Vec<(&String, &interior::store::Item)> = Vec::new();
    for (id, item) in &owned {
        match nach_id.get(id.as_str()) {
            Some(p) => set.push((id, p)),
            None => without.push((id, item)),
        }
    }
    // A placement for a piece no longer owned points at something that is gone; no list above would
    // name it.
    let orphaned: Vec<&&str> = nach_id
        .iter()
        .filter(|(id, _)| !owned.iter().any(|(b, _)| b.as_str() == **id))
        .map(|(id, _)| id)
        .collect();

    if argv.iter().any(|a| a == "--json") {
        let wert = serde_json::json!({
            "flat": flat,
            "written": written,
            "owned": owned.len(),
            "placed": set.iter().map(|(id, p)| serde_json::json!({
                "id": id, "x": p.x, "y": p.y, "rot": p.rot
            })).collect::<Vec<_>>(),
            "unplaced": without.iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>(),
            "orphaned": orphaned,
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
        owned.len(),
        dim("places are one row per piece; a layout is a proposal and a file")
    );
    println!("  gesetzt    {}", bold(&set.len().to_string()));
    for (id, p) in &set {
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
        bold(&format!("without a place {}", without.len())),
        dim("(the plan knows nothing of it yet)")
    );
    for (id, item) in &without {
        println!("    {:<28} {}", id, dim(&item.label));
    }
    if !orphaned.is_empty() {
        println!(
            "\n  {}",
            yellow(&format!(
                "placements without an owned piece: {}; they point at something no longer there",
                orphaned.iter().map(|s| **s).collect::<Vec<_>>().join(", ")
            ))
        );
    }

    if let Some(file) = flag(argv, "plan") {
        let layout = interior::model::Layout {
            name: "as-is".to_string(),
            id: "as-is".to_string(),
            items: set
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
        match plan::page(model, std::slice::from_ref(&layout), plan::Origin::AsIs) {
            Ok(html) => match std::fs::write(&file, &html) {
                Ok(()) => println!("\n  {} {}", green("as-is plan written:"), file),
                Err(e) => {
                    eprintln!("{}", red(&format!("{file}: {e}")));
                    return 1;
                }
            },
            Err(e) => {
                eprintln!("{}", red(&format!("plan not built: {e}")));
                return 1;
            }
        }
    }
    println!();
    0
}

/// `id=x,y` or `id=x,y,rot`; the id cannot contain `=`, which separates the field.
fn parse_placement(raw: &str) -> Option<(String, i32, i32, i32)> {
    let (id, rest) = raw.split_once('=')?;
    if id.is_empty() {
        return None;
    }
    let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let x = parts[0].parse::<i32>().ok()?;
    let y = parts[1].parse::<i32>().ok()?;
    let rot = match parts.get(2) {
        Some(r) => r.parse::<i32>().ok()?,
        None => 0,
    };
    Some((id.to_string(), x, y, rot))
}

/// Cents as euros with a comma.
fn euro(cent: i64) -> String {
    format!("{},{:02} €", cent / 100, (cent % 100).abs())
}
