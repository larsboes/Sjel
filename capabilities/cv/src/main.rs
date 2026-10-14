//! `cv-cli`: the binary `capabilities/cv/cv` execs. Run `cv` with no arguments for usage.

use cv::model::{Application, Lang, Master, Status};
use cv::{linkedin, Built, NewApp, Paths, Res, Saved, Target};
use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "Usage:
  cv build --profile <tag> --lang <en|de> [--out <path>]
  cv build --all [--lang <en|de>]     # every profile x its languages
  cv watch --profile <tag> --lang <en|de> [--out <path>]
  cv list-profiles
  cv status --json                    # profiles, applications, PDFs and staleness

  cv app new <slug> --company <name> --role <title> [--profile <tag>] [--lang <en|de>] [--url <url>]
  cv app build <slug>                 # -> dist/apps/CV_<slug>_<lang>.pdf
  cv app list [--json]
  cv app status <slug> <status> [note]  # draft applied interview offer accepted rejected withdrawn

  cv edit get [slug]                  # raw YAML of the master, or of one application
  cv edit put [slug] < file           # replace it after a parse and render check; commits it
  cv model get [slug]                 # the same file as typed JSON
  cv model put [slug] < file.json     # replace it from typed JSON; same checks

  cv linkedin pull [DOMAIN...]        # Member Snapshot API -> data/cv/linkedin/<DOMAIN>.json
  cv linkedin diff
  cv linkedin sheet [--lang <en|de>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cv: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `--name value` pairs and bare flags after the verb.
struct Flags {
    values: Vec<(String, String)>,
    switches: Vec<String>,
    rest: Vec<String>,
}

impl Flags {
    fn parse(args: &[String], with_value: &[&str], switches: &[&str]) -> Res<Flags> {
        let mut flags = Flags { values: Vec::new(), switches: Vec::new(), rest: Vec::new() };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if with_value.contains(&a.as_str()) {
                let v = it.next().ok_or_else(|| format!("{a} needs a value"))?;
                flags.values.push((a.clone(), v.clone()));
            } else if switches.contains(&a.as_str()) {
                flags.switches.push(a.clone());
            } else if a.starts_with("--") {
                return Err(format!("unknown option {a}\n{USAGE}"));
            } else {
                flags.rest.push(a.clone());
            }
        }
        Ok(flags)
    }
    fn get(&self, name: &str) -> Option<&str> {
        self.values.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn on(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
}

fn lang_flag(flags: &Flags) -> Res<Lang> {
    flags.get("--lang").map_or(Ok(Lang::En), Lang::parse)
}

fn report(built: &Built) -> String {
    let fit = if built.keep < 3 { format!(", kept priority <= {}", built.keep) } else { String::new() };
    format!("{} ({} page(s){fit})", built.path.display(), built.pages)
}

fn stdin() -> Res<String> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).map_err(|e| e.to_string())?;
    Ok(text)
}

fn saved(path: &std::path::Path, outcome: Saved) -> String {
    match outcome {
        Saved::Unchanged => "no change".into(),
        Saved::Written { commit: Some(c) } => format!("saved {} ({c})", path.display()),
        Saved::Written { commit: None } => format!("saved {}", path.display()),
    }
}

fn target(slug: Option<&String>) -> Target<'_> {
    slug.map_or(Target::Master, |s| Target::App(s))
}

fn run(args: &[String]) -> Res<()> {
    let Some(verb) = args.first() else { return Err(USAGE.into()) };
    let paths = Paths::from_env()?;
    let tail = &args[1..];
    match verb.as_str() {
        "build" => {
            let f = Flags::parse(tail, &["--profile", "--lang", "--out"], &["--all"])?;
            if f.on("--all") {
                let only = f.get("--lang").map(Lang::parse).transpose()?;
                let mut failed = false;
                for result in cv::build_all(&paths, only)? {
                    match result {
                        Ok(b) => println!("cv: {}", report(&b)),
                        Err(e) => {
                            eprintln!("cv: {e}");
                            failed = true;
                        }
                    }
                }
                return if failed { Err("some builds failed".into()) } else { Ok(()) };
            }
            let profile = f.get("--profile").ok_or(format!("--profile is required (or use --all)\n{USAGE}"))?;
            let built = cv::build_profile(&paths, profile, lang_flag(&f)?, f.get("--out").map(PathBuf::from))?;
            println!("cv: {}", report(&built));
        }
        "watch" => {
            let f = Flags::parse(tail, &["--profile", "--lang", "--out"], &[])?;
            let profile = f.get("--profile").ok_or("--profile is required")?;
            let lang = lang_flag(&f)?;
            let master = cv::load_master(&paths)?;
            if !master.profiles().iter().any(|p| p == profile) {
                return Err(format!("unknown profile '{profile}'"));
            }
            let out = f.get("--out").map(PathBuf::from).unwrap_or_else(|| {
                paths.dist().join(format!("CV_{profile}_{}.pdf", lang.code()))
            });
            println!("cv: watching {} -> {} (ctrl-c to stop)", paths.master().display(), out.display());
            let status = std::process::Command::new("typst")
                .args(["watch", "--root", "/"])
                .args(["--input", &format!("master={}", paths.master().display())])
                .args(["--input", &format!("profile={profile}"), "--input", &format!("lang={}", lang.code())])
                .arg(&paths.template)
                .arg(&out)
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("typst watch failed".into());
            }
        }
        "list-profiles" => {
            for p in cv::load_master(&paths)?.profiles() {
                println!("{p}");
            }
        }
        "status" => {
            if tail.first().map(String::as_str) != Some("--json") {
                return Err(USAGE.into());
            }
            println!("{}", cv::status(&paths)?);
        }
        "app" => app(&paths, tail)?,
        "edit" | "model" => {
            let json = verb == "model";
            let (sub, slug) = (tail.first().map(String::as_str), tail.get(1));
            let target = target(slug);
            let path = target.path(&paths)?;
            match (sub, json) {
                (Some("get"), false) => print!("{}", std::fs::read_to_string(&path).map_err(|e| e.to_string())?),
                (Some("get"), true) => {
                    let value = match target {
                        Target::Master => serde_json::to_value(cv::load_master(&paths)?),
                        Target::App(s) => serde_json::to_value(cv::load_app(&paths, s)?),
                    };
                    println!("{}", value.map_err(|e| e.to_string())?);
                }
                (Some("put"), false) => {
                    println!("cv: {}", saved(&path, cv::write_checked(&paths, &path, &stdin()?, target)?));
                }
                (Some("put"), true) => {
                    let input = stdin()?;
                    let yaml = match target {
                        Target::Master => {
                            let m: Master = serde_json::from_str(&input).map_err(|e| format!("master: {e}"))?;
                            m.to_yaml()?
                        }
                        Target::App(_) => {
                            let a: Application =
                                serde_json::from_str(&input).map_err(|e| format!("application: {e}"))?;
                            a.to_yaml()?
                        }
                    };
                    println!("cv: {}", saved(&path, cv::write_checked(&paths, &path, &yaml, target)?));
                }
                _ => return Err(USAGE.into()),
            }
        }
        "linkedin" => {
            let sub = tail.first().map(String::as_str);
            match sub {
                Some("pull") => {
                    for line in linkedin::pull(&paths, &tail[1..])? {
                        println!("cv: {line}");
                    }
                }
                Some("diff") => print!("{}", linkedin::diff(&paths, &cv::load_master(&paths)?)?),
                Some("sheet") => {
                    let f = Flags::parse(&tail[1..], &["--lang"], &[])?;
                    println!("{}", linkedin::sheet(&cv::load_master(&paths)?, lang_flag(&f)?));
                }
                _ => return Err(USAGE.into()),
            }
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}

fn app(paths: &Paths, tail: &[String]) -> Res<()> {
    match tail.first().map(String::as_str) {
        Some("new") => {
            let f = Flags::parse(&tail[1..], &["--company", "--role", "--profile", "--lang", "--url"], &[])?;
            let slug = f.rest.first().cloned().unwrap_or_default();
            let path = cv::new_app(
                paths,
                NewApp {
                    slug,
                    company: f.get("--company").unwrap_or_default().into(),
                    role: f.get("--role").unwrap_or_default().into(),
                    url: f.get("--url").unwrap_or_default().into(),
                    profile: f.get("--profile").unwrap_or("default").into(),
                    lang: lang_flag(&f)?,
                },
            )?;
            println!("cv: {}", path.display());
        }
        Some("build") => {
            let slug = tail.get(1).ok_or("app build needs a slug")?;
            println!("cv: {}", report(&cv::build_app(paths, slug)?));
        }
        Some("list") => {
            let apps = cv::list_apps(paths)?;
            if tail.get(1).map(String::as_str) == Some("--json") {
                let all: Vec<_> = apps
                    .into_iter()
                    .map(|(slug, a)| {
                        let mut v = serde_json::to_value(a).unwrap_or_default();
                        v["slug"] = serde_json::json!(slug);
                        v
                    })
                    .collect();
                println!("{}", serde_json::Value::Array(all));
                return Ok(());
            }
            println!("{:<24} {:<10} {:<11} COMPANY / ROLE", "SLUG", "STATUS", "UPDATED");
            for (slug, a) in apps {
                let updated = a.log.last().map_or("", |l| l.date.as_str()).to_string();
                println!("{slug:<24} {:<10} {updated:<11} {} / {}", a.status.name(), a.company, a.role);
            }
        }
        Some("status") => {
            let slug = tail.get(1).ok_or("app status needs a slug")?;
            let status = Status::parse(tail.get(2).map_or("", String::as_str))?;
            cv::set_status(paths, slug, status, tail.get(3).cloned())?;
            println!("cv: {slug} -> {}", status.name());
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}
