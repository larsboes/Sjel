use super::types::{BuildOutput, Diagnostic};

/// Reads cargo's `--message-format=json-render-diagnostics` output. It arrives on stdout together
/// with whatever else the install script prints, so a line that is not a record is kept as text.
pub fn read_cargo_output(stdout: &str) -> BuildOutput {
    let mut output = BuildOutput::default();
    for line in stdout.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            output.log.push_str(line);
            output.log.push('\n');
            continue;
        };
        if record["reason"] != "compiler-message" {
            continue;
        }
        let message = &record["message"];
        if let Some(rendered) = message["rendered"].as_str() {
            output.log.push_str(rendered);
        }
        if message["level"] != "error" {
            continue;
        }
        let span = message["spans"]
            .as_array()
            .and_then(|spans| spans.iter().find(|span| span["is_primary"] == true))
            .or_else(|| message["spans"].as_array().and_then(|spans| spans.first()));
        let text = |value: &serde_json::Value, key: &str| {
            value[key].as_str().unwrap_or_default().to_owned()
        };
        output.errors.push(Diagnostic {
            code: message["code"]["code"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            file: span.map(|span| text(span, "file_name")).unwrap_or_default(),
            line: span
                .and_then(|span| span["line_start"].as_u64())
                .unwrap_or(0) as u32,
            message: text(message, "message"),
        });
    }
    output
}
