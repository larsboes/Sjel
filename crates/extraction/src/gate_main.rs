//! Runs one OCR engine over the frozen DE/EN corpus and prints the scorecard.
//!
//! ```sh
//! cargo run -p sjel-extraction --bin extraction-gate -- libs/extraction/eval/ocr-corpus.json
//! ```
//!
//! From the repository root, which is where `cargo run -p` is typed: `cargo run`
//! does not change the working directory, and a fixture path is resolved
//! against the corpus file's own parent. The default argument is the same path
//! for the same reason.
//!
//! By hand, never from `cargo test`. Scoring is hermetic and lives in
//! [`sjel_extraction::gate`]; this binary is the half that needs an engine, a
//! host that has it, and the operator's decision to run it — the same shape
//! `comms-extraction-eval` and `bun run-relevance.ts` already have.
//!
//! Exits non-zero unless all three lines pass. The Apple Vision baseline exits
//! non-zero on purpose: it clears prose and fails notation, which is the
//! measurement that put a third rung in the ladder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sjel_extraction::gate::{evaluate, Corpus, Scorecard};
use sjel_extraction::vision;

fn main() {
    let mut corpus_path = PathBuf::from("libs/extraction/eval/ocr-corpus.json");
    let mut engine = vision::ENGINE.to_string();
    let mut record: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--engine" => {
                engine = args
                    .next()
                    .unwrap_or_else(|| usage("--engine needs a name"))
            }
            // Writes what the engine returned, verbatim, so the scoring rule
            // stays testable on a host that cannot run this engine at all.
            "--record" => {
                record = Some(PathBuf::from(
                    args.next()
                        .unwrap_or_else(|| usage("--record needs a path")),
                ))
            }
            "--help" | "-h" => {
                usage("usage: extraction-gate [<corpus.json>] [--engine <name>] [--record <file>]")
            }
            other if other.starts_with('-') => usage(&format!("unknown option {other}")),
            other => corpus_path = PathBuf::from(other),
        }
    }

    let body = std::fs::read_to_string(&corpus_path)
        .unwrap_or_else(|e| fail(&format!("{}: {e}", corpus_path.display())));
    let corpus = Corpus::parse(&body).unwrap_or_else(|e| fail(&e));
    let root = corpus_path.parent().unwrap_or(Path::new("."));

    if engine != vision::ENGINE {
        fail(&format!(
            "no runner for engine {engine:?}. Only {:?} is wired in; a new engine adds its \
             runner here and its record under eval/results/ before it may be adopted.",
            vision::ENGINE
        ));
    }

    // One child process for every page. Vision loads its language assets once,
    // which is the whole reason tools/visocr speaks a batch protocol.
    let paths: Vec<PathBuf> = corpus
        .fixtures
        .iter()
        .map(|fixture| root.join(&fixture.source))
        .collect();
    let read = vision::recognize(&paths).unwrap_or_else(|e| fail(&e.to_string()));

    let outputs: BTreeMap<String, String> = corpus
        .fixtures
        .iter()
        .zip(read.iter())
        .map(|(fixture, (_, text))| (fixture.id.clone(), text.clone()))
        .collect();

    if let Some(path) = record {
        let document = serde_json::json!({
            "_note": "Verbatim engine output over the frozen corpus. Never hand-edited: libs/extraction/src/gate.rs scores this recording in cargo test, so the scoring rule runs on a host with no OCR engine. The run date is in the filename and in the matching record under eval/results/; this file does not repeat it, because a date written twice is a date that can disagree with itself.",
            "engine": engine,
            "corpus": corpus_path.file_name().map(|n| n.to_string_lossy().into_owned()),
            "outputs": outputs,
        });
        std::fs::write(&path, format!("{document:#}\n"))
            .unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
        eprintln!("recorded {} page(s) to {}", outputs.len(), path.display());
    }

    let card = evaluate(&corpus, &engine, &outputs);
    print(&card, &corpus);

    let passed = card.prose_line_passed(&corpus.acceptance)
        && card.notation_line_passed(&corpus.acceptance)
        && card.detector_line_passed(&corpus.acceptance);
    std::process::exit(i32::from(!passed));
}

fn print(card: &Scorecard, corpus: &Corpus) {
    println!("engine: {}", card.engine);
    println!();
    for fixture in &card.fixtures {
        println!(
            "  {:<14} {:<6} {:<5} {:>3}/{:<3} survived   detector {}",
            fixture.id,
            fixture.page_kind,
            fixture.language,
            fixture.survived,
            fixture.judged,
            if fixture.detector_fired {
                "fired"
            } else {
                "quiet"
            },
        );
        for failure in &fixture.failures {
            println!("      ✗ {failure}");
        }
    }
    println!();
    // Two lines, never one number: an engine may take rung 2 on prose alone.
    println!(
        "  prose recall     {:>6.1}%   {}   (rung 2)",
        card.prose_recall_percent,
        verdict(card.prose_line_passed(&corpus.acceptance))
    );
    println!(
        "  notation         {:>6.1}%   {}   (rung 3), {} forbidden confusion(s)",
        card.notation_recall_percent,
        verdict(card.notation_line_passed(&corpus.acceptance)),
        card.notation_confusions
    );
    println!(
        "  detector agrees  {:>6.1}%   {}",
        card.detector_agreement_percent,
        verdict(card.detector_line_passed(&corpus.acceptance))
    );
}

fn verdict(passed: bool) -> &'static str {
    if passed {
        "pass"
    } else {
        "MISS"
    }
}

fn usage(message: &str) -> ! {
    eprintln!("extraction-gate: {message}");
    eprintln!("usage: extraction-gate [<corpus.json>] [--engine <name>] [--record <file>]");
    std::process::exit(2);
}

fn fail(message: &str) -> ! {
    eprintln!("extraction-gate: {message}");
    std::process::exit(2);
}
