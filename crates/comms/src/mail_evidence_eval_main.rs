use std::{
    fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

use comms::mail_evidence_eval::{self, Corpus, Review, Source};

fn main() {
    if let Err(error) = execute() {
        eprintln!("mail evidence evaluation: {error}");
        std::process::exit(2);
    }
}

fn validate_output(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let overlay = sjel_config::overlay_root()
        .ok_or("private overlay unavailable")?
        .canonicalize()?;
    let parent = path
        .parent()
        .ok_or("output needs a parent directory")?
        .canonicalize()?;
    if !parent.starts_with(&overlay) {
        return Err("evaluation artifacts must stay in the private overlay".into());
    }
    if path.exists() {
        return Err("refusing to overwrite an existing evaluation artifact".into());
    }
    Ok(())
}

fn private_write(
    path: &Path,
    value: &impl serde::Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_output(path)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn execute() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 2
        || !matches!(args[0].as_str(), "freeze" | "run")
        || (args[0] == "freeze" && args.len() != 2)
        || (args[0] == "run" && args.len() != 4)
    {
        return Err("usage: comms-mail-evidence-eval freeze CORPUS < reviewed.json; run CORPUS REPORT ROLE < sources.json".into());
    }
    let mut input = String::new();
    validate_output(Path::new(if args[0] == "freeze" {
        &args[1]
    } else {
        &args[2]
    }))?;
    io::stdin()
        .take(4 * 1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 4 * 1024 * 1024 {
        return Err("stdin exceeds 4 MiB".into());
    }
    if args[0] == "freeze" {
        let review: Review = serde_json::from_str(&input).map_err(|_| "invalid review JSON")?;
        let corpus = mail_evidence_eval::freeze(review)?;
        private_write(Path::new(&args[1]), &corpus)?;
        println!(
            "frozen {} cases; no message bodies written",
            corpus.cases.len()
        );
    } else {
        let mut bytes = Vec::new();
        fs::File::open(&args[1])?
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("corpus exceeds 4 MiB".into());
        }
        let corpus: Corpus = serde_json::from_slice(&bytes).map_err(|_| "invalid corpus JSON")?;
        let sources: Vec<Source> =
            serde_json::from_str(&input).map_err(|_| "invalid source JSON")?;
        let cfg = comms::config::Config::load();
        let runtime = sjel_runtime::current().map_err(|_| "runtime unavailable")?;
        let report = mail_evidence_eval::run(&corpus, &sources, &cfg, &runtime, &args[3])?;
        private_write(Path::new(&args[2]), &report)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if !report.failures.is_empty() {
            std::process::exit(1);
        }
    }
    Ok(())
}
