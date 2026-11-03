//! The runner for the frozen digest-quality corpus (PRD D16).
//!
//! Prints ids, producers and counts, never a digest and never a subject. The finding here is a
//! rate, not a sentence — unlike `comms-redaction-eval`, whose leaked value IS the finding.
//!
//! Makes no model call. It joins hand-written judgements to digests already in the store, so
//! the number is the same on every run and the scoring cannot drift from the generation.

use std::path::Path;

use comms::config::Config;
use comms::digest_eval::evaluate_file;
use comms::store::Store;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        // Deliberately absent from this repository. The corpus quotes real articles and real
        // mail, so it lives in the private overlay beside the other two.
        "eval/digest-corpus.json".to_string()
    });
    let cfg = Config::load();
    let store = match Store::open(&cfg.database_path) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("digest evaluation error: could not open the store: {error}");
            std::process::exit(2);
        }
    };
    let report = match evaluate_file(Path::new(&path), &store) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("digest evaluation error: {error}");
            std::process::exit(2);
        }
    };

    println!("producer\tscored\tunfaithful\tunfaithful%\tmean useful band");
    for (producer, score) in &report.by_producer {
        let band = score
            .mean_useful_band()
            .map(|mean| format!("{mean:.2}"))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{producer}\t{}\t{}\t{:.1}\t{band}",
            score.scored,
            score.unfaithful,
            score.unfaithful_percent()
        );
    }

    println!();
    println!("declared {}", report.declared);
    println!("scored   {}", report.scored);
    for (reason, count) in &report.skipped {
        println!("skipped  {count}\t{}", reason.as_str());
    }
    println!(
        "unfaithful {} of {} = {:.1}% (gate: <= {:.1}%)",
        report.unfaithful,
        report.scored,
        report.unfaithful_percent(),
        report.max_unfaithful_percent
    );
    match report.minimum_useful_percent {
        Some(minimum) => println!("minimum_useful_percent {minimum:.1} — reported, does not gate"),
        None => println!(
            "minimum_useful_percent is null: write it in after this run has been read, never before"
        ),
    }

    if report.passed() {
        println!("PASS");
    } else if report.scored == 0 {
        // Named separately, because "nothing was measured" and "the measurement failed" are
        // different facts and only one of them is about the digests.
        println!("FAIL — nothing was scored. Every fixture was skipped, so this is not a 0%.");
        std::process::exit(1);
    } else {
        println!("FAIL");
        std::process::exit(1);
    }
}
