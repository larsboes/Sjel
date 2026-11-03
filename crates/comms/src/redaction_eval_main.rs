use std::path::Path;

use comms::redaction_eval::{evaluate_file_with, Mode};

/// `comms-redaction-eval [--pseudonymized] [corpus.json]`. The flag runs the corpus through
/// `prepare_pseudonymized` instead of `prepare`; the gate is the same.
fn main() {
    let mut mode = Mode::Destructive;
    let mut path = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--pseudonymized" => mode = Mode::Pseudonymized,
            _ => path = Some(arg),
        }
    }
    let path = path.unwrap_or_else(|| "eval/redaction-corpus.json".to_string());
    println!(
        "mode: {}",
        match mode {
            Mode::Destructive => "prepare (destructive markers)",
            Mode::Pseudonymized => "prepare_pseudonymized (reversible tokens)",
        }
    );
    let report = match evaluate_file_with(Path::new(&path), mode) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("redaction evaluation error: {error}");
            std::process::exit(2);
        }
    };

    println!("type\tlang\tcaught/total\trecall");
    for ((entity_type, language), (caught, total)) in &report.by_type_language {
        println!(
            "{entity_type}\t{language}\t{caught}/{total}\t{:.1}%",
            *caught as f64 * 100.0 / *total as f64
        );
    }
    if !report.leaks.is_empty() {
        println!("\nleaked (present in the document that would be sent):");
        for leak in &report.leaks {
            println!(
                "  {} [{}] {}: {}",
                leak.fixture, leak.language, leak.entity_type, leak.value
            );
        }
    }
    println!(
        "\n{} fixture(s) measured, {} skipped, {} marker(s) written",
        report.fixtures,
        report.skipped_fixtures(),
        report.markers_written
    );

    // An empty measurement is a failure, not a perfect score. `prepare` refuses every class
    // that is not c0 or c1, so a corpus written against an older vocabulary is skipped whole —
    // and printing "recall 0/0 = 100%, PASS" over it is the gate reporting on a corpus it never
    // read. The refused classes are named, because that is the fix.
    if report.empty_measurement() {
        println!("gate: FAIL — no label was measured");
        if report.skipped.is_empty() {
            println!("  the corpus declared no fixture to check");
        }
        for reason in report.skip_reasons() {
            println!("  {reason}");
        }
        std::process::exit(1);
    }

    println!(
        "recall {}/{} = {:.1}%",
        report.caught(),
        report.total(),
        report.recall_percent()
    );
    match report.minimum_recall_percent {
        Some(minimum) => println!(
            "gate: recall >= {minimum:.1}% — {}",
            if report.passed() { "PASS" } else { "FAIL" }
        ),
        None => println!("gate: none declared — this run measures, it does not judge"),
    }

    if !report.passed() {
        std::process::exit(1);
    }
}
