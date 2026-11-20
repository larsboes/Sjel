//! Local Places backfills and climate refresh.

use std::io::Write;

use places::store::{Fallible, PlacesStore};

pub(crate) const USAGE: &str =
    "usage: places-server [server | backfill <amex|calendar|cities|stations|takeout|travelers|vault> | climate fetch [--place <id>] [--force]]";

pub(crate) fn run(store: &PlacesStore, args: &[String], out: &mut dyn Write) -> Fallible<()> {
    let names: Vec<_> = args.iter().map(String::as_str).collect();
    let today = places::today();
    let backfill = match names.as_slice() {
        ["backfill", "amex"] => places::backfill::amex,
        ["backfill", "calendar"] => places::backfill::calendar,
        ["backfill", "cities"] => places::backfill::cities,
        ["backfill", "stations"] => places::backfill::stations,
        ["backfill", "takeout"] => places::backfill::takeout,
        ["backfill", "travelers"] => places::backfill::travelers,
        ["backfill", "vault"] => places::backfill::vault,
        ["climate", ..] => return climate(store, &args[1..], out),
        _ => return Err(USAGE.into()),
    };
    backfill(store, &today, out)?;
    match places::people::link_exact(store) {
        Ok(0) => {}
        Ok(n) => writeln!(out, "places: linked {n} person name(s) to people")?,
        Err(error) => writeln!(out, "places: person names not linked: {error}")?,
    }
    Ok(())
}

fn climate(store: &PlacesStore, args: &[String], out: &mut dyn Write) -> Fallible<()> {
    if args.first().map(String::as_str) != Some("fetch") {
        return Err(USAGE.into());
    }
    let mut place_id: Option<String> = None;
    let mut force = false;
    let mut rest = args[1..].iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--force" => force = true,
            "--place" => place_id = Some(rest.next().cloned().ok_or(USAGE)?),
            _ => return Err(USAGE.into()),
        }
    }

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
        match places::climate::fetch_normals(store, place, &url, &today, force) {
            Ok(outcome) if outcome.cached => cached += 1,
            Ok(outcome) => {
                fetched += 1;
                writeln!(out, "{}: {} months", place.id, outcome.months)?;
            }
            Err(error) => {
                failed += 1;
                // The message is already stripped of the request URL, which is
                // the coordinate pair (climate.rs).
                writeln!(out, "{}: {error}", place.id)?;
            }
        }
    }
    writeln!(out,
        "climate fetch: {} considered, {fetched} fetched, {cached} served from cache, {failed} failed",
        targets.len()
    )?;
    if failed > 0 && fetched == 0 {
        return Err("every climate fetch failed".into());
    }
    Ok(())
}
