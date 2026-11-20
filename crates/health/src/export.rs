//! Apple Health "Export All Health Data" → one value per metric per day.
//!
//! `export.xml` is often several gigabytes, so it is read line by line and never held whole. The
//! Health app writes each `<Record>` start tag on one line, which is all this reader needs: the
//! attributes it uses are on that line, and child elements (metadata, beat lists) are skipped.
//!
//! The day of a sample is the local date its timestamp carries, so a run in Bonn and a run in
//! Valencia each land on the day the operator lived them. Sleep belongs to the day it ends.

use std::collections::BTreeMap;
use std::io::BufRead;

use chrono::{Datelike, NaiveDate};

/// How the samples of one day become one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    /// Total per source; the source with the largest total wins, so a phone and a watch that
    /// both counted the same steps are not added together.
    Sum,
    Mean,
    Min,
    Max,
    /// The newest sample of the day.
    Last,
}

pub struct Metric {
    pub key: &'static str,
    pub group: &'static str,
    pub unit: &'static str,
    hk: &'static str,
    fold: Fold,
}

const fn m(
    key: &'static str,
    group: &'static str,
    unit: &'static str,
    hk: &'static str,
    fold: Fold,
) -> Metric {
    Metric {
        key,
        group,
        unit,
        hk,
        fold,
    }
}

const Q: &str = "HKQuantityTypeIdentifier";

/// Every quantity metric this capability keeps. Sleep is folded separately (`SLEEP`).
pub const METRICS: &[Metric] = &[
    m("steps", "activity", "count", "StepCount", Fold::Sum),
    m(
        "distance_walking_km",
        "activity",
        "km",
        "DistanceWalkingRunning",
        Fold::Sum,
    ),
    m(
        "flights_climbed",
        "activity",
        "count",
        "FlightsClimbed",
        Fold::Sum,
    ),
    m(
        "active_energy_kcal",
        "activity",
        "kcal",
        "ActiveEnergyBurned",
        Fold::Sum,
    ),
    m(
        "exercise_min",
        "activity",
        "min",
        "AppleExerciseTime",
        Fold::Sum,
    ),
    m("resting_hr", "heart", "bpm", "RestingHeartRate", Fold::Mean),
    m(
        "walking_hr",
        "heart",
        "bpm",
        "WalkingHeartRateAverage",
        Fold::Mean,
    ),
    m(
        "hrv_sdnn_ms",
        "heart",
        "ms",
        "HeartRateVariabilitySDNN",
        Fold::Mean,
    ),
    m("heart_rate_avg", "heart", "bpm", "HeartRate", Fold::Mean),
    m("heart_rate_min", "heart", "bpm", "HeartRate", Fold::Min),
    m("heart_rate_max", "heart", "bpm", "HeartRate", Fold::Max),
    m("body_mass_kg", "body", "kg", "BodyMass", Fold::Last),
    m("body_fat_pct", "body", "%", "BodyFatPercentage", Fold::Last),
    m("lean_mass_kg", "body", "kg", "LeanBodyMass", Fold::Last),
    m("bmi", "body", "kg/m²", "BodyMassIndex", Fold::Last),
];

/// The sleep keys, all in hours. Onset and wake are hours from midnight of the wake day, so
/// 22:30 the evening before is -1.5.
pub const SLEEP: &[(&str, &str)] = &[
    ("sleep_asleep_h", "h"),
    ("sleep_in_bed_h", "h"),
    ("sleep_awake_h", "h"),
    ("sleep_core_h", "h"),
    ("sleep_deep_h", "h"),
    ("sleep_rem_h", "h"),
    ("sleep_onset_h", "h"),
    ("sleep_wake_h", "h"),
];

/// The catalogue's own key and unit for `key`, so a client-sent string maps to a stored one.
pub fn canonical(key: &str) -> Option<(&'static str, &'static str)> {
    METRICS
        .iter()
        .find(|metric| metric.key == key)
        .map(|metric| (metric.key, metric.unit))
        .or_else(|| SLEEP.iter().find(|(k, _)| *k == key).copied())
}

/// The group and unit of a stored key.
pub fn describe(key: &str) -> Option<(&'static str, &'static str)> {
    METRICS
        .iter()
        .find(|metric| metric.key == key)
        .map(|metric| (metric.group, metric.unit))
        .or_else(|| {
            SLEEP
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, unit)| ("sleep", *unit))
        })
}

/// One folded value.
#[derive(Debug, Clone, PartialEq)]
pub struct DayValue {
    pub metric: &'static str,
    pub day: String,
    pub value: f64,
    pub unit: &'static str,
    pub source: String,
}

#[derive(Debug, Default)]
pub struct Summary {
    pub lines: u64,
    pub records: u64,
    pub used: u64,
    pub values: Vec<DayValue>,
}

#[derive(Default)]
struct Acc {
    per_source: BTreeMap<String, f64>,
    sum: f64,
    count: u64,
    min: f64,
    max: f64,
    last: Option<(String, f64, String)>,
}

/// One sleep sample, in hours since 0001-01-01 local time.
struct Segment {
    start: f64,
    end: f64,
    key: &'static str,
}

/// A night is a session whose midpoint lies in the window from 20:00 the evening before to
/// 12:00 on its day.
const NIGHT_STARTS_BEFORE_MIDNIGHT_H: f64 = 4.0;
const NIGHT_WINDOW_H: f64 = 16.0;

/// A gap longer than this between two samples of one source starts a new sleep session.
const SESSION_GAP_H: f64 = 2.0;

#[derive(Default)]
struct Session {
    hours: BTreeMap<&'static str, f64>,
    onset: Option<f64>,
    wake: Option<f64>,
    end: f64,
}

impl Session {
    fn asleep(&self) -> f64 {
        self.hours.get("sleep_asleep_h").copied().unwrap_or(0.0)
    }

    fn in_bed(&self) -> f64 {
        self.hours.get("sleep_in_bed_h").copied().unwrap_or(0.0)
    }

    /// The day whose night this is: its midpoint falls between 20:00 the evening before and
    /// 12:00. `None` for a nap, whose midpoint falls between 12:00 and 20:00.
    fn night_of(&self) -> Option<i64> {
        let start = self.onset.unwrap_or(self.end - self.in_bed());
        let mid = (start + self.wake.unwrap_or(self.end)) / 2.0;
        let shifted = mid + NIGHT_STARTS_BEFORE_MIDNIGHT_H;
        let day = (shifted / 24.0).floor();
        (shifted - day * 24.0 <= NIGHT_WINDOW_H).then_some(day as i64)
    }
}

/// Split one source's samples into sessions. Each session is one sleep, a night or a nap.
fn sessions(mut segments: Vec<Segment>) -> Vec<Session> {
    segments.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut out: Vec<Session> = Vec::new();
    for segment in segments {
        let starts_new = out
            .last()
            .is_none_or(|session| segment.start > session.end + SESSION_GAP_H);
        if starts_new {
            out.push(Session::default());
        }
        let session = out.last_mut().expect("a session exists");
        let hours = segment.end - segment.start;
        *session.hours.entry(segment.key).or_default() += hours;
        if matches!(segment.key, "sleep_core_h" | "sleep_deep_h" | "sleep_rem_h") {
            *session.hours.entry("sleep_asleep_h").or_default() += hours;
        }
        if !matches!(segment.key, "sleep_in_bed_h" | "sleep_awake_h") {
            session.onset = Some(
                session
                    .onset
                    .map_or(segment.start, |o| o.min(segment.start)),
            );
            session.wake = Some(session.wake.map_or(segment.end, |w| w.max(segment.end)));
        }
        session.end = session.end.max(segment.end);
    }
    out
}

/// Read an `export.xml` stream and fold it.
///
/// # Errors
/// The stream cannot be read.
pub fn fold(reader: impl BufRead) -> std::io::Result<Summary> {
    let mut summary = Summary::default();
    let mut quantities: BTreeMap<(&'static str, String), Acc> = BTreeMap::new();
    let mut sleep: BTreeMap<String, Vec<Segment>> = BTreeMap::new();
    for line in reader.lines() {
        let line = line?;
        summary.lines += 1;
        let line = line.trim_start();
        if !line.starts_with("<Record ") {
            continue;
        }
        summary.records += 1;
        let Some(kind) = attr(line, "type") else {
            continue;
        };
        if kind == "HKCategoryTypeIdentifierSleepAnalysis" {
            if sleep_sample(line, &mut sleep).is_some() {
                summary.used += 1;
            }
            continue;
        }
        let Some(hk) = kind.strip_prefix(Q) else {
            continue;
        };
        let (Some(value), Some(unit), Some(start)) = (
            attr(line, "value").and_then(|v| v.parse::<f64>().ok()),
            attr(line, "unit"),
            attr(line, "startDate"),
        ) else {
            continue;
        };
        let Some(day) = start.get(..10) else { continue };
        let source = attr(line, "sourceName").map(unescape).unwrap_or_default();
        let mut used = false;
        for metric in METRICS.iter().filter(|metric| metric.hk == hk) {
            let Some(value) = convert(value, unit, metric.unit) else {
                continue;
            };
            let acc = quantities.entry((metric.key, day.to_owned())).or_default();
            match metric.fold {
                Fold::Sum => *acc.per_source.entry(source.clone()).or_default() += value,
                Fold::Mean => {
                    acc.sum += value;
                    acc.count += 1;
                }
                Fold::Min => {
                    acc.min = if acc.count == 0 {
                        value
                    } else {
                        acc.min.min(value)
                    }
                }
                Fold::Max => {
                    acc.max = if acc.count == 0 {
                        value
                    } else {
                        acc.max.max(value)
                    }
                }
                Fold::Last => {
                    if acc
                        .last
                        .as_ref()
                        .is_none_or(|(at, _, _)| start >= at.as_str())
                    {
                        acc.last = Some((start.to_owned(), value, source.clone()));
                    }
                }
            }
            if matches!(metric.fold, Fold::Min | Fold::Max) {
                acc.count += 1;
            }
            used = true;
        }
        if used {
            summary.used += 1;
        }
    }
    for ((key, day), acc) in quantities {
        let metric = METRICS
            .iter()
            .find(|metric| metric.key == key)
            .expect("known key");
        let (value, source) = match metric.fold {
            Fold::Sum => match acc
                .per_source
                .into_iter()
                .max_by(|a, b| a.1.total_cmp(&b.1))
            {
                Some((source, total)) => (total, source),
                None => continue,
            },
            Fold::Mean if acc.count > 0 => (acc.sum / acc.count as f64, String::new()),
            Fold::Min if acc.count > 0 => (acc.min, String::new()),
            Fold::Max if acc.count > 0 => (acc.max, String::new()),
            Fold::Last => match acc.last {
                Some((_, value, source)) => (value, source),
                None => continue,
            },
            _ => continue,
        };
        summary.values.push(DayValue {
            metric: key,
            day,
            value,
            unit: metric.unit,
            source,
        });
    }
    summary.values.extend(nights(sleep));
    Ok(summary)
}

/// Raw sleep samples to one night per day, by the rule [`fold`] uses: `start` and `end` in the
/// export's `2026-11-18 06:15:00 +0100` form, `stage` the part after
/// `HKCategoryValueSleepAnalysis` (`AsleepCore`, `InBed` and so on). Unknown stages are skipped.
pub fn fold_sleep<'a>(
    samples: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a str)>,
) -> Vec<DayValue> {
    let mut sleep: BTreeMap<String, Vec<Segment>> = BTreeMap::new();
    for (source, start, end, stage) in samples {
        push_segment(&mut sleep, source.to_owned(), start, end, stage);
    }
    nights(sleep)
}

fn nights(sleep: BTreeMap<String, Vec<Segment>>) -> Vec<DayValue> {
    let mut values = Vec::new();
    // Per day, the night: of the sessions in its night window, the one with the most sleep across
    // all sources, then the longest in-bed time. Naps and other sources' copies are dropped.
    let mut best: BTreeMap<i64, (String, Session)> = BTreeMap::new();
    for (source, segments) in sleep {
        for session in sessions(segments) {
            let Some(day) = session.night_of() else {
                continue;
            };
            let better = best.get(&day).is_none_or(|(_, kept)| {
                (session.asleep(), session.in_bed()) > (kept.asleep(), kept.in_bed())
            });
            if better {
                best.insert(day, (source.clone(), session));
            }
        }
    }
    for (day, (source, session)) in best {
        let Some(date) = NaiveDate::from_num_days_from_ce_opt(i32::try_from(day).unwrap_or(0))
        else {
            continue;
        };
        let midnight = day as f64 * 24.0;
        let day = date.to_string();
        let mut push = |key: &'static str, value: f64| {
            values.push(DayValue {
                metric: key,
                day: day.clone(),
                value,
                unit: "h",
                source: source.clone(),
            });
        };
        for (key, hours) in &session.hours {
            push(key, *hours);
        }
        if let Some(onset) = session.onset {
            push("sleep_onset_h", onset - midnight);
        }
        if let Some(wake) = session.wake {
            push("sleep_wake_h", wake - midnight);
        }
    }
    values
}

fn sleep_sample(line: &str, sleep: &mut BTreeMap<String, Vec<Segment>>) -> Option<()> {
    let stage = attr(line, "value")?.strip_prefix("HKCategoryValueSleepAnalysis")?;
    let source = attr(line, "sourceName").map(unescape).unwrap_or_default();
    push_segment(
        sleep,
        source,
        attr(line, "startDate")?,
        attr(line, "endDate")?,
        stage,
    )
}

fn push_segment(
    sleep: &mut BTreeMap<String, Vec<Segment>>,
    source: String,
    start: &str,
    end: &str,
    stage: &str,
) -> Option<()> {
    let start = hours_since_ce(start)?;
    let end = hours_since_ce(end)?;
    if !(0.0..=24.0).contains(&(end - start)) {
        return None;
    }
    let key = match stage {
        "InBed" => "sleep_in_bed_h",
        "Awake" => "sleep_awake_h",
        "AsleepCore" => "sleep_core_h",
        "AsleepDeep" => "sleep_deep_h",
        "AsleepREM" => "sleep_rem_h",
        "Asleep" | "AsleepUnspecified" => "sleep_asleep_h",
        _ => return None,
    };
    sleep
        .entry(source)
        .or_default()
        .push(Segment { start, end, key });
    Some(())
}

/// Local wall-clock hours since 0001-01-01, so samples on both sides of midnight compare.
fn hours_since_ce(stamp: &str) -> Option<f64> {
    let (day, hour) = clock(stamp)?;
    Some(f64::from(day.num_days_from_ce()) * 24.0 + hour)
}

/// `2024-08-23 07:12:03 +0200` → the local date and the local hour as a fraction.
fn clock(stamp: &str) -> Option<(NaiveDate, f64)> {
    let day = NaiveDate::parse_from_str(stamp.get(..10)?, "%Y-%m-%d").ok()?;
    let hour: f64 = stamp.get(11..13)?.parse().ok()?;
    let minute: f64 = stamp.get(14..16)?.parse().ok()?;
    let second: f64 = stamp.get(17..19)?.parse().ok()?;
    Some((day, hour + minute / 60.0 + second / 3600.0))
}

/// Convert to the metric unit this capability stores. `None` for a unit it does not know.
fn convert(value: f64, from: &str, to: &str) -> Option<f64> {
    Some(match (from, to) {
        // Apple stores a percentage as a fraction: 0.18 is 18 %.
        ("%", "%") => value * 100.0,
        (a, b) if a == b => value,
        ("count/min", "bpm") => value,
        ("m", "km") => value / 1000.0,
        ("mi", "km") => value * 1.609_344,
        ("kJ", "kcal") => value / 4.184,
        ("Cal", "kcal") => value,
        ("lb", "kg") => value * 0.453_592_37,
        ("g", "kg") => value / 1000.0,
        ("count", "kg/m²") => value,
        _ => return None,
    })
}

/// The value of `name="…"` in a start tag.
fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = line.find(&needle)? + needle.len();
    let len = line[start..].find('"')?;
    Some(&line[start..start + len])
}

fn unescape(raw: &str) -> String {
    raw.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<HealthData locale="de_DE">
 <Record type="HKQuantityTypeIdentifierStepCount" sourceName="Phone" unit="count" startDate="2026-11-18 08:00:00 +0100" endDate="2026-11-18 08:10:00 +0100" value="1200"/>
 <Record type="HKQuantityTypeIdentifierStepCount" sourceName="Phone" unit="count" startDate="2026-11-18 12:00:00 +0100" endDate="2026-11-18 12:10:00 +0100" value="800"/>
 <Record type="HKQuantityTypeIdentifierStepCount" sourceName="Watch" unit="count" startDate="2026-11-18 08:00:00 +0100" endDate="2026-11-18 08:10:00 +0100" value="1500"/>
 <Record type="HKQuantityTypeIdentifierDistanceWalkingRunning" sourceName="Phone" unit="mi" startDate="2026-11-18 08:00:00 +0100" endDate="2026-11-18 08:10:00 +0100" value="1"/>
 <Record type="HKQuantityTypeIdentifierHeartRate" sourceName="Watch" unit="count/min" startDate="2026-11-18 09:00:00 +0100" endDate="2026-11-18 09:00:00 +0100" value="60">
  <MetadataEntry key="HKMetadataKeyHeartRateMotionContext" value="1"/>
 </Record>
 <Record type="HKQuantityTypeIdentifierHeartRate" sourceName="Watch" unit="count/min" startDate="2026-11-18 10:00:00 +0100" endDate="2026-11-18 10:00:00 +0100" value="100"/>
 <Record type="HKQuantityTypeIdentifierBodyMass" sourceName="Scale &amp; Co" unit="kg" startDate="2026-11-18 07:00:00 +0100" endDate="2026-11-18 07:00:00 +0100" value="80"/>
 <Record type="HKQuantityTypeIdentifierBodyMass" sourceName="Scale &amp; Co" unit="kg" startDate="2026-11-18 21:00:00 +0100" endDate="2026-11-18 21:00:00 +0100" value="81"/>
 <Record type="HKQuantityTypeIdentifierBodyFatPercentage" sourceName="Scale" unit="%" startDate="2026-11-18 07:00:00 +0100" endDate="2026-11-18 07:00:00 +0100" value="0.18"/>
 <Record type="HKCategoryTypeIdentifierSleepAnalysis" sourceName="Watch" startDate="2026-11-17 23:00:00 +0100" endDate="2026-11-18 03:00:00 +0100" value="HKCategoryValueSleepAnalysisAsleepCore"/>
 <Record type="HKCategoryTypeIdentifierSleepAnalysis" sourceName="Watch" startDate="2026-11-18 03:00:00 +0100" endDate="2026-11-18 04:30:00 +0100" value="HKCategoryValueSleepAnalysisAsleepDeep"/>
 <Record type="HKCategoryTypeIdentifierSleepAnalysis" sourceName="Watch" startDate="2026-11-18 04:30:00 +0100" endDate="2026-11-18 05:00:00 +0100" value="HKCategoryValueSleepAnalysisAwake"/>
 <Record type="HKCategoryTypeIdentifierSleepAnalysis" sourceName="Phone" startDate="2026-11-17 22:30:00 +0100" endDate="2026-11-18 06:00:00 +0100" value="HKCategoryValueSleepAnalysisInBed"/>
 <Record type="HKQuantityTypeIdentifierDietaryWater" sourceName="Phone" unit="mL" startDate="2026-11-18 07:00:00 +0100" endDate="2026-11-18 07:00:00 +0100" value="250"/>
</HealthData>
"#;

    fn value(summary: &Summary, metric: &str) -> f64 {
        summary
            .values
            .iter()
            .find(|v| v.metric == metric)
            .unwrap_or_else(|| panic!("{metric}"))
            .value
    }

    #[test]
    fn folds_each_metric_by_its_rule() {
        let summary = fold(SAMPLE.as_bytes()).unwrap();
        assert_eq!(summary.records, 14);
        assert_eq!(
            value(&summary, "steps"),
            2000.0,
            "the larger source wins, sources are not added"
        );
        assert!((value(&summary, "distance_walking_km") - 1.609_344).abs() < 1e-9);
        assert_eq!(value(&summary, "heart_rate_avg"), 80.0);
        assert_eq!(value(&summary, "heart_rate_min"), 60.0);
        assert_eq!(value(&summary, "heart_rate_max"), 100.0);
        assert_eq!(value(&summary, "body_mass_kg"), 81.0);
        assert!((value(&summary, "body_fat_pct") - 18.0).abs() < 1e-9);
        assert!(summary.values.iter().all(|v| v.day == "2026-11-18"));
        assert!(!summary.values.iter().any(|v| v.metric.contains("water")));
    }

    #[test]
    fn sleep_lands_on_the_wake_day_from_the_best_source() {
        let summary = fold(SAMPLE.as_bytes()).unwrap();
        assert_eq!(value(&summary, "sleep_asleep_h"), 5.5);
        assert_eq!(value(&summary, "sleep_deep_h"), 1.5);
        assert_eq!(value(&summary, "sleep_awake_h"), 0.5);
        assert_eq!(value(&summary, "sleep_onset_h"), -1.0);
        assert_eq!(value(&summary, "sleep_wake_h"), 4.5);
        let source = &summary
            .values
            .iter()
            .find(|v| v.metric == "sleep_asleep_h")
            .unwrap()
            .source;
        assert_eq!(
            source, "Watch",
            "the in-bed-only phone night loses to the night with sleep"
        );
    }

    #[test]
    fn a_nap_does_not_move_the_night_and_midnight_does_not_split_it() {
        let rec = |start: &str, end: &str, stage: &str| {
            format!(
                " <Record type=\"HKCategoryTypeIdentifierSleepAnalysis\" sourceName=\"Watch\" startDate=\"{start} +0100\" endDate=\"{end} +0100\" value=\"HKCategoryValueSleepAnalysis{stage}\"/>\n"
            )
        };
        let xml = [
            rec("2026-11-17 22:45:00", "2026-11-17 23:50:00", "AsleepCore"),
            rec("2026-11-17 23:50:00", "2026-11-18 06:15:00", "AsleepCore"),
            rec("2026-11-18 14:00:00", "2026-11-18 14:40:00", "AsleepCore"),
        ]
        .concat();
        let summary = fold(xml.as_bytes()).unwrap();
        let days: Vec<_> = summary.values.iter().map(|v| v.day.as_str()).collect();
        assert!(days.iter().all(|d| *d == "2026-11-18"), "{days:?}");
        assert!((value(&summary, "sleep_asleep_h") - 7.5).abs() < 1e-9);
        assert!((value(&summary, "sleep_onset_h") + 1.25).abs() < 1e-9);
        assert!((value(&summary, "sleep_wake_h") - 6.25).abs() < 1e-9);
    }

    #[test]
    fn uploaded_samples_fold_like_the_export() {
        let from_export = fold(SAMPLE.as_bytes()).unwrap();
        let mut expected: Vec<_> = from_export
            .values
            .into_iter()
            .filter(|v| v.metric.starts_with("sleep_"))
            .collect();
        let samples = [
            (
                "Watch",
                "2026-11-17 23:00:00 +0100",
                "2026-11-18 03:00:00 +0100",
                "AsleepCore",
            ),
            (
                "Watch",
                "2026-11-18 03:00:00 +0100",
                "2026-11-18 04:30:00 +0100",
                "AsleepDeep",
            ),
            (
                "Watch",
                "2026-11-18 04:30:00 +0100",
                "2026-11-18 05:00:00 +0100",
                "Awake",
            ),
            (
                "Phone",
                "2026-11-17 22:30:00 +0100",
                "2026-11-18 06:00:00 +0100",
                "InBed",
            ),
        ];
        let mut uploaded = fold_sleep(samples);
        let key = |v: &DayValue| (v.metric, v.day.clone());
        expected.sort_by_key(key);
        uploaded.sort_by_key(key);
        assert_eq!(uploaded, expected);
        assert!(fold_sleep([("W", "x", "y", "AsleepCore")]).is_empty());
    }

    #[test]
    fn a_day_with_only_a_nap_has_no_night() {
        let xml = " <Record type=\"HKCategoryTypeIdentifierSleepAnalysis\" sourceName=\"Watch\" startDate=\"2026-11-01 14:30:00 +0100\" endDate=\"2026-11-01 15:53:00 +0100\" value=\"HKCategoryValueSleepAnalysisAsleepUnspecified\"/>\n";
        assert!(fold(xml.as_bytes()).unwrap().values.is_empty());
    }

    #[test]
    fn attributes_are_unescaped_and_unknown_units_are_dropped() {
        let summary = fold(SAMPLE.as_bytes()).unwrap();
        let mass = summary
            .values
            .iter()
            .find(|v| v.metric == "body_mass_kg")
            .unwrap();
        assert_eq!(mass.source, "Scale & Co");
        assert_eq!(convert(1.0, "furlong", "km"), None);
        assert!(fold("<Record type=\"x\"".as_bytes())
            .unwrap()
            .values
            .is_empty());
    }
}
