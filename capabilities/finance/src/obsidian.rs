//! Reading subscription notes out of the vault, and writing derived figures back.
//!
//! The boundary this file implements, and the reason it is worth reading closely:
//! the vault owns why the principal pays for something, whether it is worth it, and
//! what the alternatives are. This capability owns the price series, the state
//! series, and anything computed from them. Neither writes the other's fields.
//!
//! Reading is one direction of that. A note's frontmatter seeds a subscription: its
//! current cost becomes the *first* price point, its status becomes the *first*
//! state change. From then on the series is authoritative and the frontmatter is
//! not re-read for those fields, because a series cannot be reconstructed from the
//! single mutable number it replaced.
//!
//! Writing is the other, and it has two branches, which PRD Q31 (2026-08-23) rules
//! between: write a region into the human's note when one exists, and create a whole
//! generated file only when none does.
//!
//! - **The region.** Derived figures go into a marked region via
//!   `markdown_root::region`, which guarantees bytes outside the markers survive and
//!   refuses to overwrite a region a human edited.
//! - **The projection.** `render_projection`/`export_projections` below, for a
//!   subscription whose note is not in the vault. Measured 2026-08-28: that is all
//!   seven of them, because the 2026-08-23 reorganisation moved every finance note out
//!   under PRD §5.5. The projection is removed the moment a note reappears, so the two
//!   branches never both hold one subscription's figures.
//!
//! This module never opens a file for writing outside those two paths.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use markdown_root::{frontmatter, region, MarkdownRoot, RegionOutcome, RegionSpec};
use serde::Serialize;

use crate::money::{self, EurAmount};
use crate::price::FxObservation;
use crate::subscription::{
    cents_to_decimal, decimal_to_cents, BillingCycle, PricePoint, State, StateChange, Subscription,
};

/// The region marker owner. Stable forever: changing it orphans every region
/// already written into the vault and the next write appends a second one.
pub const REGION_OWNER: &str = "finance";

/// Bumped when the rendered block's shape changes, so a later generator can tell
/// its own old output from a shape it no longer produces.
///
/// 2 (2026-08-28): the price and state series joined the current-state callout, per
/// PRD Q47. The bump does not force a rewrite by itself — the region's hash does that,
/// because every v1 body differs from its v2 replacement.
///
/// 3 (2026-09-09): a non-EUR price states its EUR figure, or states that it has no rate
/// to state one with, per PRD Q103. Every note whose price is not EUR gains a line.
/// Unlike the v2 bump, this one does not reach every note: `region::apply` returns
/// `Unchanged` when the body matches, so only a note whose price is not EUR renders a
/// different body, and a EUR-only note keeps its `v=2` marker until something else
/// changes it. A marker version therefore dates the last *body* change, not the last
/// renderer.
pub const REGION_VERSION: u32 = 3;

/// A note found by the scanner, before anything is persisted.
#[derive(Debug, Clone, PartialEq)]
pub struct ScannedNote {
    /// Vault-relative, slash-separated. The import identity.
    pub source_path: String,
    pub absolute: PathBuf,
    pub name: String,
    pub fields: HashMap<String, String>,
}

#[derive(Debug)]
pub enum ScanError {
    Root(markdown_root::RootError),
    Read { path: PathBuf, detail: String },
    Frontmatter { path: PathBuf, detail: String },
}

impl std::fmt::Display for ScanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanError::Root(e) => write!(f, "{e}"),
            ScanError::Read { path, detail } => {
                write!(f, "cannot read {}: {detail}", path.display())
            }
            ScanError::Frontmatter { path, detail } => {
                write!(f, "frontmatter in {}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for ScanError {}

/// Every markdown note directly inside the configured subscriptions directory.
///
/// Read-only, and bounded by `MarkdownRoot`, so a misconfigured directory cannot
/// walk out of the declared vault. A note that fails to parse is an error naming
/// the file rather than a silently skipped subscription: a subscription that
/// vanishes from a burn total because its frontmatter had a typo is exactly the
/// kind of quietly-wrong figure this whole capability exists to avoid.
pub fn scan(root: &MarkdownRoot, dir: &Path) -> Result<Vec<ScannedNote>, ScanError> {
    let pattern = format!("{}/*.md", dir.to_string_lossy().trim_end_matches('/'));
    let files = root.markdown_files(&pattern).map_err(ScanError::Root)?;

    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let body = std::fs::read_to_string(&file).map_err(|e| ScanError::Read {
            path: file.clone(),
            detail: e.to_string(),
        })?;
        let fields = frontmatter(&body).map_err(|detail| ScanError::Frontmatter {
            path: file.clone(),
            detail,
        })?;
        let source_path = root
            .relative_id(&file)
            .unwrap_or_else(|| file.to_string_lossy().into_owned());
        let name = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| source_path.clone());
        out.push(ScannedNote {
            source_path,
            absolute: file,
            name,
            fields,
        });
    }
    Ok(out)
}

/// A frontmatter key that states what a subscription costs.
///
/// Four spellings of one fact occur in subscription notes. Renaming them in the notes
/// is a vault edit this capability does not own, so the reader learns the vocabulary
/// instead. The
/// declaration order below **is** the precedence: the first key present wins, and the
/// rest are recorded as shadowed rather than merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceKey {
    /// Canonical. The amount, with `currency:` naming the unit and `billing_cycle:`
    /// naming the period. The only spelling a new note should use.
    Cost,
    /// Deprecated. EUR is in the key name, so `currency:` may not contradict it.
    CostEur,
    /// Deprecated. Used on purchase-decision notes, such as a card or a travel pass,
    /// which carry an annual fee and often no `billing_cycle:` at all.
    PriceEur,
    /// Deprecated. EUR and a yearly period are both in the key name.
    YearlyCostEur,
}

/// Precedence order, highest first. The canonical key wins over every alias.
pub const PRICE_KEY_PRECEDENCE: [PriceKey; 4] = [
    PriceKey::Cost,
    PriceKey::CostEur,
    PriceKey::PriceEur,
    PriceKey::YearlyCostEur,
];

impl PriceKey {
    pub fn as_str(self) -> &'static str {
        match self {
            PriceKey::Cost => "cost",
            PriceKey::CostEur => "cost_eur",
            PriceKey::PriceEur => "price_eur",
            PriceKey::YearlyCostEur => "yearly_cost_eur",
        }
    }

    /// Whether the key name itself declares the currency. When it does, a `currency:`
    /// field saying otherwise is a contradiction rather than an override.
    pub fn declares_eur(self) -> bool {
        !matches!(self, PriceKey::Cost)
    }

    pub fn is_deprecated(self) -> bool {
        !matches!(self, PriceKey::Cost)
    }
}

/// What the frontmatter says one subscription costs, once the precedence has settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PriceReading {
    pub key: PriceKey,
    pub amount_cents: i64,
    pub currency: String,
    pub cycle: BillingCycle,
    /// Lower-precedence keys that were also present and were not read. Kept so a note
    /// carrying two disagreeing figures can be reported instead of half-read.
    pub shadowed: Vec<PriceKey>,
}

/// Why a note's price was not read. Every variant leaves the subscription with no
/// price point, which reads downstream as "unknown", never as zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum PriceRefusal {
    /// None of the four keys is present, or the one present is empty.
    NotStated,
    /// A key is present and its value is not a number.
    Unparsable { key: PriceKey, value: String },
    /// The key name says EUR and `currency:` says something else. Guessing which the
    /// human meant is how a wrong figure gets written confidently.
    CurrencyContradiction { key: PriceKey, declared: String },
    /// `price_eur` with no `billing_cycle:`. Defaulting to monthly would turn a 240 EUR
    /// annual fee into 240 EUR a month, a 2,880 EUR a year error that looks like a real
    /// figure.
    CycleNotDeclared { key: PriceKey },
    /// `yearly_cost_eur` beside a `billing_cycle:` that is not yearly. The key name
    /// and the field state different periods for the same money.
    CycleContradiction { key: PriceKey, declared: String },
}

impl std::fmt::Display for PriceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PriceRefusal::NotStated => write!(f, "no price key is present"),
            PriceRefusal::Unparsable { key, value } => {
                write!(f, "`{}: {value}` is not a number", key.as_str())
            }
            PriceRefusal::CurrencyContradiction { key, declared } => write!(
                f,
                "`{}` states EUR and `currency: {declared}` states otherwise",
                key.as_str()
            ),
            PriceRefusal::CycleNotDeclared { key } => write!(
                f,
                "`{}` needs a `billing_cycle:`; the period cannot be guessed",
                key.as_str()
            ),
            PriceRefusal::CycleContradiction { key, declared } => write!(
                f,
                "`{}` states a year and `billing_cycle: {declared}` states otherwise",
                key.as_str()
            ),
        }
    }
}

/// Read `billing_cycle:`, if it is present and non-empty.
fn declared_cycle(fields: &HashMap<String, String>) -> Option<(BillingCycle, String)> {
    let raw = fields.get("billing_cycle")?.trim();
    if raw.is_empty() {
        return None;
    }
    let cycle = match raw.to_ascii_lowercase().as_str() {
        "weekly" => BillingCycle::Weekly,
        "quarterly" => BillingCycle::Quarterly,
        "yearly" | "annual" | "annually" => BillingCycle::Yearly,
        "once" | "one_off" | "one-off" => BillingCycle::OneOff,
        _ => BillingCycle::Monthly,
    };
    Some((cycle, raw.to_string()))
}

/// Read `currency:` as an ISO code, if it is present and three letters.
fn declared_currency(fields: &HashMap<String, String>) -> Option<String> {
    fields
        .get("currency")
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| value.len() == 3)
}

/// Settle the four spellings into one figure, or refuse and say which key failed.
///
/// This is the Q103 reader. It never converts — [`crate::money::to_eur`] owns that —
/// and it never invents a missing period or a missing unit.
pub fn read_price(fields: &HashMap<String, String>) -> Result<PriceReading, PriceRefusal> {
    let present: Vec<PriceKey> = PRICE_KEY_PRECEDENCE
        .into_iter()
        .filter(|key| {
            fields
                .get(key.as_str())
                .is_some_and(|value| !value.trim().is_empty())
        })
        .collect();
    let Some((&key, shadowed)) = present.split_first() else {
        return Err(PriceRefusal::NotStated);
    };

    let raw = fields[key.as_str()].clone();
    let Some(amount_cents) = decimal_to_cents(&raw) else {
        return Err(PriceRefusal::Unparsable {
            key,
            value: raw.trim().to_string(),
        });
    };

    let declared = declared_currency(fields);
    let currency = if key.declares_eur() {
        if let Some(declared) = declared.filter(|code| !crate::money::is_declared(code)) {
            return Err(PriceRefusal::CurrencyContradiction { key, declared });
        }
        crate::money::DECLARED_CURRENCY.to_string()
    } else {
        declared.unwrap_or_else(|| crate::money::DECLARED_CURRENCY.to_string())
    };

    let cycle = match key {
        PriceKey::YearlyCostEur => match declared_cycle(fields) {
            Some((BillingCycle::Yearly, _)) | None => BillingCycle::Yearly,
            Some((_, declared)) => return Err(PriceRefusal::CycleContradiction { key, declared }),
        },
        PriceKey::PriceEur => match declared_cycle(fields) {
            Some((cycle, _)) => cycle,
            None => return Err(PriceRefusal::CycleNotDeclared { key }),
        },
        // `cost` and `cost_eur` are recurring-subscription vocabulary, and the whole
        // directory bills monthly. An absent `billing_cycle:` reads as monthly, which
        // is what every note using these keys already means.
        PriceKey::Cost | PriceKey::CostEur => declared_cycle(fields)
            .map(|(cycle, _)| cycle)
            .unwrap_or(BillingCycle::Monthly),
    };

    Ok(PriceReading {
        key,
        amount_cents,
        currency,
        cycle,
        shadowed: shadowed.to_vec(),
    })
}

/// Map a note's frontmatter onto the shape of a subscription.
///
/// Only ever used to *seed*. Re-running it against a note whose series has since
/// moved on would throw that series away, so the store imports by path and leaves
/// an existing subscription's history alone.
///
/// The vault's own vocabulary is honoured rather than replaced: [`read_price`] states
/// the precedence over the four cost spellings and refuses where a figure would have to
/// be guessed. A `start_date` seeds the first price point's date; without one the
/// caller's `today` is used, which is wrong-but-visible rather than invented.
pub fn seed_from_note(note: &ScannedNote, today: &str) -> Subscription {
    let f = &note.fields;

    let start = f
        .get("start_date")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(today)
        .to_string();

    let prices = read_price(f)
        .map(|reading| {
            vec![PricePoint {
                valid_from: start.clone(),
                amount_cents: reading.amount_cents,
                currency: reading.currency,
                cycle: reading.cycle,
                plan: f
                    .get("plan")
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty()),
                reason: format!("seeded from the vault note (`{}`)", reading.key.as_str()),
            }]
        })
        .unwrap_or_default();

    let states = f
        .get("status")
        .and_then(|s| State::parse(s))
        .map(|state| {
            vec![StateChange {
                effective: start,
                state,
                note: "seeded from the vault note".into(),
            }]
        })
        .unwrap_or_default();

    Subscription {
        id: String::new(), // assigned by the store
        name: note.name.clone(),
        source_path: note.source_path.clone(),
        category: f.get("category").map(|c| c.trim().to_string()),
        value_rating: f.get("value_rating").and_then(|v| v.trim().parse().ok()),
        prices,
        states,
    }
}

/// The body of the derived block, as it appears between the markers.
///
/// Every line here is computed. Nothing a human typed is reproduced, because a copy
/// of somebody's prose inside a machine-owned region is a second writable home for
/// it, which is the doubling the whole boundary exists to prevent.
///
/// Two parts, and they answer different questions. The callout is the **current
/// state**: what it costs now, what happens next, whether it has drifted. The two
/// tables below it are the **series** — every price point and every state change, as
/// rows. Version 1 rendered only the callout.
///
/// The series are here because PRD Q47 (2026-08-27) counted `finance_price_points` and
/// `finance_state_changes` among the 512 irreplaceable rows in the store and made
/// projecting them a rule: a price is observed once, on the day it changed, and a
/// series cannot be recomputed from anything. A current-state summary is not a copy of
/// them. This region is the one existing machine→vault writer, so the safety copy goes
/// where the writer already is rather than into a second file.
///
/// `rates` carries the published FX observations, and PRD Q103 (2026-09-09) is why the
/// argument exists: EUR is the declared currency, so the block states the EUR figure
/// beside a foreign price, or states that it cannot. Pass an empty slice and every
/// non-EUR price renders its refusal, which is the state of a store with no FX rows.
pub fn render_block(sub: &Subscription, today: &str, rates: &[FxObservation]) -> String {
    let mut out = String::new();
    out.push_str("> [!info] Derived by Axon — do not edit inside this block\n");

    match sub.price_at(today) {
        Some(p) => {
            out.push_str(&format!(
                "> **Current price:** {} {} / {}\n",
                cents_to_decimal(p.amount_cents),
                p.currency,
                cycle_word(p.cycle)
            ));
            let monthly = p.cycle.monthly_cents(p.amount_cents);
            out.push_str(&format!(
                "> **Monthly equivalent:** {} {}\n",
                cents_to_decimal(monthly),
                p.currency
            ));
            // Q103: the monthly figure above is in the price's own currency and says
            // so. This line is the EUR one, and it refuses rather than relabels.
            match money::to_eur(monthly, &p.currency, rates) {
                Ok(EurAmount::Declared { .. }) => {}
                Ok(EurAmount::Converted { cents, rate, .. }) => out.push_str(&format!(
                    "> **Monthly in {}:** {} {} (at the {}/{} rate of {}, {})\n",
                    money::DECLARED_CURRENCY,
                    cents_to_decimal(cents),
                    money::DECLARED_CURRENCY,
                    rate.base,
                    rate.quote,
                    rate.observed_on,
                    rate.source,
                )),
                Err(refusal) => out.push_str(&format!(
                    "> **Monthly in {}:** not stated — {}\n",
                    money::DECLARED_CURRENCY,
                    refusal.detail
                )),
            }
            if let Some(plan) = &p.plan {
                out.push_str(&format!("> **Plan:** {plan}\n"));
            }
        }
        None => out.push_str("> **Current price:** not recorded yet\n"),
    }

    out.push_str(&format!("> **State:** {}\n", sub.state_at(today).as_str()));

    // Drift only when something has actually drifted. The first version keyed off
    // `prices.len() > 1`, which rendered "drift: down 0.00" for a subscription
    // whose second price point is still in the future — a line that says a price
    // moved when none has, on the one surface meant to notice exactly that.
    if let Some(first) = sub.prices.first() {
        match sub.price_drift_cents(&first.valid_from, today) {
            Some(drift) if drift != 0 => {
                out.push_str(&format!(
                    "> **Price drift since {}:** {} {} {}\n",
                    first.valid_from,
                    if drift > 0 { "up" } else { "down" },
                    cents_to_decimal(drift.abs()),
                    first.currency
                ));
            }
            _ => {}
        }
    }

    // A price point dated ahead of today is the increase you want warning about
    // before it bills, which is the whole reason the series is dated rather than
    // overwritten. `scheduled_price_after` rather than a local `min_by`, so the row
    // announced here is the row that will actually be in force on its own date.
    if let Some(next) = sub.scheduled_price_after(today) {
        out.push_str(&format!(
            "> **Scheduled:** {}{} {} / {} from {}\n",
            next.plan
                .as_deref()
                .map(|p| format!("{p}, "))
                .unwrap_or_default(),
            cents_to_decimal(next.amount_cents),
            next.currency,
            cycle_word(next.cycle),
            next.valid_from
        ));
    }

    // The count line that used to sit here is gone. The table below holds every price
    // point, so a count beside it is the same fact written twice, and the second copy
    // is the one that goes wrong.

    out.push_str("\n#### Price series\n\n");
    if sub.prices.is_empty() {
        out.push_str("None recorded.\n");
    } else {
        out.push_str("| From | Amount | Cycle | Plan | Reason |\n|---|---|---|---|---|\n");
        for price in &sub.prices {
            out.push_str(&format!(
                "| {} | {} {} | {} | {} | {} |\n",
                price.valid_from,
                cents_to_decimal(price.amount_cents),
                price.currency,
                cycle_word(price.cycle),
                cell(price.plan.as_deref().unwrap_or("")),
                cell(&price.reason),
            ));
        }
    }

    out.push_str("\n#### State series\n\n");
    if sub.states.is_empty() {
        out.push_str("None recorded.\n");
    } else {
        out.push_str("| From | State | Note |\n|---|---|---|\n");
        for change in &sub.states {
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                change.effective,
                change.state.as_str(),
                cell(&change.note),
            ));
        }
    }

    out
}

/// One table cell.
///
/// A `|` in a reason ends the row early and shifts every column after it, which turns
/// a safety copy into a wrong one silently. A newline does the same to the whole table.
/// Both are escaped rather than stripped: the text is the record.
fn cell(text: &str) -> String {
    let flattened = text.replace(['\n', '\r'], " ");
    let escaped = flattened.replace('|', "\\|");
    let trimmed = escaped.trim();
    if trimmed.is_empty() {
        "—".to_string()
    } else {
        trimmed.to_string()
    }
}

fn cycle_word(cycle: BillingCycle) -> &'static str {
    match cycle {
        BillingCycle::Weekly => "week",
        BillingCycle::Monthly => "month",
        BillingCycle::Quarterly => "quarter",
        BillingCycle::Yearly => "year",
        BillingCycle::OneOff => "one-off",
    }
}

/// Q31's home for a subscription whose note is no longer in the vault.
///
/// Vault-relative, not configurable — a second declaration of where machine output goes
/// is how two hosts write to two folders and neither notices.
// `Resources/Axon/Subscriptions` until 2026-10-08, when the vault folder took the product name.
pub const PROJECTION_DIR: &str = "Resources/Sjel/Subscriptions";

/// One subscription as a whole file, for the case where no note exists to hold a region.
///
/// This is not a fallback that was designed for; it is the measured state of the vault.
/// The 2026-08-23 reorganisation (vault commit `ba60231`, ruled in PRD §5.5) moved all
/// 37 finance notes out to `<overlay>/data/finance/vault-notes/` on the grounds that
/// they are entity rows rather than things a human wrote sentences into. So on
/// 2026-08-28 every one of the seven live subscriptions pointed at a note that is not in
/// the vault, and the region writer had nothing to write into. The series was in no file
/// anywhere, which is exactly the exposure Q47 named.
///
/// Q31 answers it directly: write a region when a human note for the subject exists, and
/// create a projected file only when none does. This is the "none does" branch. The
/// region path above is unchanged and takes over the moment a note comes back, because
/// the writeback prefers it.
///
/// The body is `render_block`'s, so the two paths cannot drift into two shapes of the
/// same figures.
pub fn render_projection(sub: &Subscription, today: &str, rates: &[FxObservation]) -> String {
    let mut out = String::from("---\n");
    let field = |out: &mut String, key: &str, value: &str| {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!("{key}: \"{escaped}\"\n"));
    };
    field(&mut out, "axon_subscription_id", &sub.id);
    field(&mut out, "title", &sub.name);
    // Where the human note is expected. It is the store's import identity, so a note
    // written back at this path is the one that ends this projection.
    field(&mut out, "axon_source_path", &sub.source_path);
    if let Some(category) = &sub.category {
        field(&mut out, "category", category);
    }
    if let Some(rating) = sub.value_rating {
        field(&mut out, "value_rating", &rating.to_string());
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", sub.name));
    out.push_str(&render_block(sub, today, rates));
    out
}

/// Project every subscription that has no note in the vault, and sweep the rest.
///
/// A projection is removed when its subscription is gone **or** when a note for it has
/// appeared: Q31's promotion, run automatically. `owned` carries the vault-relative
/// paths of the notes the scanner found, which is what "a note exists" means here.
pub fn export_projections(
    root: &MarkdownRoot,
    subs: &[Subscription],
    owned: &[String],
    today: &str,
    rates: &[FxObservation],
) -> Result<ProjectionReport, markdown_root::RootError> {
    let spec = RegionSpec::new(REGION_OWNER, REGION_VERSION);
    let mut report = ProjectionReport::default();
    let mut wanted: Vec<String> = Vec::new();

    for sub in subs {
        if owned.iter().any(|path| path == &sub.source_path) {
            continue;
        }
        let stem = markdown_root::projection::file_stem(&sub.name, &sub.id);
        let path = format!("{PROJECTION_DIR}/{stem}.md");
        match root.write_projection(&path, &spec, &render_projection(sub, today, rates))? {
            markdown_root::ProjectionOutcome::Created => report.created += 1,
            markdown_root::ProjectionOutcome::Updated => report.updated += 1,
            markdown_root::ProjectionOutcome::Unchanged => report.unchanged += 1,
            markdown_root::ProjectionOutcome::NotOurs => report.refused.push(path.clone()),
        }
        wanted.push(path);
    }

    // A missing folder is zero files, not an error: nothing has been projected yet.
    let existing = match root.markdown_files(&format!("{PROJECTION_DIR}/*.md")) {
        Ok(files) => files,
        Err(markdown_root::RootError::Unreadable { .. }) => Vec::new(),
        Err(e) => return Err(e),
    };
    for file in existing {
        let Some(id) = root.relative_id(&file) else {
            continue;
        };
        if wanted.contains(&id) {
            continue;
        }
        if root.remove_projection(&id, &spec)? {
            report.removed.push(id);
        }
    }
    Ok(report)
}

/// What one projection pass did.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ProjectionReport {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    /// A file exists at the path and this capability did not write it.
    pub refused: Vec<String>,
    /// The subscription is gone, or its note is back and the region owns it now.
    pub removed: Vec<String>,
}

/// What a writeback attempt did to one note.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteBack {
    Created,
    Updated,
    /// Already correct. The file was not opened for writing at all, so the vault's
    /// git history stays free of no-op commits.
    Unchanged,
    /// A human edited inside the region. Nothing was written, and both revisions
    /// come back so the caller can show them rather than pick one.
    Conflict {
        theirs: String,
        ours: String,
    },
}

/// Regenerate one note's derived block.
///
/// The write happens only on `Created` and `Updated`. Every other outcome, conflict
/// included, leaves the file exactly as it was found.
pub fn write_block(
    path: &Path,
    sub: &Subscription,
    today: &str,
    rates: &[FxObservation],
) -> Result<WriteBack, Box<dyn std::error::Error>> {
    let original = std::fs::read_to_string(path)?;
    let spec = RegionSpec::new(REGION_OWNER, REGION_VERSION);
    let (updated, outcome) = region::apply(&original, &spec, &render_block(sub, today, rates))?;

    match outcome {
        RegionOutcome::Created => {
            std::fs::write(path, updated)?;
            Ok(WriteBack::Created)
        }
        RegionOutcome::Updated => {
            std::fs::write(path, updated)?;
            Ok(WriteBack::Updated)
        }
        RegionOutcome::Unchanged => Ok(WriteBack::Unchanged),
        RegionOutcome::Conflict { theirs, ours } => Ok(WriteBack::Conflict { theirs, ours }),
    }
}

/// A note and its series disagree about what the money is.
///
/// Reported, never resolved, in the same spirit as the writeback's region conflicts.
/// Both sides are shown because either can be the wrong one: the human may have
/// corrected the frontmatter, or the series may hold the correction and the frontmatter
/// the stale figure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CurrencyMismatch {
    pub source_path: String,
    pub subscription_id: String,
    /// The key the note states its price under, and the currency that implies.
    pub note_key: PriceKey,
    pub note_currency: String,
    /// The currency of the price point in force, and the date it took effect.
    pub series_currency: String,
    pub series_valid_from: String,
    pub detail: String,
}

/// Every note whose declared currency differs from the currency of its price in force.
///
/// The guard PRD Q103 asks for, and the shape of the case that produced the ruling: a
/// note declares `currency: USD` while the price point in force, a stale seed written
/// before the importer read `currency:`, is EUR, so the block Sjel wrote into that note
/// reads `EUR / month`. The store never re-seeds, by design, so a corrected
/// `currency:` cannot reach the series on its own. Detecting the divergence
/// is what turns a silent wrong figure into a visible one.
///
/// A note with no price key, and a subscription with no price in force, are both
/// silent here: there is nothing to disagree about. A note whose price key is refused
/// (`price_eur` with no cycle, a contradiction) is likewise not a currency mismatch —
/// [`read_price`] already reports that.
pub fn currency_mismatches(
    notes: &[ScannedNote],
    subs: &[Subscription],
    today: &str,
) -> Vec<CurrencyMismatch> {
    let mut out = Vec::new();
    for note in notes {
        let Some(sub) = subs.iter().find(|s| s.source_path == note.source_path) else {
            continue;
        };
        let Some(price) = sub.price_at(today) else {
            continue;
        };
        let Ok(reading) = read_price(&note.fields) else {
            continue;
        };
        if reading.currency.eq_ignore_ascii_case(&price.currency) {
            continue;
        }
        out.push(CurrencyMismatch {
            source_path: note.source_path.clone(),
            subscription_id: sub.id.clone(),
            note_key: reading.key,
            note_currency: reading.currency.clone(),
            series_currency: price.currency.clone(),
            series_valid_from: price.valid_from.clone(),
            detail: format!(
                "the note states {} under `{}` and the price point of {} is in {}; the derived block will say {}",
                reading.currency,
                reading.key.as_str(),
                price.valid_from,
                price.currency,
                price.currency,
            ),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(fields: &[(&str, &str)], name: &str) -> ScannedNote {
        ScannedNote {
            source_path: format!("Atlas/Finance/Subscriptions/{name}.md"),
            absolute: PathBuf::from(format!("/nowhere/{name}.md")),
            name: name.to_string(),
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_notes_cost_becomes_the_first_price_point_not_a_mutable_field() {
        let sub = seed_from_note(
            &note(
                &[
                    ("cost_eur", "20"),
                    ("billing_cycle", "monthly"),
                    ("status", "active"),
                    ("start_date", "2026-02-01"),
                    ("value_rating", "5"),
                    ("category", "productivity"),
                ],
                "Example",
            ),
            "2026-08-08",
        );

        assert_eq!(sub.prices.len(), 1);
        assert_eq!(sub.prices[0].amount_cents, 2000);
        assert_eq!(sub.prices[0].valid_from, "2026-02-01");
        assert_eq!(sub.states[0].state, State::Active);
        assert_eq!(sub.value_rating, Some(5));
        assert_eq!(sub.category.as_deref(), Some("productivity"));
    }

    #[test]
    fn generic_cost_and_currency_support_non_euro_notes() {
        let note = note(
            &[
                ("cost", "12.50"),
                ("currency", "usd"),
                ("billing_cycle", "monthly"),
                ("status", "active"),
            ],
            "Synthetic USD plan",
        );
        let subscription = seed_from_note(&note, "2026-08-09");
        assert_eq!(subscription.prices[0].amount_cents, 1_250);
        assert_eq!(subscription.prices[0].currency, "USD");
    }

    #[test]
    fn a_yearly_note_seeds_a_yearly_cycle() {
        let sub = seed_from_note(
            &note(&[("cost_eur", "120"), ("billing_cycle", "yearly")], "Drive"),
            "2026-08-08",
        );
        assert_eq!(sub.prices[0].cycle, BillingCycle::Yearly);
        assert_eq!(
            sub.monthly_cents_at("2026-08-08"),
            0,
            "no status means not billing"
        );
    }

    #[test]
    fn a_note_with_no_cost_seeds_no_price_rather_than_a_zero() {
        let sub = seed_from_note(&note(&[("status", "considering")], "Maybe"), "2026-08-08");
        assert!(sub.prices.is_empty());
        assert_eq!(sub.state_at("2026-08-08"), State::Considering);
    }

    #[test]
    fn a_missing_start_date_falls_back_to_today_rather_than_inventing_one() {
        let sub = seed_from_note(&note(&[("cost_eur", "9,99")], "Thing"), "2026-08-08");
        assert_eq!(sub.prices[0].valid_from, "2026-08-08");
        assert_eq!(sub.prices[0].amount_cents, 999);
    }

    #[test]
    fn the_rendered_block_is_entirely_computed() {
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "Subscriptions/Example.md".into(),
            category: None,
            value_rating: None,
            prices: vec![
                PricePoint {
                    valid_from: "2026-02-01".into(),
                    amount_cents: 2000,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: None,
                    reason: String::new(),
                },
                PricePoint {
                    valid_from: "2026-07-01".into(),
                    amount_cents: 2500,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: None,
                    reason: "provider raised it".into(),
                },
            ],
            states: vec![StateChange {
                effective: "2026-02-01".into(),
                state: State::Active,
                note: String::new(),
            }],
        };

        let block = render_block(&sub, "2026-08-08", &[]);
        assert!(block.contains("**Current price:** 25.00 EUR / month"));
        assert!(block.contains("**Monthly equivalent:** 25.00 EUR"));
        assert!(block.contains("**State:** active"));
        assert!(block.contains("**Price drift since 2026-02-01:** up 5.00 EUR"));
        // The count line this used to assert is gone: the series table below carries
        // every price point, and a count beside it is one fact written twice.
        assert!(block.contains("| 2026-02-01 | 20.00 EUR | month | — | — |"));
        assert!(block.contains("| 2026-07-01 | 25.00 EUR | month | — | provider raised it |"));
        assert!(block.contains("| 2026-02-01 | active | — |"));
    }

    /// The reason the series is in the region at all (PRD Q47): these rows exist
    /// nowhere else, so every one of them has to survive the render. A current-state
    /// summary is not a copy of a series.
    #[test]
    fn every_price_point_and_state_change_reaches_the_block() {
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "x.md".into(),
            category: None,
            value_rating: None,
            prices: (0..5)
                .map(|i| PricePoint {
                    valid_from: format!("2026-0{}-01", i + 1),
                    amount_cents: 1000 + i * 100,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: Some(format!("tier-{i}")),
                    reason: format!("step {i}"),
                })
                .collect(),
            states: (0..3)
                .map(|i| StateChange {
                    effective: format!("2026-0{}-01", i + 1),
                    state: State::Active,
                    note: format!("note {i}"),
                })
                .collect(),
        };
        let block = render_block(&sub, "2026-08-08", &[]);
        for i in 0..5 {
            assert!(block.contains(&format!("step {i}")), "price point {i} lost");
            assert!(block.contains(&format!("tier-{i}")), "plan {i} lost");
        }
        for i in 0..3 {
            assert!(
                block.contains(&format!("note {i}")),
                "state change {i} lost"
            );
        }
    }

    fn subscription(name: &str, source_path: &str) -> Subscription {
        Subscription {
            id: format!("sub_{name}"),
            name: name.to_string(),
            source_path: source_path.to_string(),
            category: Some("productivity".into()),
            value_rating: Some(5),
            prices: vec![PricePoint {
                valid_from: "2026-02-01".into(),
                amount_cents: 2000,
                currency: "EUR".into(),
                cycle: BillingCycle::Monthly,
                plan: None,
                reason: "initial".into(),
            }],
            states: vec![StateChange {
                effective: "2026-02-01".into(),
                state: State::Active,
                note: String::new(),
            }],
        }
    }

    fn temp_root() -> (PathBuf, MarkdownRoot) {
        let dir = std::env::temp_dir().join(format!(
            "axon-finance-projection-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();
        (dir, root)
    }

    /// Q31's rule, both directions: a subscription with no note gets a file, and the
    /// file goes away the moment a note for it exists — because the region in that note
    /// then owns the figures, and two homes for one number is the failure the whole
    /// boundary exists to prevent.
    #[test]
    fn a_subscription_is_projected_only_while_it_has_no_note() {
        let (dir, root) = temp_root();
        let subs = vec![subscription(
            "Claude Max",
            "Atlas/Finance/Subscriptions/Claude Max.md",
        )];

        let first = export_projections(&root, &subs, &[], "2026-08-28", &[]).unwrap();
        assert_eq!((first.created, first.unchanged), (1, 0));
        let file = dir.join("Resources/Sjel/Subscriptions/Claude Max.md");
        let body = std::fs::read_to_string(&file).unwrap();
        assert!(
            body.contains("| 2026-02-01 | 20.00 EUR | month |"),
            "{body}"
        );

        let second = export_projections(&root, &subs, &[], "2026-08-28", &[]).unwrap();
        assert_eq!((second.created, second.unchanged), (0, 1));

        let owned = vec!["Atlas/Finance/Subscriptions/Claude Max.md".to_string()];
        let third = export_projections(&root, &subs, &owned, "2026-08-28", &[]).unwrap();
        assert_eq!(
            third.removed,
            vec!["Resources/Sjel/Subscriptions/Claude Max.md"]
        );
        assert!(!file.exists(), "the note's region owns the figures now");

        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A pipe in a reason ends the row early and shifts every column after it, which
    /// is how a safety copy becomes a wrong one without anything failing.
    #[test]
    fn a_pipe_in_a_reason_is_escaped_rather_than_breaking_the_row() {
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "x.md".into(),
            category: None,
            value_rating: None,
            prices: vec![PricePoint {
                valid_from: "2026-02-01".into(),
                amount_cents: 2000,
                currency: "EUR".into(),
                cycle: BillingCycle::Monthly,
                plan: None,
                reason: "moved Pro | Max\nafter the mail".into(),
            }],
            states: vec![],
        };
        let block = render_block(&sub, "2026-08-08", &[]);
        assert!(block.contains("moved Pro \\| Max after the mail"));
        let row = block
            .lines()
            .find(|l| l.contains("2026-02-01") && l.starts_with('|'))
            .expect("a price row");
        assert_eq!(
            row.matches("| ").count() - row.matches("\\| ").count(),
            5,
            "five cells, however many pipes the text held: {row}"
        );
    }

    #[test]
    fn a_subscription_with_no_price_says_so_rather_than_rendering_zero() {
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "x.md".into(),
            category: None,
            value_rating: None,
            prices: vec![],
            states: vec![],
        };
        let block = render_block(&sub, "2026-08-08", &[]);
        assert!(block.contains("not recorded yet"));
        assert!(!block.contains("0.00"));
    }

    #[test]
    fn a_future_price_point_is_announced_rather_than_reported_as_drift() {
        // The case the fixture run caught: a second price point dated ahead of
        // today. Nothing has drifted, and saying so would be a false alarm on the
        // one surface built to raise real ones.
        let sub = Subscription {
            id: "s1".into(),
            name: "Example".into(),
            source_path: "x.md".into(),
            category: None,
            value_rating: None,
            prices: vec![
                PricePoint {
                    valid_from: "2026-08-08".into(),
                    amount_cents: 2000,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: None,
                    reason: String::new(),
                },
                PricePoint {
                    valid_from: "2026-10-01".into(),
                    amount_cents: 10_000,
                    currency: "EUR".into(),
                    cycle: BillingCycle::Monthly,
                    plan: Some("Max".into()),
                    reason: "upgrade".into(),
                },
            ],
            states: vec![StateChange {
                effective: "2026-08-08".into(),
                state: State::Active,
                note: String::new(),
            }],
        };

        let block = render_block(&sub, "2026-08-08", &[]);
        assert!(!block.contains("drift"), "nothing has drifted yet");
        // The plan rides along, so the line answers "to what" as well as "to how much".
        assert!(block.contains("**Scheduled:** Max, 100.00 EUR / month from 2026-10-01"));
        assert!(block.contains("**Current price:** 20.00 EUR / month"));

        // Once it lands, it is drift and there is nothing left to schedule.
        let after = render_block(&sub, "2026-10-02", &[]);
        assert!(after.contains("**Price drift since 2026-08-08:** up 80.00 EUR"));
        assert!(!after.contains("Scheduled"));
    }

    #[test]
    fn a_single_price_point_renders_no_drift_line() {
        let sub = seed_from_note(
            &note(
                &[
                    ("cost_eur", "20"),
                    ("status", "active"),
                    ("start_date", "2026-02-01"),
                ],
                "Example",
            ),
            "2026-08-08",
        );
        let block = render_block(&sub, "2026-08-08", &[]);
        assert!(
            !block.contains("drift"),
            "no drift to report from one point"
        );
    }

    // -- Q103: the four cost spellings, one fixture per note shape ------------------
    //
    // The fixtures below are synthetic: invented names and amounts, one per frontmatter
    // shape the reader has to settle. They are written here rather than read from a
    // vault, because a test that reads a live vault passes or fails for reasons that
    // have nothing to do with this code.

    #[test]
    fn a_card_fee_under_price_eur_without_a_cycle_is_refused_not_read_as_monthly() {
        // `price_eur: 240` is an annual card fee. Before Q103 the key was not read at
        // all, so the note seeded no price; the trap is in adding it to the alias
        // chain without the cycle rule, because `billing_cycle:` is absent and the
        // default for the other three keys is monthly. That reading would state
        // 240.00 EUR *a month*.
        let fields = note(
            &[
                ("category", "other"),
                ("price_eur", "240"),
                ("status", "decided-yes"),
            ],
            "Meridian Card",
        );
        assert_eq!(
            read_price(&fields.fields),
            Err(PriceRefusal::CycleNotDeclared {
                key: PriceKey::PriceEur
            })
        );
        let sub = seed_from_note(&fields, "2026-09-09");
        assert!(sub.prices.is_empty(), "a refusal seeds no price");

        // One line of frontmatter settles it, and then the figure is annual.
        let fixed = note(
            &[
                ("price_eur", "240"),
                ("billing_cycle", "yearly"),
                ("status", "decided-yes"),
            ],
            "Meridian Card",
        );
        let reading = read_price(&fixed.fields).unwrap();
        assert_eq!(reading.key, PriceKey::PriceEur);
        assert_eq!(reading.amount_cents, 24_000);
        assert_eq!(reading.currency, "EUR");
        assert_eq!(reading.cycle, BillingCycle::Yearly);
    }

    #[test]
    fn a_travel_pass_has_the_same_shape_as_the_card_and_the_same_answer() {
        let fields = note(
            &[
                ("category", "transport"),
                ("price_eur", "1490"),
                ("status", "decided-yes"),
            ],
            "Rail Pass",
        );
        assert!(read_price(&fields.fields).is_err());
        assert!(seed_from_note(&fields, "2026-09-09").prices.is_empty());
    }

    #[test]
    fn cost_with_an_explicit_eur_currency_is_read_as_eur() {
        let fields = note(
            &[
                ("cost", "35"),
                ("currency", "EUR"),
                ("plan", "Pro"),
                ("billing_cycle", "monthly"),
                ("start_date", "2026-05-15"),
                ("status", "cancelled"),
            ],
            "Harbor Cloud",
        );
        let reading = read_price(&fields.fields).unwrap();
        assert_eq!(reading.key, PriceKey::Cost);
        assert_eq!(reading.amount_cents, 3_500);
        assert_eq!(reading.currency, "EUR");
        assert!(reading.shadowed.is_empty());
    }

    #[test]
    fn usd_cost_is_read_and_shadows_the_yearly_eur_twin() {
        // The collision Q103 is about. `cost: 45 / currency: USD` and
        // `yearly_cost_eur: 540` are two different claims about one price: 45 USD a
        // month is 540 *USD* a year, not 540 EUR. `cost` wins and the other is
        // recorded as shadowed rather than averaged in or silently dropped.
        let fields = note(
            &[
                ("cost", "45"),
                ("currency", "USD"),
                ("billing_cycle", "monthly"),
                ("yearly_cost_eur", "540"),
                ("start_date", "2026-07-01"),
                ("status", "cancelled"),
            ],
            "Studio Max",
        );
        let reading = read_price(&fields.fields).unwrap();
        assert_eq!(reading.key, PriceKey::Cost);
        assert_eq!(
            reading.currency, "USD",
            "the note says USD, so the seed does"
        );
        assert_eq!(reading.shadowed, vec![PriceKey::YearlyCostEur]);

        // And the seeded point carries USD, so the region cannot render it as EUR.
        let sub = seed_from_note(&fields, "2026-09-09");
        assert_eq!(sub.prices[0].currency, "USD");
        let block = render_block(&sub, "2026-09-09", &[]);
        assert!(block.contains("**Current price:** 45.00 USD / month"));
        assert!(
            block.contains("**Monthly in EUR:** not stated — no published EUR/USD rate"),
            "a USD price with no rate must refuse, not relabel:\n{block}"
        );
        assert!(!block.contains("45.00 EUR"));
    }

    #[test]
    fn usd_cost_with_an_empty_yearly_twin_does_not_shadow_anything() {
        // `yearly_cost_eur:` is present but empty in the note. An empty value is not a
        // figure, so it is neither read nor counted as a competing claim.
        let fields = note(
            &[
                ("cost", "15"),
                ("currency", "USD"),
                ("yearly_cost_eur", ""),
                ("billing_cycle", "monthly"),
                ("status", "active"),
                ("start_date", "2026-08-01"),
            ],
            "Studio Lite",
        );
        let reading = read_price(&fields.fields).unwrap();
        assert_eq!(reading.amount_cents, 1500);
        assert_eq!(reading.currency, "USD");
        assert!(reading.shadowed.is_empty());
    }

    #[test]
    fn the_cost_eur_alias_is_read_and_needs_no_currency_field() {
        let fields = note(
            &[
                ("cost_eur", "12"),
                ("yearly_cost_eur", ""),
                ("billing_cycle", "monthly"),
                ("status", "covered"),
            ],
            "Vault Storage",
        );
        let reading = read_price(&fields.fields).unwrap();
        assert_eq!(reading.key, PriceKey::CostEur);
        assert!(reading.key.is_deprecated());
        assert_eq!(reading.currency, "EUR");
        assert_eq!(reading.amount_cents, 1200);
    }

    #[test]
    fn a_second_usd_note_reads_usd_from_cost() {
        let fields = note(
            &[
                ("cost", "8"),
                ("currency", "USD"),
                ("billing_cycle", "monthly"),
                ("start_date", "2026-08-01"),
                ("status", "active"),
            ],
            "Pixel Relay",
        );
        let reading = read_price(&fields.fields).unwrap();
        assert_eq!(reading.amount_cents, 800);
        assert_eq!(reading.currency, "USD");
    }

    #[test]
    fn a_eur_key_beside_a_foreign_currency_field_is_a_contradiction_not_an_override() {
        let fields = note(&[("cost_eur", "20"), ("currency", "USD")], "Planted");
        assert_eq!(
            read_price(&fields.fields),
            Err(PriceRefusal::CurrencyContradiction {
                key: PriceKey::CostEur,
                declared: "USD".into()
            })
        );
    }

    #[test]
    fn a_yearly_key_beside_a_monthly_cycle_is_a_contradiction() {
        let fields = note(
            &[("yearly_cost_eur", "1200"), ("billing_cycle", "monthly")],
            "Planted",
        );
        assert_eq!(
            read_price(&fields.fields),
            Err(PriceRefusal::CycleContradiction {
                key: PriceKey::YearlyCostEur,
                declared: "monthly".into()
            })
        );
    }

    #[test]
    fn an_unparsable_cost_names_the_key_rather_than_seeding_nothing_quietly() {
        let fields = note(&[("cost", "free")], "Planted");
        assert_eq!(
            read_price(&fields.fields),
            Err(PriceRefusal::Unparsable {
                key: PriceKey::Cost,
                value: "free".into()
            })
        );
    }

    #[test]
    fn a_converted_price_states_the_rate_it_used() {
        let sub = seed_from_note(
            &note(
                &[
                    ("cost", "20"),
                    ("currency", "USD"),
                    ("status", "active"),
                    ("start_date", "2026-08-01"),
                ],
                "Pixel Relay",
            ),
            "2026-09-09",
        );
        let rates = vec![FxObservation {
            base: "EUR".into(),
            quote: "USD".into(),
            observed_on: "2026-09-08".into(),
            rate: crate::investment::Quantity {
                mantissa: 10_850,
                scale: 4,
            },
            source: "ecb".into(),
            fetched_at: "2026-09-08T16:00:00Z".into(),
        }];
        let block = render_block(&sub, "2026-09-09", &rates);
        assert!(block.contains("**Current price:** 20.00 USD / month"));
        assert!(
            block
                .contains("**Monthly in EUR:** 18.43 EUR (at the EUR/USD rate of 2026-09-08, ecb)"),
            "a converted figure must carry its source:\n{block}"
        );
    }

    #[test]
    fn a_eur_price_gains_no_conversion_line_because_there_is_nothing_to_convert() {
        let sub = seed_from_note(
            &note(&[("cost_eur", "20"), ("status", "active")], "Vault Storage"),
            "2026-09-09",
        );
        let block = render_block(&sub, "2026-09-09", &[]);
        assert!(block.contains("**Monthly equivalent:** 20.00 EUR"));
        assert!(!block.contains("**Monthly in EUR:**"));
    }

    // -- Q103: the guard --------------------------------------------------------------

    fn imported(name: &str, prices: Vec<PricePoint>) -> Subscription {
        Subscription {
            id: format!("sub_{name}"),
            name: name.into(),
            source_path: format!("Atlas/Finance/Subscriptions/{name}.md"),
            category: None,
            value_rating: None,
            prices,
            states: vec![StateChange {
                effective: "2026-07-01".into(),
                state: State::Active,
                note: String::new(),
            }],
        }
    }

    fn point(from: &str, cents: i64, currency: &str, reason: &str) -> PricePoint {
        PricePoint {
            valid_from: from.into(),
            amount_cents: cents,
            currency: currency.into(),
            cycle: BillingCycle::Monthly,
            plan: None,
            reason: reason.into(),
        }
    }

    #[test]
    fn the_guard_stays_silent_when_the_note_and_the_series_agree() {
        let notes = vec![note(
            &[
                ("cost", "8"),
                ("currency", "USD"),
                ("billing_cycle", "monthly"),
                ("status", "active"),
            ],
            "Pixel Relay",
        )];
        let subs = vec![imported(
            "Pixel Relay",
            vec![point("2026-08-01", 800, "USD", "seeded")],
        )];
        assert_eq!(currency_mismatches(&notes, &subs, "2026-09-09"), Vec::new());
    }

    #[test]
    fn the_guard_refuses_a_stale_eur_seed_under_a_usd_note() {
        // The planted bad input: the note declares USD and the price point in force, a
        // stale seed dated after the corrected point, is EUR.
        let notes = vec![note(
            &[
                ("cost", "45"),
                ("currency", "USD"),
                ("billing_cycle", "monthly"),
                ("status", "cancelled"),
            ],
            "Studio Max",
        )];
        let subs = vec![imported(
            "Studio Max",
            vec![
                point("2026-07-01", 4_500, "USD", "reviewed plan history"),
                point("2026-08-08", 4_500, "EUR", "seeded from the vault note"),
            ],
        )];

        let found = currency_mismatches(&notes, &subs, "2026-09-09");
        assert_eq!(found.len(), 1, "the mismatch must be reported: {found:?}");
        assert_eq!(found[0].note_currency, "USD");
        assert_eq!(found[0].series_currency, "EUR");
        assert_eq!(found[0].series_valid_from, "2026-08-08");
        assert_eq!(found[0].note_key, PriceKey::Cost);
        assert!(found[0].detail.contains("2026-08-08"));

        // Appending the correction the note already implies clears it, and nothing
        // else has to change.
        let mut corrected = subs;
        corrected[0]
            .prices
            .push(point("2026-09-09", 4_500, "USD", "currency correction"));
        assert_eq!(
            currency_mismatches(&notes, &corrected, "2026-09-09"),
            Vec::new()
        );
    }

    #[test]
    fn the_guard_is_silent_where_there_is_nothing_to_compare() {
        // A note whose price key is refused, and a subscription with no price in
        // force, are both absences rather than disagreements.
        let notes = vec![note(&[("price_eur", "240")], "Meridian Card")];
        let subs = vec![imported("Meridian Card", Vec::new())];
        assert_eq!(currency_mismatches(&notes, &subs, "2026-09-09"), Vec::new());
    }
}
