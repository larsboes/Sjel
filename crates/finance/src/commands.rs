//! Local Finance commands shared by the CLI and the owner process.

use std::fmt::Write;

use crate::{config::Config, store::FinanceStore};

macro_rules! line {
    ($out:expr, $($arg:tt)*) => {
        writeln!($out, $($arg)*).map_err(|error| error.to_string())?
    };
}

pub const USAGE: &str = "\
Usage:
  finance-cli prices fetch                     fetch from every registered provider
  finance-cli prices fetch --provider broker   fetch from one provider
  finance-cli prices fetch --dry-run           print what would be written, touch nothing
  finance-cli prices status                    per-instrument freshness and the last fetch per provider
  finance-cli decisions run                    recompute proposals and reconcile the ledger
  finance-cli decisions run --dry-run          print the proposals, write nothing
  finance-cli decisions export                 re-render every month file from the ledger
  finance-cli decisions export --month 2026-09 re-render one month
  finance-cli subscriptions audit              read-only Q103 currency audit; exit 1 on a finding

subscriptions audit opens no file for writing. It reports three things a wrong
figure hides behind: a subscription note whose price key cannot be read, a note whose
declared currency differs from the currency of its price in force, and a billing
subscription whose price cannot be stated in EUR because no rate covers the pair.
Exit status is 1 when it finds any of the three, so a scheduled run notices.

prices fetch writes one finance_prices row per new observation and one
finance_price_fetches row per attempt, successful or not. A per-instrument
refusal is recorded and never fatal, and a re-fetch that finds nothing new is
the normal outcome rather than a failure: the exit status is non-zero only when
every attempt refused or errored.

decisions export re-renders <overlay>/data/finance/decisions/YYYY-MM.md, which
the server already writes on every verdict. This is the copy you can take by
hand when the server is down.

Environment:
  SJEL_FINANCE_DB_PATH           the owner database
  SJEL_PERSONAL_ROOT             the overlay, where config/finance.json lives
  SJEL_FINANCE_DECISIONS_ROOT    where the month files and the subscription series copy are
                                 written (default: the overlay)
  SJEL_PROJECTS_BASE_URL         where subscription notes are read (default: projects on 8103)

A verification run should set SJEL_FINANCE_DECISIONS_ROOT to a scratch directory. Neither is
isolated by SJEL_FINANCE_DB_PATH: the exports and the vault projection are files, and
they land wherever the configuration points.";

/// Run a local Finance operation and collect its console output.
///
/// # Errors
/// The command, its configuration, or a Finance operation failed.
pub fn run(args: &[String], out: &mut String) -> Result<bool, String> {
    let rest: Vec<_> = args.iter().skip(2).map(String::as_str).collect();
    match (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
    ) {
        (Some("prices"), Some("fetch")) => fetch_prices(&rest, out),
        (Some("prices"), Some("status")) => print_status(&rest, out),
        (Some("decisions"), Some("run")) => run_decisions(&rest, out),
        (Some("decisions"), Some("export")) => export_decisions(&rest, out),
        (Some("subscriptions"), Some("audit")) => audit_subscriptions(&rest, out),
        _ => Err(USAGE.into()),
    }
}

fn flag(rest: &[&str], name: &str) -> bool {
    rest.contains(&name)
}

fn option(rest: &[&str], name: &str) -> Option<String> {
    rest.iter()
        .position(|argument| *argument == name)
        .and_then(|index| rest.get(index + 1))
        .map(|value| (*value).to_string())
}

fn store_and_config() -> Result<(FinanceStore, Config), String> {
    let config = Config::load();
    #[cfg(feature = "turso-wal")]
    if config.database_path == sjel_config::database_path() {
        return Err("Turso requires a separate Finance owner file".into());
    }
    let store = FinanceStore::open(&config.database_path).map_err(|error| error.to_string())?;
    Ok((store, config))
}

/// Where the month files go. `SJEL_FINANCE_DECISIONS_ROOT` if set, else the
/// overlay. The override is what lets a verification run write its exports to a
/// scratch directory while still reading the real configuration.
fn overlay_root() -> Option<std::path::PathBuf> {
    Config::load().decisions_root
}

/// Exit 0 when at least one target produced a row; non-zero only when every
/// target refused. One refusal among twelve successes is a recorded fact, not a
/// failed run.
fn fetch_prices(rest: &[&str], out: &mut String) -> Result<bool, String> {
    let dry_run = flag(rest, "--dry-run");
    let providers: Vec<String> = match option(rest, "--provider") {
        Some(name) => vec![name],
        None => crate::price::PROVIDERS
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    };
    let (store, config) = store_and_config()?;
    let as_of = crate::clock::today();
    let fetched_at = crate::clock::now_timestamp();
    let runs = crate::price::run_named(&store, &config, &providers, &as_of, &fetched_at, dry_run)?;
    let mut any = false;
    for run in &runs {
        for attempt in &run.attempts {
            line!(
                out,
                "{:<8} {:<24} {:<8} {:>4} row(s){}",
                run.provider,
                attempt.target,
                attempt.status.as_str(),
                attempt.rows_written,
                if attempt.detail.is_empty() {
                    String::new()
                } else {
                    format!("  {}", attempt.detail)
                }
            );
        }
        line!(
            out,
            "{}: {} target(s), {} row(s) written, {} refused, {} errored{}",
            run.provider,
            run.attempted,
            run.written,
            run.refused,
            run.errored,
            if dry_run {
                "  (dry run, nothing written)"
            } else {
                ""
            }
        );
        any = any || run.succeeded();
    }
    if !dry_run {
        let outcomes = crate::ideas::check_triggers(
            &store,
            &config.projects_base_url,
            &crate::config::ideas_project(),
            &as_of,
            &fetched_at,
        )?;
        for outcome in &outcomes {
            line!(
                out,
                "trigger  {:<24} {:<8} {}{}",
                outcome.label,
                outcome.action,
                outcome.trigger_key,
                if outcome.detail.is_empty() {
                    String::new()
                } else {
                    format!("  {}", outcome.detail)
                }
            );
        }
        line!(out, "triggers: {} changed or skipped", outcomes.len());
        let scored = crate::review::score(&store, &as_of)?;
        line!(out, "reviews: {scored} move(s) scored");
        let reviewed = crate::review::run(
            &store,
            &crate::review::RunContext {
                comms_base_url: &config.comms_base_url,
                assistant_base_url: &crate::config::assistant_base_url(),
                model_role: &crate::config::ideas_model_role(),
                today: &as_of,
                now: &fetched_at,
            },
            None,
        )?;
        for outcome in &reviewed {
            line!(
                out,
                "review   {:<24} {:<8} {}",
                outcome.label,
                outcome.result,
                outcome.detail
            );
        }
    }
    Ok(any)
}

fn print_status(_rest: &[&str], out: &mut String) -> Result<bool, String> {
    let (store, config) = store_and_config()?;
    let as_of = crate::clock::today();
    let threshold = config
        .targets
        .as_ref()
        .map(|targets| targets.price_freshness_days)
        .unwrap_or(4);
    let counts: std::collections::BTreeMap<String, i64> = store
        .price_counts()
        .map_err(|error| error.to_string())?
        .into_iter()
        .collect();
    let latest = store.latest_prices().map_err(|error| error.to_string())?;
    if latest.is_empty() {
        line!(
            out,
            "no price has ever been observed; run: finance prices fetch"
        );
    }
    for observation in &latest {
        let age = crate::clock::days_between(&observation.observed_on, &as_of);
        line!(
            out,
            "{:<24} {} via {:<8} {:>4} day(s) old  {:>5} observation(s)  {}",
            observation.instrument,
            observation.observed_on,
            observation.source,
            age.unwrap_or(-1),
            counts.get(&observation.instrument).copied().unwrap_or(0),
            match age {
                Some(age) if age <= threshold => "fresh",
                Some(_) => "stale",
                None => "unknown",
            }
        );
    }
    for attempt in store
        .recent_fetches(10)
        .map_err(|error| error.to_string())?
    {
        line!(
            out,
            "  last: {:<8} {:<24} {:<8} at {}  {}",
            attempt.provider,
            attempt.target,
            attempt.status.as_str(),
            attempt.fetched_at,
            attempt.detail
        );
    }
    Ok(true)
}

fn run_decisions(rest: &[&str], out: &mut String) -> Result<bool, String> {
    let dry_run = flag(rest, "--dry-run");
    let (store, config) = store_and_config()?;
    let currency = config
        .targets
        .as_ref()
        .map(|targets| targets.currency.clone())
        .unwrap_or_else(|| "EUR".into());
    let as_of = crate::clock::today();
    let proposed_at = crate::clock::now_timestamp();
    // No feed evidence from the CLI: the loopback call needs a running comms, and
    // a scheduled job that fails because a different capability is down is a job
    // that reports the wrong fault. Evidence is optional; the proposal is not.
    let (minted, caveats) =
        crate::decision::recompute(&store, &config, &[], &as_of, &proposed_at, &currency)?;
    for caveat in &caveats {
        line!(out, "caveat: {caveat}");
    }
    if dry_run {
        for proposal in &minted {
            line!(
                out,
                "{:<12} {:<28} {}",
                proposal.proposal.kind,
                proposal.proposal.subject,
                proposal.proposal.title
            );
        }
        line!(out, "{} proposal(s) — nothing written", minted.len());
        return Ok(true);
    }
    let outcome = crate::decision::run(&store, &minted, &proposed_at, overlay_root().as_deref())?;
    line!(
        out,
        "{} proposed, {} unchanged, {} reopened, {} superseded",
        outcome.proposed,
        outcome.unchanged,
        outcome.reopened,
        outcome.superseded
    );
    Ok(true)
}

/// An unconfigured overlay is an error rather than a quiet zero: the whole point
/// of the command is that a copy exists, and "nothing to do" is
/// indistinguishable from "nothing was saved" once the terminal scrolls.
fn export_decisions(rest: &[&str], out: &mut String) -> Result<bool, String> {
    let overlay = overlay_root()
        .ok_or("no overlay configured: set SJEL_PERSONAL_ROOT so the copy has somewhere to live")?;
    let (store, _config) = store_and_config()?;
    let written = match option(rest, "--month") {
        Some(month) => vec![crate::decision::export_month(&store, &overlay, &month)?],
        None => crate::decision::export_all(&store, &overlay)?,
    };
    if written.is_empty() {
        line!(
            out,
            "no decision has been proposed yet, so no month file was written"
        );
        return Ok(true);
    }
    for path in &written {
        line!(out, "{path}");
    }
    line!(out, "{} month file(s) written", written.len());
    Ok(true)
}

/// PRD Q103's audit: every place a subscription figure can carry the wrong currency.
///
/// Read-only by construction — it opens the store, reads the subscription notes, and prints. It does
/// not import, does not write a region, and does not fetch. That matters because the
/// answer it gives is most wanted while a service is running, and a reporting command
/// that mutates is a command nobody runs at that moment.
///
/// Exit 1 on any finding. A silent pass and a pass with a wrong figure in it have to
/// look different from a scheduler, or the audit is decoration.
fn audit_subscriptions(_rest: &[&str], out: &mut String) -> Result<bool, String> {
    let (store, config) = store_and_config()?;
    let as_of = crate::clock::today();
    let subs = store.list().map_err(|error| error.to_string())?;
    let rates = store.latest_fx_rates().map_err(|error| error.to_string())?;
    line!(
        out,
        "as of {as_of}: {} subscription(s), {} FX rate(s) recorded",
        subs.len(),
        rates.len()
    );

    // An unreachable projects capability is a fact worth printing, not a crash: the
    // store half of this audit is still worth running without the notes.
    let notes = match crate::subscription_notes::fetch(&config.projects_base_url) {
        Ok(notes) => notes,
        Err(error) => {
            line!(out, "notes not read: {error}");
            Vec::new()
        }
    };

    let mut findings = 0usize;

    let mut deprecated: Vec<String> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    for note in &notes {
        match crate::subscription_notes::read_price(&note.fields) {
            Ok(reading) => {
                if reading.key.is_deprecated() || !reading.shadowed.is_empty() {
                    let shadowed: Vec<&str> =
                        reading.shadowed.iter().map(|key| key.as_str()).collect();
                    deprecated.push(format!(
                        "  {:<34} reads `{}` ({} {}), shadowing [{}]",
                        note.name,
                        reading.key.as_str(),
                        crate::subscription::cents_to_decimal(reading.amount_cents),
                        reading.currency,
                        shadowed.join(", ")
                    ));
                }
            }
            Err(refusal) => unreadable.push(format!("  {:<34} {refusal}", note.name)),
        }
    }

    // A deprecated spelling is read, so it is not a finding. It is printed because the
    // set only ever shrinks by editing a note, and an unlisted alias is one nobody
    // remembers to retire.
    line!(out, "\n-- notes read through a deprecated key --");
    print_or_none(&deprecated, out)?;

    line!(out, "\n-- notes whose price key cannot be read --");
    print_or_none(&unreadable, out)?;
    findings += unreadable.len();

    line!(
        out,
        "\n-- notes whose currency differs from the price in force --"
    );
    let mismatches = crate::subscription_notes::currency_mismatches(&notes, &subs, &as_of);
    print_or_none(
        &mismatches
            .iter()
            .map(|mismatch| format!("  {} — {}", mismatch.source_path, mismatch.detail))
            .collect::<Vec<_>>(),
        out,
    )?;
    findings += mismatches.len();

    line!(out, "\n-- burn in {} --", crate::money::DECLARED_CURRENCY);
    let burn = crate::money::burn_in_eur(&subs, &as_of, &rates);
    line!(out,
        "  {} {} / month ({} {} / year) from {} billing, {} converted, {} covered, {} without a price",
        crate::subscription::cents_to_decimal(burn.monthly_cents),
        burn.currency,
        crate::subscription::cents_to_decimal(burn.annual_cents),
        burn.currency,
        burn.billing_count,
        burn.converted_count,
        burn.covered_count,
        burn.unknown_price_count,
    );
    for left_out in &burn.not_convertible {
        line!(
            out,
            "  NOT IN THE TOTAL: {:<24} {} {} / month — {}",
            left_out.subscription_name,
            crate::subscription::cents_to_decimal(left_out.monthly_cents),
            left_out.currency,
            left_out.detail
        );
    }
    findings += burn.not_convertible.len();
    // A billing subscription with no price point shortens the total by an unknown
    // amount, which is the failure Q103 names, and `EurBurn::is_complete` already
    // calls it incomplete. Counting only `not_convertible` let that case print
    // "0 finding(s)" and exit 0 — an audit that answers the same for a good store and
    // a short one. Watched: planting an `active` state on a subscription with no price
    // moved a fully-corrected copy from `0 finding(s)` exit 0 to `1 finding(s)` exit 1.
    findings += burn.unknown_price_count;
    if burn.unknown_price_count > 0 {
        line!(
            out,
            "  NOT IN THE TOTAL: {} billing subscription(s) have no price point at all",
            burn.unknown_price_count
        );
    }

    line!(out, "\n{findings} finding(s)");
    Ok(findings == 0)
}

/// An empty section says so. A heading with nothing under it reads as output that was
/// cut off, which is the one thing an audit must never look like.
fn print_or_none(lines: &[String], out: &mut String) -> Result<(), String> {
    if lines.is_empty() {
        line!(out, "  none");
        return Ok(());
    }
    for line in lines {
        line!(out, "{line}");
    }
    Ok(())
}
