//! The runner for the frozen mail-classification corpus.
//!
//! Prints fixture ids and counts, never a subject, a snippet or a rationale.
//! That is where it differs from `comms-redaction-eval`, whose leaked value IS
//! the finding: here the finding is a stream name.

use std::path::Path;

use comms::config::Config;
use comms::mail_model_eval::evaluate_file;
use comms::store::Store;

fn main() {
    let path = std::env::args()
        .nth(1)
        // Deliberately absent from this repository. The corpus holds real mail
        // and lives in the private overlay beside `comms-redaction-shadow.json`;
        // the operator passes that path.
        .unwrap_or_else(|| "eval/mail-stream-corpus.json".to_string());
    let cfg = Config::load();
    let store = match Store::open(&cfg.database_path) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("mail model evaluation error: could not open the store: {error}");
            std::process::exit(2);
        }
    };
    let report = match evaluate_file(Path::new(&path), &store) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("mail model evaluation error: {error}");
            std::process::exit(2);
        }
    };

    println!("label\tproposed\tn");
    for ((label, proposal), n) in &report.confusion {
        println!("{label}\t{proposal}\t{n}");
    }

    println!(
        "\n{} of {} fixture(s) scored, {} skipped",
        report.fixtures,
        report.declared,
        report.skipped_fixtures()
    );
    for (reason, ids) in &report.skipped {
        println!("  {}", reason.describe());
        // Ids only. A triage id is a Gmail thread id, which the rest of this
        // capability already logs.
        println!("    {}", ids.join(", "));
    }

    // An empty measurement is a failure, not a perfect score. A corpus whose
    // rows have all been re-swept, or which was written before any pass ran,
    // is skipped whole — and printing "agreement 0/0 = 100%, PASS" over it is
    // the gate reporting on a corpus it never read.
    if report.empty_measurement() {
        println!("\ngate: FAIL — no fixture was measured");
        if report.skipped.is_empty() {
            println!("  the corpus declared no fixture to check");
        }
        std::process::exit(1);
    }

    println!(
        "\ncontrol  (the deterministic rules) {:.1}%",
        report.control_agreement_percent()
    );
    println!(
        "model    (the local rung)          {:.1}%",
        report.model_agreement_percent()
    );
    for (language, (scored, agreed)) in &report.by_language {
        println!(
            "  {language}\t{agreed}/{scored}\t{:.1}%",
            *agreed as f64 * 100.0 / *scored as f64
        );
    }
    println!(
        "false eviction from aktiv          {:.1}%  ({} of {} correctly-aktiv mails moved out)",
        report.false_eviction_percent(),
        report.false_evictions,
        report.aktiv_labels
    );
    if report.urgency_scored > 0 {
        println!(
            "urgency band error                 {:.2} bands over {} fixture(s), overstated {:.1}%",
            report.urgency_band_error(),
            report.urgency_scored,
            report.urgency_overstatement_percent()
        );
    } else {
        println!("urgency                            not labelled in this corpus");
    }
    if let Some(producer) = &report.producer {
        println!("producer                           {producer}");
    }

    println!();
    for (claim, passed) in report.verdicts() {
        println!("gate: {claim} — {}", if passed { "PASS" } else { "FAIL" });
    }
    if report.minimum_agreement_percent.is_none() {
        println!(
            "gate: no agreement threshold declared — this run measures, it does not judge. \
             Write the measured number into acceptance.minimum_agreement_percent AFTER reading it."
        );
    }
    if report.max_urgency_band_error.is_none() {
        println!(
            "gate: no urgency threshold declared — urgency stays display-only and ranks nothing."
        );
    }

    if !report.passed() {
        std::process::exit(1);
    }
}
