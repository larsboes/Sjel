//! Pulling a chartable table out of prose, and refusing to draw one that the
//! source does not actually contain.
//!
//! ## The blocker was never rendering
//!
//! A feed item is text. "Chart this article" is not a rendering problem, it is
//! an extraction problem, and the extraction has one failure mode that matters:
//! a model asked for numbers will produce plausible ones. Every value therefore
//! has to appear **literally in the source** before it is allowed into a figure.
//! [`value_appears_in`] is that gate, and it is deterministic — no second
//! model pass grading the first one.
//!
//! ## One measure, one series, on purpose
//!
//! The palette these charts are drawn in is a print palette: low chroma, chosen
//! so figures read well on paper. Run it through a categorical-separation check
//! and even two of its hues fail — `warm_dark` against `teal_dark` is ΔE 9.7 for
//! *normal* vision, under the 15 floor, before colour-vision deficiency is
//! considered. It cannot carry identity, so a chart drawn from it must not need
//! to. A single measure needs no categorical scale and no legend, and it is also
//! what prose actually yields: "+23.8 for basic, +36.2 for expert" is one
//! measure over two categories.
//!
//! A table that would need a second series is refused rather than folded into an
//! encoding the palette cannot honestly support.
//!
//! ## The form is derived, not chosen
//!
//! There is no second model call to pick a chart type. The data's job picks the
//! form: ordered categories get a line, everything else gets bars. One less
//! model output to validate, and the same answer every time.

// This is an ordinary submodule of sjel-summarize; keep sibling imports grouped
// through the parent module.
use super::{complete, truncate, Outcome, Reach, Target, INPUT_CAP};

/// Bump with any change to [`chart_prompt`] or the extraction contract.
pub const CHART_PROMPT_REVISION: &str = "content-chart-v1-single-measure";

/// Under two points there is no shape to see; past two dozen the categories stop
/// being readable in a reader-column-width figure.
pub const MIN_CHART_ROWS: usize = 2;
pub const MAX_CHART_ROWS: usize = 24;
const MAX_LABEL_CHARS: usize = 48;
/// `note` is a sentence, not an axis label, and the label bound cut it mid-word
/// on the first real extraction ("across 15 s").
const MAX_NOTE_CHARS: usize = 220;
/// A caption, likewise. The same first extraction lost the end of "…Output
/// Quality". Axis labels stay short because an axis has no room; a caption has
/// the figure's whole width.
const MAX_TITLE_CHARS: usize = 120;

/// Bar or line. Derived from the categories, never asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Bar,
    Line,
}

impl Mark {
    pub fn as_str(self) -> &'static str {
        match self {
            Mark::Bar => "bar",
            Mark::Line => "line",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartRow {
    pub category: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartData {
    pub title: String,
    pub category_label: String,
    pub measure_label: String,
    pub unit: Option<String>,
    pub mark: Mark,
    pub rows: Vec<ChartRow>,
    /// What the model says the numbers mean. Kept because a figure without its
    /// caveat is a figure that overstates.
    pub note: String,
}

impl ChartData {
    /// The stored and wire form. Plain JSON rather than a Vega-Lite spec: the
    /// dashboard compiles this into one, which is what keeps the model out of
    /// the rendering layer entirely.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "title": self.title,
            "category_label": self.category_label,
            "measure_label": self.measure_label,
            "unit": self.unit,
            "mark": self.mark.as_str(),
            "note": self.note,
            "rows": self.rows.iter().map(|row| serde_json::json!({
                "category": row.category,
                "value": row.value,
            })).collect::<Vec<_>>(),
        })
    }
}

pub fn chart_prompt(input: &str) -> String {
    format!(
        "Find the one set of numbers in the following content that is worth drawing as a chart, \
         and return it as JSON. Answer with a single JSON object and nothing else.\n\n\
         The shape:\n\
         {{\"has_data\": true, \"title\": \"...\", \"category_label\": \"...\", \
         \"measure_label\": \"...\", \"unit\": \"%\" or null, \"note\": \"...\", \
         \"rows\": [{{\"category\": \"...\", \"value\": 12.3}}]}}\n\n\
         Rules you must follow:\n\
         - Every `value` must be a number that appears **verbatim in the content**. Do not \
           compute, convert, round, or estimate one. A number you cannot point at in the text \
           does not belong in the answer.\n\
         - Exactly one measure. If the content compares two different quantities, pick the more \
           interesting one and leave the other out.\n\
         - Between {MIN_CHART_ROWS} and {MAX_CHART_ROWS} rows. Fewer than {MIN_CHART_ROWS} \
           comparable numbers means there is nothing to draw.\n\
         - `note` is one short sentence saying what these numbers are and where they came from.\n\
         - If the content has no comparable set of numbers — most articles do not — answer \
           exactly {{\"has_data\": false}}. That is a correct answer, not a failure.\n\n\
         Content:\n{input}"
    )
}

/// Whether a number the model reported can actually be found in the source.
///
/// Deliberately conservative: a value that is real but written in a form not
/// listed here is dropped, which costs a row. The opposite mistake puts an
/// invented number in a figure, which is the thing this whole module exists to
/// prevent.
///
/// The renderings tried are the ones prose actually uses: the plain decimal, the
/// German decimal comma, and — for a whole number — the bare integer.
pub fn value_appears_in(value: f64, source: &str) -> bool {
    let mut candidates: Vec<String> = Vec::new();
    let plain = format!("{value}");
    candidates.push(plain.clone());
    candidates.push(plain.replace('.', ","));
    if value.fract() == 0.0 {
        let integer = format!("{}", value as i64);
        candidates.push(integer.clone());
        // Thousands separators, both conventions.
        if value.abs() >= 1_000.0 {
            candidates.push(group_thousands(&integer, ','));
            candidates.push(group_thousands(&integer, '.'));
            candidates.push(group_thousands(&integer, ' '));
        }
    }
    candidates
        .iter()
        .any(|candidate| source.contains(candidate))
}

fn group_thousands(digits: &str, separator: char) -> String {
    let (sign, body) = match digits.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", digits),
    };
    let grouped: String = body
        .chars()
        .rev()
        .enumerate()
        .flat_map(|(index, digit)| {
            let mut out = vec![digit];
            if index % 3 == 2 && index + 1 < body.len() {
                out.push(separator);
            }
            out
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{sign}{grouped}")
}

/// Categories that read as an ordered sequence get a line; anything else gets
/// bars. Years, versions and plain numbers are sequences; product names are not.
fn derive_mark(rows: &[ChartRow]) -> Mark {
    let ordered = rows
        .iter()
        .all(|row| row.category.trim().replace(',', ".").parse::<f64>().is_ok());
    if ordered && rows.len() >= 3 {
        Mark::Line
    } else {
        Mark::Bar
    }
}

fn bounded(value: &str, cap: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(cap)
        .collect::<String>()
        .trim()
        .to_string()
}

fn bounded_label(value: &str) -> String {
    bounded(value, MAX_LABEL_CHARS)
}

/// Pull the JSON object out of a model answer, tolerating a fence or a stray
/// sentence around it.
fn json_object(answer: &str) -> Result<serde_json::Value, String> {
    let body = match answer.split_once("```") {
        Some((_, rest)) => {
            let rest = rest.strip_prefix("json").unwrap_or(rest);
            rest.split_once("```")
                .map(|(inside, _)| inside)
                .ok_or_else(|| "unterminated code fence".to_string())?
        }
        None => answer,
    };
    let start = body.find('{').ok_or_else(|| "no JSON object".to_string())?;
    let end = body
        .rfind('}')
        .ok_or_else(|| "no JSON object".to_string())?;
    if end <= start {
        return Err("no JSON object".into());
    }
    serde_json::from_str(&body[start..=end]).map_err(|error| error.to_string())
}

/// Validate a model answer into a [`ChartData`], or say why not.
///
/// `Ok(None)` is the honest "this content has no chart in it", which is the
/// answer for most articles and is not an error.
pub fn parse_chart(answer: &str, source: &str) -> Result<Option<ChartData>, String> {
    let value = json_object(answer)?;
    if value.get("has_data").and_then(serde_json::Value::as_bool) == Some(false) {
        return Ok(None);
    }
    let text = |key: &str| -> String {
        bounded_label(
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
        )
    };

    let raw_rows = value
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "no rows".to_string())?;
    if raw_rows.len() > MAX_CHART_ROWS * 4 {
        return Err(format!("{} rows is not a figure", raw_rows.len()));
    }

    let mut rows: Vec<ChartRow> = Vec::new();
    let mut unverified = 0usize;
    for raw in raw_rows.iter().take(MAX_CHART_ROWS * 4) {
        let category = bounded_label(
            raw.get("category")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
        );
        let Some(number) = raw.get("value").and_then(serde_json::Value::as_f64) else {
            continue;
        };
        if category.is_empty() || !number.is_finite() {
            continue;
        }
        // The gate. A number nobody can point at in the source is not a
        // measurement, it is the model being helpful.
        if !value_appears_in(number, source) {
            unverified += 1;
            continue;
        }
        rows.push(ChartRow {
            category,
            value: number,
        });
        if rows.len() == MAX_CHART_ROWS {
            break;
        }
    }

    if rows.len() < MIN_CHART_ROWS {
        return Err(if unverified > 0 {
            format!(
                "{unverified} of the reported values do not appear in the source; too few left to draw"
            )
        } else {
            "not enough comparable numbers to draw".into()
        });
    }

    let mark = derive_mark(&rows);
    Ok(Some(ChartData {
        title: bounded(
            value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            MAX_TITLE_CHARS,
        ),
        category_label: text("category_label"),
        measure_label: text("measure_label"),
        unit: value
            .get("unit")
            .and_then(serde_json::Value::as_str)
            .map(bounded_label)
            .filter(|unit| !unit.is_empty()),
        mark,
        rows,
        note: bounded(
            value
                .get("note")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            MAX_NOTE_CHARS,
        ),
    }))
}

/// Extract a chartable table from `text`, verified against it.
///
/// Same remote refusal as the digest: a chart of a private mail is still that
/// mail. [`Outcome::SkippedShort`] carries "there is nothing chartable here",
/// which for most content is the right answer.
pub fn chart(target: Option<&Target>, text: &str, reach: Reach) -> Outcome {
    let Some(target) = target else {
        return Outcome::Unconfigured;
    };
    if !reach.admits(target) {
        return Outcome::RemoteRefused;
    }
    if text.trim().is_empty() {
        return Outcome::SkippedShort;
    }
    let input = truncate(text, INPUT_CAP);
    match complete(target, &chart_prompt(&input), 900) {
        // Verification runs against the *capped* input, not the full source:
        // claiming a number is present because it sits in text the model was
        // never shown would make the gate a formality.
        Outcome::Ok(answer) => match parse_chart(&answer, &input) {
            Ok(Some(data)) => Outcome::Ok(data.to_json().to_string()),
            Ok(None) => Outcome::SkippedShort,
            Err(reason) => Outcome::ModelError(reason),
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "An author-measured A/B test (n=15) showed an average quality \
         improvement of +60% (from 49.5 to 79.3). Effectiveness ranges from +23.8 for basic \
         to +36.2 for expert tasks, with 1,204 runs in total.";

    #[test]
    fn a_number_is_found_in_the_forms_prose_actually_uses() {
        assert!(value_appears_in(23.8, SOURCE));
        assert!(value_appears_in(36.2, SOURCE));
        assert!(value_appears_in(60.0, SOURCE));
        assert!(value_appears_in(1204.0, SOURCE), "thousands separator");
        assert!(
            value_appears_in(12.3, "der Wert lag bei 12,3 Prozent"),
            "decimal comma"
        );
    }

    /// The whole point. A model that reports a plausible number nobody wrote
    /// must not get it into a figure.
    #[test]
    fn an_invented_number_is_not_found() {
        for invented in [23.9, 61.0, 100.5, 0.238] {
            assert!(
                !value_appears_in(invented, SOURCE),
                "{invented} should not verify"
            );
        }
    }

    #[test]
    fn a_verified_table_parses_and_derives_bars() {
        let answer = r#"{"has_data":true,"title":"Effectiveness by difficulty",
            "category_label":"Task difficulty","measure_label":"Improvement","unit":"%",
            "note":"Author-measured A/B test, n=15.",
            "rows":[{"category":"basic","value":23.8},{"category":"expert","value":36.2}]}"#;
        let data = parse_chart(answer, SOURCE).unwrap().unwrap();
        assert_eq!(data.mark, Mark::Bar);
        assert_eq!(data.rows.len(), 2);
        assert_eq!(data.unit.as_deref(), Some("%"));
        assert_eq!(data.rows[1].value, 36.2);
    }

    #[test]
    fn ordered_categories_become_a_line() {
        let source = "2024: 10, 2025: 20, 2026: 30";
        let answer = r#"{"has_data":true,"title":"t","category_label":"Year",
            "measure_label":"Count","unit":null,"note":"n",
            "rows":[{"category":"2024","value":10},{"category":"2025","value":20},
                    {"category":"2026","value":30}]}"#;
        assert_eq!(
            parse_chart(answer, source).unwrap().unwrap().mark,
            Mark::Line
        );
    }

    /// Two ordered points are a comparison, not a trend; a line through two dots
    /// implies everything between them.
    #[test]
    fn two_points_stay_bars_even_when_ordered() {
        let source = "2025: 10, 2026: 30";
        let answer = r#"{"has_data":true,"title":"t","category_label":"Year",
            "measure_label":"Count","unit":null,"note":"n",
            "rows":[{"category":"2025","value":10},{"category":"2026","value":30}]}"#;
        assert_eq!(
            parse_chart(answer, source).unwrap().unwrap().mark,
            Mark::Bar
        );
    }

    #[test]
    fn rows_the_source_does_not_support_are_dropped_and_reported() {
        let answer = r#"{"has_data":true,"title":"t","category_label":"c",
            "measure_label":"m","unit":null,"note":"n",
            "rows":[{"category":"basic","value":23.8},{"category":"made up","value":99.9},
                    {"category":"also made up","value":88.8}]}"#;
        let error = parse_chart(answer, SOURCE).unwrap_err();
        assert!(error.contains("do not appear in the source"), "{error}");
    }

    /// "Nothing to chart here" is the answer for most articles, and it is not an
    /// error — a reader that showed an error for every ordinary blog post would
    /// train the operator to ignore the state entirely.
    #[test]
    fn no_data_is_a_verdict_not_a_failure() {
        assert!(parse_chart(r#"{"has_data": false}"#, SOURCE)
            .unwrap()
            .is_none());
        assert!(parse_chart("```json\n{\"has_data\": false}\n```", SOURCE)
            .unwrap()
            .is_none());
    }

    #[test]
    fn prose_instead_of_json_is_rejected() {
        for answer in ["I'd be happy to help with that!", "", "```json\n{oops"] {
            assert!(parse_chart(answer, SOURCE).is_err(), "{answer:?}");
        }
    }

    /// A caption cut mid-word reads as a bug, and the first live extraction
    /// produced exactly that: "across 15 s".
    #[test]
    fn the_note_is_a_sentence_and_not_bounded_like_a_label() {
        let note = "Results from a controlled experiment across 15 sessions, reported by the \
                    author rather than measured independently.";
        let answer = format!(
            r#"{{"has_data":true,"title":"t","category_label":"c","measure_label":"m",
            "unit":null,"note":"{note}",
            "rows":[{{"category":"basic","value":23.8}},{{"category":"expert","value":36.2}}]}}"#
        );
        let data = parse_chart(&answer, SOURCE).unwrap().unwrap();
        assert!(
            data.note.ends_with("independently."),
            "cut short: {}",
            data.note
        );
        assert!(data.note.chars().count() > MAX_LABEL_CHARS);
    }

    #[test]
    fn labels_are_bounded_and_stripped_of_control_characters() {
        let long = "x".repeat(200);
        let answer = format!(
            r#"{{"has_data":true,"title":"{long}","category_label":"c","measure_label":"m",
            "unit":null,"note":"n",
            "rows":[{{"category":"basic","value":23.8}},{{"category":"expert","value":36.2}}]}}"#
        );
        let data = parse_chart(&answer, SOURCE).unwrap().unwrap();
        assert_eq!(data.title.chars().count(), MAX_TITLE_CHARS);
        assert_eq!(data.category_label, "c");
    }

    #[test]
    fn the_json_shape_is_what_the_reader_compiles() {
        let answer = r#"{"has_data":true,"title":"t","category_label":"c","measure_label":"m",
            "unit":"%","note":"n",
            "rows":[{"category":"basic","value":23.8},{"category":"expert","value":36.2}]}"#;
        let value = parse_chart(answer, SOURCE).unwrap().unwrap().to_json();
        assert_eq!(value["mark"], "bar");
        assert_eq!(value["rows"][0]["category"], "basic");
        assert_eq!(value["rows"][1]["value"], 36.2);
        assert_eq!(value["unit"], "%");
    }

    #[test]
    fn a_non_loopback_target_is_refused_for_restricted_content() {
        let cloud = Target {
            endpoint: "https://api.example.com/v1/chat/completions".into(),
            model: "m".into(),
            api_key: None,
            loopback: false,
            operator_owned: false,
            gate: None,
        };
        assert_eq!(
            chart(Some(&cloud), SOURCE, Reach::LoopbackOnly),
            Outcome::RemoteRefused
        );
    }
}
