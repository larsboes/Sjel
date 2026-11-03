use std::path::Path;

use comms::extraction_eval::evaluate_file;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "eval/extraction-corpus.json".to_string());
    let report = match evaluate_file(Path::new(&path)) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("extraction evaluation error: {error}");
            std::process::exit(2);
        }
    };

    println!("fixture\tclass\traw→clean\tretained\tuseful\tleakage\tresult");
    for fixture in &report.fixtures {
        println!(
            "{}\t{}\t{}→{}\t{:.1}%\t{:.1}%\t{:.1}%\t{}",
            fixture.id,
            fixture.input_class,
            fixture.raw_chars,
            fixture.normalized_chars,
            fixture.retained_percent,
            fixture.useful_retention_percent,
            fixture.boilerplate_leakage_percent,
            if fixture.passed { "PASS" } else { "FAIL" }
        );
        for failure in &fixture.failures {
            println!("  {failure}");
        }
    }
    println!(
        "gate: useful retention >= {:.1}%, boilerplate leakage <= {:.1}% — {}",
        report.minimum_useful_retention_percent,
        report.maximum_boilerplate_leakage_percent,
        if report.passed { "PASS" } else { "FAIL" }
    );

    if !report.passed {
        std::process::exit(1);
    }
}
