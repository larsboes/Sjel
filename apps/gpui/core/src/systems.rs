//! The Systems page's numbers, and the rules that decide how they read.
//!
//! Every payload here is another owner's JSON, passed through unchanged: macmon's sample,
//! `tools/storage report --json`, `tools/updates --json`, `tools/host-watch`'s findings, and
//! `sjel_runtime::Status`. The reductions below are the native half of
//! `dashboard/src/lib/systems/{storage,updates}.ts` and `dashboard/src/lib/runtime.ts` —
//! same rules, same wording, so a reader comparing the two surfaces sees one answer.
//!
//! Nothing here draws. The page lives in `apps/gpui/src/systems.rs`; this file is pure so every
//! ordering, unit switch and "not checked is not current" rule is testable without a window.

use chrono::{DateTime, Local};

pub use status_api::{
    audit_label, audit_needs_attention, category_label, dated_absences, day_label, gigabytes,
    percent, power_label, selection_label, severity, signed_storage_bytes, storage_bytes,
    temperature, version_label, watts, ApplyReceipt, Disk, ExpectedService, Finding, Findings,
    MacmonMemory, MacmonSample, MacmonTemperature, Owner, RuntimeStatus, RuntimeView, Severity,
    StorageAbsence, StorageClass, StorageClassSample, StorageHistory, StorageProtected,
    StorageReport, StorageSample, StorageTrend, StorageView, Temperature, UpdateGroup, UpdateRow,
    UpdateStatus, UpdateSurface, UpdatesReport, UpdatesView, RUNTIME_CATEGORIES,
    RUNTIME_SELECTIONS, TREND_WINDOW_HOURS,
};

/// The sample's clock time in the reader's own zone. An unparseable stamp prints as itself:
/// the stamp is the evidence, and `Invalid Date` would hide it.
pub fn clock_label(stamp: &str) -> String {
    DateTime::parse_from_rfc3339(stamp).map_or_else(
        |_| stamp.to_owned(),
        |at| at.with_timezone(&Local).format("%H:%M:%S").to_string(),
    )
}

/// The line under the buttons: what the last run did, or what it is doing.
pub fn apply_summary(view: &UpdatesView, now: DateTime<Local>) -> String {
    let Some(last) = &view.last_apply else {
        return String::new();
    };
    let class = last.class.clone().unwrap_or_default();
    let when = last
        .at
        .as_deref()
        .map(|at| since_label(at, now))
        .unwrap_or_default();
    if last.state.as_deref() == Some("running") {
        return format!("{class} — applying, started {when}");
    }
    let failed = last.failed.unwrap_or(0);
    let steps = last.steps.unwrap_or(0);
    let still = last.still_stale.unwrap_or(0);
    let mut line = if failed > 0 {
        format!("{class} — {failed} of {steps} failed")
    } else {
        format!(
            "{class} — {steps} step{} applied",
            if steps == 1 { "" } else { "s" }
        )
    };
    if still > 0 {
        line.push_str(&format!(", {still} still stale"));
    }
    line.push_str(&audit_label(last.audit.as_deref()));
    if !when.is_empty() {
        line.push_str(&format!(" · {when}"));
    }
    line
}

fn since_label(stamp: &str, now: DateTime<Local>) -> String {
    let Ok(at) = DateTime::parse_from_rfc3339(stamp) else {
        return String::new();
    };
    let mins = (now - at.with_timezone(&Local)).num_minutes().max(0);
    match mins {
        0 => "just now".to_owned(),
        m if m < 60 => format!("{m}m ago"),
        m if m < 60 * 24 => format!("{}h ago", m / 60),
        m => format!("{}d ago", m / (60 * 24)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(stamp: &str) -> DateTime<Local> {
        DateTime::parse_from_rfc3339(stamp)
            .expect("a fixed stamp")
            .with_timezone(&Local)
    }

    /// A real `macmon serve` sample, trimmed to the fields this page reads. The field names are
    /// the machine's own, not a shape invented here: a rename upstream has to fail this test
    /// rather than quietly empty the panel.
    const MACMON_SAMPLE: &str = r#"{
        "all_power": 1.0040725469589233,
        "ane_power": 0.0,
        "cpu_power": 0.9918359518051147,
        "cpu_usage_pct": 0.06693775951862335,
        "ecpu_usage": [1898, 0.16085276007652283],
        "gpu_power": 0.012236538343131542,
        "gpu_ram_power": 0.0,
        "gpu_usage": [338, 0.0],
        "memory": {"ram_total": 25769803776, "ram_usage": 18336169984,
                   "swap_total": 4294967296, "swap_usage": 3279814656},
        "pcpu_usage": [2575, 0.019980259239673615],
        "ram_power": 0.0,
        "sys_power": 9.936366081237793,
        "temp": {"cpu_temp_avg": 46.08387756347656, "gpu_temp_avg": 44.22336959838867},
        "timestamp": "2026-10-23T07:54:05.745853+00:00"
    }"#;

    #[test]
    fn a_live_macmon_sample_reads_as_the_panel_draws_it() {
        let sample: MacmonSample =
            serde_json::from_str(MACMON_SAMPLE).expect("the machine's own sample parses");
        assert_eq!(watts(sample.all_power), "1.00 W");
        assert_eq!(percent(sample.cpu_usage_pct), "6.7%");
        assert_eq!(gigabytes(sample.memory.ram_usage), "17.1 GB");
        assert_eq!(temperature(sample.temp.cpu_temp_avg), Temperature::Cool);
        assert!(sample.under_pressure(), "3.1 of 4.0 GB end in swap");
        assert!(
            (sample.ram_ratio() - 0.7115).abs() < 0.001,
            "a ratio, not a percentage"
        );
        assert_eq!(
            clock_label(&sample.timestamp).matches(':').count(),
            2,
            "a clock time, not the raw stamp: {}",
            clock_label(&sample.timestamp)
        );
    }

    #[test]
    fn a_temperature_reads_by_its_own_thresholds() {
        assert_eq!(temperature(41.5), Temperature::Cool);
        assert_eq!(temperature(60.0), Temperature::Warm);
        assert_eq!(temperature(79.9), Temperature::Warm);
        assert_eq!(temperature(80.0), Temperature::Hot);
    }

    #[test]
    fn macmon_bytes_print_as_gigabytes_and_watts_carry_two_decimals() {
        assert_eq!(gigabytes(34_359_738_368.0), "32.0 GB");
        assert_eq!(watts(4.5), "4.50 W");
        assert_eq!(percent(0.1234), "12.3%");
    }

    #[test]
    fn a_zero_total_never_divides_by_it() {
        let sample = MacmonSample::default();
        assert_eq!(sample.ram_ratio(), 0.0);
        assert_eq!(sample.swap_ratio(), 0.0);
        assert!(!sample.under_pressure(), "no swap is not pressure");
    }

    #[test]
    fn storage_bytes_switches_unit_exactly_where_the_tool_does() {
        assert_eq!(storage_bytes(0), "0 MB");
        assert_eq!(storage_bytes(1024 * 1024 * 1024 - 1), "1023 MB");
        assert_eq!(storage_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(signed_storage_bytes(0), "±0 MB");
        assert_eq!(signed_storage_bytes(3_221_225_472), "+3.0 GB");
        assert_eq!(signed_storage_bytes(-1_073_741_824), "-1.0 GB");
    }

    #[test]
    fn an_unknown_volume_state_reads_as_ok_rather_than_red() {
        assert_eq!(severity("critical"), Severity::Critical);
        assert_eq!(severity("warn"), Severity::Warn);
        assert_eq!(severity("ok"), Severity::Ok);
        assert_eq!(severity("something-new"), Severity::Ok);
    }

    fn class(name: &str, bytes: u64) -> StorageClass {
        StorageClass {
            name: name.to_owned(),
            bytes,
            applicable: true,
            ..StorageClass::default()
        }
    }

    #[test]
    fn the_view_sorts_classes_by_size_and_keeps_absent_ones_out_of_the_sized_list() {
        const MB: u64 = 1024 * 1024;
        let report = StorageReport {
            disk: Disk {
                total: 1000 * MB,
                used: 250 * MB,
                free: 750 * MB,
            },
            state: "warn".to_owned(),
            classes: vec![
                class("small", 10 * MB),
                StorageClass {
                    present: Some(false),
                    ..class("gone", 0)
                },
                class("big", 900 * MB),
                StorageClass {
                    measured: Some(false),
                    ..class("unreadable", 0)
                },
            ],
            ..StorageReport::default()
        };
        let view = report.view();
        let names: Vec<&str> = view.classes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["big", "small"], "largest first, sized rows only");
        assert_eq!(view.classes_absent.len(), 1);
        assert_eq!(view.classes_absent[0].name, "gone");
        assert_eq!(view.unmeasured.len(), 1);
        assert_eq!(view.unmeasured[0].name, "unreadable");
        assert_eq!(view.used_pct, 0.25);
        assert_eq!(view.reclaimable_label, "910 MB");
        assert_eq!(
            view.scale_bytes,
            900 * MB,
            "the partial 0-byte class does not set the scale"
        );
        assert_eq!(view.severity, Severity::Warn);
        assert_eq!(view.state_label, "warn", "the tool's own word, verbatim");
        assert!(
            view.reconciliation.is_none(),
            "an older report omits the fields, so the line is omitted"
        );
    }

    #[test]
    fn a_protected_path_is_the_point_of_its_row_even_at_zero_bytes() {
        let report = StorageReport {
            protected: vec![
                StorageProtected {
                    path: "/gone".to_owned(),
                    bytes: 0,
                    reason: "kept".to_owned(),
                    present: Some(false),
                },
                StorageProtected {
                    path: "/empty".to_owned(),
                    bytes: 0,
                    reason: "kept".to_owned(),
                    present: Some(true),
                },
            ],
            ..StorageReport::default()
        };
        let view = report.view();
        assert_eq!(view.protected.len(), 2);
        assert_eq!(view.protected_absent, 1);
        assert_eq!(view.protected_absent_paths, ["/gone"]);
        assert_eq!(view.protected_unreadable, 1);
    }

    #[test]
    fn only_an_absent_service_is_named_and_unverifiable_is_not() {
        let report = StorageReport {
            expected_service: vec![
                ExpectedService {
                    name: "comms".to_owned(),
                    presence: Some("running".to_owned()),
                    ..ExpectedService::default()
                },
                ExpectedService {
                    name: "soundscape".to_owned(),
                    presence: Some("absent".to_owned()),
                    ..ExpectedService::default()
                },
                ExpectedService {
                    name: "probe".to_owned(),
                    presence: Some("unverifiable".to_owned()),
                    ..ExpectedService::default()
                },
            ],
            ..StorageReport::default()
        };
        assert_eq!(report.view().services_absent, ["soundscape"]);
    }

    fn history(samples: &[(&str, i64)], absent: Vec<StorageAbsence>) -> StorageHistory {
        StorageHistory {
            retention_days: 30,
            samples: samples
                .iter()
                .map(|(hour, used)| StorageSample {
                    hour: (*hour).to_owned(),
                    used: *used,
                    unattributed: 0,
                })
                .collect(),
            absent,
            classes: Vec::new(),
        }
    }

    #[test]
    fn one_sample_is_a_point_and_not_a_trend() {
        let one = history(&[("2026-10-12 21:00:00.000+00:00", 10)], Vec::new());
        let trend = one.trend().expect("a sample exists");
        assert!(!trend.enough);
        assert_eq!(trend.span_label, "0h");
        assert!(history(&[], Vec::new()).trend().is_none());
    }

    #[test]
    fn a_trend_reads_its_span_from_the_stamps_not_the_sample_count() {
        let gb: i64 = 1024 * 1024 * 1024;
        let rows = [
            ("2026-10-12 21:00:00.000+00:00", gb),
            ("2026-10-12 23:00:00.000+00:00", 2 * gb),
            ("2026-10-19 21:00:00.000+00:00", 4 * gb),
        ];
        let trend = history(&rows, Vec::new()).trend().expect("three samples");
        assert!(trend.enough);
        assert_eq!(trend.delta_label, "+3.0 GB");
        assert_eq!(trend.span_label, "7d");
        assert_eq!(trend.bars.len(), 3);
        assert_eq!(trend.bars[0], 0.25, "the window's smallest bar still shows");
        assert_eq!(trend.bars[2], 1.0);
    }

    #[test]
    fn a_flat_series_draws_half_bars_rather_than_nothing() {
        let rows = [
            ("2026-10-12 21:00:00.000+00:00", 500),
            ("2026-10-12 22:00:00.000+00:00", 500),
        ];
        let trend = history(&rows, Vec::new()).trend().expect("two samples");
        assert_eq!(trend.bars, [0.5, 0.5]);
        assert_eq!(trend.delta_label, "±0 MB");
    }

    #[test]
    fn an_absence_is_dated_only_by_the_history_that_knows_it() {
        let history = history(
            &[
                ("2026-10-12 21:00:00.000+00:00", 1),
                ("2026-10-12 22:00:00.000+00:00", 1),
            ],
            vec![
                StorageAbsence {
                    kind: "class".to_owned(),
                    name: "chrome-models".to_owned(),
                    since: "2026-10-12 21:00:00.000+00:00".to_owned(),
                    capped: false,
                },
                StorageAbsence {
                    kind: "protected".to_owned(),
                    name: "/old".to_owned(),
                    since: "2026-10-01 21:00:00.000+00:00".to_owned(),
                    capped: true,
                },
            ],
        );
        let names = vec!["chrome-models".to_owned(), "unknown".to_owned()];
        assert_eq!(
            dated_absences(&names, "class", Some(&history)),
            ["chrome-models (absent since 12 Oct)", "unknown"]
        );
        let paths = vec!["/old".to_owned()];
        assert_eq!(
            dated_absences(&paths, "protected", Some(&history)),
            ["/old (absent since 1 Oct or earlier)"]
        );
        assert_eq!(
            dated_absences(&names, "class", None),
            ["chrome-models", "unknown"],
            "no history is no date, never a guess"
        );
    }

    #[test]
    fn a_stamp_outside_the_canonical_shape_prints_as_itself() {
        assert_eq!(day_label("2026-10-12 21:00:00.000+00:00"), "12 Oct");
        assert_eq!(day_label("2026-13-01"), "2026-13-01");
        assert_eq!(day_label("yesterday"), "yesterday");
    }

    #[test]
    fn class_growth_needs_two_readings() {
        let mut history = history(
            &[
                ("2026-10-12 21:00:00.000+00:00", 1),
                ("2026-10-12 22:00:00.000+00:00", 1),
            ],
            Vec::new(),
        );
        history.classes = vec![
            StorageClassSample {
                name: "cargo-target".to_owned(),
                bytes: 1_073_741_824,
            },
            StorageClassSample {
                name: "cargo-target".to_owned(),
                bytes: 4_294_967_296,
            },
            StorageClassSample {
                name: "once".to_owned(),
                bytes: 5,
            },
        ];
        let deltas = history.class_deltas();
        assert_eq!(
            deltas.get("cargo-target").map(String::as_str),
            Some("+3.0 GB")
        );
        assert_eq!(deltas.get("once"), None);
    }

    fn surface(id: &str, owner: &str, actionable: bool) -> UpdateSurface {
        UpdateSurface {
            id: id.to_owned(),
            title: id.to_owned(),
            owner: owner.to_owned(),
            owner_detail: String::new(),
            actionable,
            why: String::new(),
        }
    }

    fn update_row(surface: &str, name: &str, status: &str, action: Option<&str>) -> UpdateRow {
        UpdateRow {
            surface: surface.to_owned(),
            name: name.to_owned(),
            status: status.to_owned(),
            action: action.map(str::to_owned),
            ..UpdateRow::default()
        }
    }

    #[test]
    fn groups_follow_the_owner_order_and_count_only_rows_a_verb_can_move() {
        let report = UpdatesReport {
            surfaces: vec![
                surface("self", "self", false),
                surface("brew", "scheduled", true),
            ],
            rows: vec![
                update_row("brew", "jq", "stale", Some("brew upgrade jq")),
                update_row("brew", "pinned", "stale", None),
                update_row("brew", "fresh", "current", None),
                update_row("self", "chrome", "current", None),
            ],
            ..UpdatesReport::default()
        };
        let view = report.view();
        let ids: Vec<&str> = view.groups.iter().map(|g| g.surface.id.as_str()).collect();
        assert_eq!(ids, ["brew", "self"], "scheduled before self-managed");
        assert_eq!(
            view.groups[0].actionable, 1,
            "a pinned row is not a promise"
        );
        assert_eq!(view.groups[1].actionable, 0);
        assert_eq!(view.stale, 2);
        assert!(!view.busy);
    }

    #[test]
    fn a_fifth_status_reads_as_unknown_and_is_never_counted_current() {
        let report = UpdatesReport {
            surfaces: vec![surface("cargo", "manual", true)],
            rows: vec![update_row("cargo", "macmon", "brand-new", None)],
            ..UpdatesReport::default()
        };
        let view = report.view();
        assert_eq!(view.stale, 0);
        assert_eq!(view.unknown, 1);
    }

    #[test]
    fn the_version_cell_shows_both_versions_only_when_they_differ() {
        let mut row = update_row("brew", "jq", "stale", None);
        assert_eq!(version_label(&row), "");
        row.installed = Some("0.7.0".to_owned());
        assert_eq!(version_label(&row), "0.7.0");
        row.latest = Some("0.7.0".to_owned());
        assert_eq!(version_label(&row), "0.7.0");
        row.latest = Some("0.8.2".to_owned());
        assert_eq!(version_label(&row), "0.7.0 → 0.8.2");
    }

    fn receipt(state: &str, audit: Option<&str>) -> ApplyReceipt {
        ApplyReceipt {
            at: Some("2026-10-22T09:00:00+00:00".to_owned()),
            class: Some("cargo".to_owned()),
            steps: Some(3),
            failed: Some(0),
            still_stale: Some(1),
            state: Some(state.to_owned()),
            audit: audit.map(str::to_owned),
        }
    }

    #[test]
    fn the_summary_says_what_the_last_run_did_and_never_hides_the_audit() {
        let now = local("2026-10-22T09:42:00+00:00");
        let view = UpdatesView {
            last_apply: Some(receipt("done", Some("clean"))),
            ..UpdatesView::default()
        };
        assert_eq!(
            apply_summary(&view, now),
            "cargo — 3 steps applied, 1 still stale · audit clean · 42m ago"
        );
        let failed = UpdatesView {
            last_apply: Some(ApplyReceipt {
                failed: Some(2),
                ..receipt("done", Some("2 findings"))
            }),
            ..UpdatesView::default()
        };
        assert_eq!(
            apply_summary(&failed, now),
            "cargo — 2 of 3 failed, 1 still stale · audit 2 findings — run tools/audit · 42m ago"
        );
        let running = UpdatesView {
            last_apply: Some(receipt("running", None)),
            ..UpdatesView::default()
        };
        assert_eq!(
            apply_summary(&running, now),
            "cargo — applying, started 42m ago"
        );
        assert_eq!(
            apply_summary(&UpdatesView::default(), now),
            "",
            "no receipt, no line"
        );
    }

    #[test]
    fn only_clean_is_good_news() {
        assert!(!audit_needs_attention(None));
        assert!(!audit_needs_attention(Some("clean")));
        assert!(audit_needs_attention(Some("1 finding")));
        assert!(audit_needs_attention(Some("scanner-missing")));
    }

    #[test]
    fn a_running_apply_disables_the_buttons() {
        let report = UpdatesReport {
            last_apply: Some(receipt("running", None)),
            ..UpdatesReport::default()
        };
        assert!(report.view().busy);
    }

    fn runtime(
        selection: &str,
        effective: &str,
        configured: bool,
        allow: &[&str],
    ) -> RuntimeStatus {
        RuntimeStatus {
            configured,
            selection: selection.to_owned(),
            effective: effective.to_owned(),
            power: "battery".to_owned(),
            power_fresh: true,
            allow: allow.iter().map(|c| (*c).to_owned()).collect(),
            ..RuntimeStatus::default()
        }
    }

    #[test]
    fn a_fresh_install_reads_as_not_configured_running_normal() {
        let view = runtime("auto", "normal", false, &[]).view();
        assert_eq!(view.label, "Normal");
        assert_eq!(view.source, "Not configured");
        assert!(!view.on_the_go);
        assert_eq!(
            view.summary(),
            "Normal · Not configured · No active exceptions"
        );
    }

    #[test]
    fn auto_names_the_power_it_resolved_from_and_flags_a_stale_observation() {
        let mut status = runtime("auto", "normal", true, &[]);
        assert_eq!(status.view().source, "Auto · Battery");
        status.power_fresh = false;
        assert_eq!(status.view().source, "Auto · Battery (stale)");
        status.power = "ac".to_owned();
        assert_eq!(status.view().source, "Auto · AC (stale)");
    }

    #[test]
    fn on_the_go_lists_its_exceptions_and_names_afm_only_when_no_model_is_allowed() {
        let view = runtime("on-the-go", "on-the-go", true, &["transcription"]).view();
        assert!(view.on_the_go);
        assert!(view.afm_only);
        assert_eq!(view.exceptions, ["Transcription"]);
        assert_eq!(
            view.summary(),
            "On the go · Manual · AFM only · Allowed: Transcription"
        );

        let with_models = runtime(
            "on-the-go",
            "on-the-go",
            true,
            &["other-local-models", "bulk-indexing"],
        )
        .view();
        assert!(!with_models.afm_only);
        assert_eq!(
            with_models.exceptions,
            ["Other local models", "Bulk indexing"]
        );
    }

    #[test]
    fn exceptions_only_exist_while_on_the_go() {
        let view = runtime("normal", "normal", true, &["transcription"]).view();
        assert!(view.exceptions.is_empty(), "normal is unrestricted");
    }

    #[test]
    fn a_runtime_status_answers_with_defaults_when_the_body_is_empty() {
        let status: RuntimeStatus = serde_json::from_str("{}").expect("an empty body parses");
        assert_eq!(status.selection, "auto");
        assert_eq!(status.view().label, "Normal");
    }
}
