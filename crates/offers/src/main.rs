//! `offers` CLI. Usage: `offers` with no arguments.

use std::path::Path;

const USAGE: &str = "usage: offers import <export-dir>      read what `comms mail export` wrote
       offers list [--all]              valid offers by last day; --all adds released ones
       offers show <thread-id>          one offer with the line each value was read from
       offers set <thread-id> active|used|dismissed
       offers page [--open]             write index.html of valid offers; --open shows it
       offers reread                    extract stored originals again; used/dismissed stay
       offers released                  JSON thread ids whose mail may go";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = run(&args) {
        eprintln!("offers: {error}");
        std::process::exit(2);
    }
}

fn run(args: &[String]) -> offers::Res<()> {
    let root = offers::root()?;
    let today = chrono::Local::now().date_naive();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["import", dir] => {
            let report = offers::import(&root, Path::new(dir))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
            );
        }
        ["list", rest @ ..] if rest.is_empty() || rest == ["--all"] => {
            let all = !rest.is_empty();
            let mut shown = offers::load(&root)?;
            shown.retain(|(_, offer)| all || !offer.released(today));
            shown.sort_by_key(|(_, offer)| offer.last_day());
            for (id, offer) in &shown {
                let value = |q: &Option<offers::Quoted>| {
                    q.as_ref().map_or("-".to_string(), |q| q.value.clone())
                };
                let until = if offer.status == offers::Status::Active {
                    offer
                        .last_day()
                        .map_or("?".into(), |d| d.format("%Y-%m-%d").to_string())
                } else {
                    serde_json::to_value(offer.status)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default()
                };
                println!(
                    "{}  {:<10}  {:<6} {:<12} {}  {}",
                    id,
                    until,
                    value(&offer.discount),
                    value(&offer.code),
                    offer.from.as_deref().unwrap_or_default(),
                    offer.subject.as_deref().unwrap_or_default(),
                );
            }
            println!("{} offer(s)", shown.len());
        }
        ["show", id] => {
            let found = offers::load(&root)?
                .into_iter()
                .find(|(stored, _)| stored == id)
                .ok_or_else(|| format!("no offer {id}"))?;
            print!("{}", toml::to_string(&found.1).map_err(|e| e.to_string())?);
        }
        ["set", id, status] => {
            offers::set_status(&root, id, offers::Status::parse_settable(status)?)?;
            println!("{id}: {status}");
        }
        ["page", rest @ ..] if rest.is_empty() || rest == ["--open"] => {
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            let path = root.join("index.html");
            std::fs::write(&path, offers::page(&offers::load(&root)?, today))
                .map_err(|e| e.to_string())?;
            println!("{}", path.display());
            if !rest.is_empty() {
                std::process::Command::new("open")
                    .arg(&path)
                    .status()
                    .map_err(|e| e.to_string())?;
            }
        }
        ["reread"] => println!("{} offer(s) changed", offers::reread(&root)?),
        ["released"] => {
            let ids = offers::released(&offers::load(&root)?, today);
            println!(
                "{}",
                serde_json::to_string(&ids).map_err(|e| e.to_string())?
            );
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}
