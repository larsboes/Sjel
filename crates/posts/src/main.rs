//! `posts-cli`: the binary `crates/posts/posts` execs. Run `posts` with no arguments for usage.

use posts::{media, Accent, Layout, Role, Slot, State, Store};
use std::io::Read as _;
use std::process::ExitCode;

const USAGE: &str = "Usage:
  posts status --json                          # every post with its state
  posts get <slug> --json                      # text, ordered media and export files
  posts new <slug> --title <text> [--event <text>]
  posts text put <slug> < file                 # replace the text; the old text becomes a revision
  posts state <slug> <sketch|draft|ready|published> [--url <url>]

  posts media add-selected <slug> [--role <cover|body>]   # the items selected in Photos
  posts media add <slug> <photos-id> [--role <cover|body>]
  posts media move <slug> <n> <up|down>        # n counts from 1
  posts media remove <slug> <n>
  posts media pair <slug> <n> [--layout <pair|stack>] [--accent <none|pixels|lowpoly>]
  posts media split <slug> <n>
  posts media accent <slug> <n> <none|pixels|lowpoly>

  posts export <slug>                          # numbered upload files in out/";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("posts: {e}");
            ExitCode::FAILURE
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn nth(args: &[String], i: usize, what: &str) -> Result<String, String> {
    args.get(i)
        .cloned()
        .ok_or(format!("missing {what}\n\n{USAGE}"))
}

fn slot_number(raw: &str) -> Result<usize, String> {
    match raw.parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n - 1),
        _ => Err(format!("slot '{raw}' is not a number from 1")),
    }
}

fn role(args: &[String]) -> Result<Role, String> {
    match flag(args, "--role").as_deref() {
        None | Some("body") => Ok(Role::Body),
        Some("cover") => Ok(Role::Cover),
        Some(other) => Err(format!("role '{other}' is not cover or body")),
    }
}

fn json<T: serde::Serialize>(value: &T) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string(value).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let store = Store::from_env()?;
    let verb: Vec<&str> = args.iter().take(2).map(String::as_str).collect();
    match verb.as_slice() {
        ["status", ..] => json(&store.view()?),
        ["get", slug] => json(&store.detail(slug)?),
        ["new", slug] => {
            let title = flag(args, "--title").ok_or("new needs --title")?;
            store.create(slug, &title, flag(args, "--event"))?;
            println!("created {slug}");
            Ok(())
        }
        ["text", "put"] => {
            let slug = nth(args, 2, "slug")?;
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| e.to_string())?;
            let n = store.set_text(&slug, &text)?;
            println!("saved {slug}; previous text is revision {n}");
            Ok(())
        }
        ["state", slug] => {
            let state = State::parse(&nth(args, 2, "state")?)?;
            store.set_state(slug, state, flag(args, "--url"))?;
            println!("{slug} is {}", state.name());
            Ok(())
        }
        ["media", sub] => media_verb(&store, sub, args),
        ["export", slug] => {
            let slots = store.media(slug)?.items;
            let files = media::export(&slots, &store.out_dir(slug))?;
            for f in files {
                println!("{}", f.display());
            }
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

fn media_verb(store: &Store, sub: &str, args: &[String]) -> Result<(), String> {
    let slug = nth(args, 2, "slug")?;
    let mut m = store.media(&slug)?;
    let single = |id: String, role| Slot {
        photos_id: Some(id),
        role,
        people: vec![],
        collage: None,
    };
    let detail = match sub {
        "add-selected" => {
            let ids = media::selected_ids()?;
            let role = role(args)?;
            let n = ids.len();
            m.items.extend(ids.into_iter().map(|id| single(id, role)));
            format!("added {n} selected item(s)")
        }
        "add" => {
            let id = nth(args, 3, "photos-id")?;
            posts::photos_id(&id)?;
            m.items.push(single(id, role(args)?));
            "added 1 item".into()
        }
        "move" => {
            let i = slot_number(&nth(args, 3, "slot")?)?;
            m.shift(i, &nth(args, 4, "direction")?)?;
            format!("moved slot {}", i + 1)
        }
        "remove" => {
            let i = slot_number(&nth(args, 3, "slot")?)?;
            if i >= m.items.len() {
                return Err(format!("no slot {}", i + 1));
            }
            m.items.remove(i);
            format!("removed slot {}", i + 1)
        }
        "pair" => {
            let i = slot_number(&nth(args, 3, "slot")?)?;
            let layout = Layout::parse(flag(args, "--layout").as_deref().unwrap_or("pair"))?;
            let accent = Accent::parse(flag(args, "--accent").as_deref().unwrap_or("none"))?;
            m.pair(i, layout, accent)?;
            format!("slots {} and {} are one collage", i + 1, i + 2)
        }
        "split" => {
            let i = slot_number(&nth(args, 3, "slot")?)?;
            m.split(i)?;
            format!("split slot {}", i + 1)
        }
        "accent" => {
            let i = slot_number(&nth(args, 3, "slot")?)?;
            let accent = Accent::parse(&nth(args, 4, "accent")?)?;
            let collage = m
                .items
                .get_mut(i)
                .and_then(|s| s.collage.as_mut())
                .ok_or(format!("slot {} is not a collage", i + 1))?;
            collage.accent = accent;
            format!("slot {} accent changed", i + 1)
        }
        other => return Err(format!("unknown media verb '{other}'\n\n{USAGE}")),
    };
    store.save_media(&slug, &m)?;
    println!("{detail}");
    Ok(())
}
