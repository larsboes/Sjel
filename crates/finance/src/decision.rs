//! The rule engine, the ledger's read model, and the human-readable copy.
//!
//! Principle 1 made structural rather than documentary: rung 1 is three rules
//! here, and every proposal names the rung that produced it. No proposal in this
//! build carries `rung = "model"`, and `every_proposal_names_its_rung_and_none_is_model`
//! is the test rather than a paragraph. The risk model in `risk.rs` explains; it
//! does not propose.
//!
//! **`sell` is accepted by the table's CHECK and minted by no rule.** A machine
//! proposing the sale of a real position is a different order of claim from
//! proposing a rebalance, and nothing measured justifies it. A sell rule needs
//! its own trigger and its own ruling.
//!
//! **The verdict writes the month file in the same call.** A human verdict plus
//! its note is the one fact in this capability that no re-import and no re-run
//! reproduces, and PRD:234 calls the database "a rebuildable index, never the
//! only copy". `finance decisions export` is the copy a human runs when the
//! server is down, exactly as `capabilities/trips/src/bin/trips-cli.rs` describes
//! its own role -- never the only writer.
//!
//! **Class c1, validated at every write site.** PRD:1201-1202: C1 is "My notes,
//! plans, drafts, journals", C2 is "Facts about named people", and a proposal
//! about the owner's own allocation names no third party. The consequence is
//! implemented rather than merely stated: c1 rows are stored verbatim, which is
//! what this module does, so no redaction obligation is left open.

use std::collections::BTreeMap;
use std::io::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::clock;
use crate::config::{InstrumentProfile, TargetPolicy};
use crate::planning::PlanningReport;
use crate::portfolio::PortfolioReport;
use crate::store::{DecisionRunOutcome, FinanceStore, StoredDecision, StoredProposal};

/// Bumped with any change to the rules below, so a stored row records which rule
/// set decided it. Mirrors `content_item::MAIL_CLASSIFIER_VERSION`'s role.
pub const ENGINE_REVISION: &str = "finance-decisions-1";

/// The class every row in this ledger carries, and the reason it does.
pub const DATA_CLASS: &str = "c1";
pub const DATA_CLASS_RATIONALE: &str =
    "A proposal about the owner's own allocation names no third party (PRD 1201-1202).";

/// Drift is bucketed to 10 bp and cash to whole currency units before the id is
/// hashed, so a re-run that moves a number by less than a bucket re-proposes
/// nothing and the inbox does not churn.
const DRIFT_BUCKET_BP: i64 = 10;

/// The most feed items one proposal may carry. Evidence, bounded: this is a
/// pointer to something worth reading, never a finding, and an unbounded list
/// would read as one.
const MAX_FEED_ITEMS: usize = 3;

/// Below this, an instrument label is not used as a matcher at all. A two-letter
/// label matched against article titles finds nothing but noise.
const MIN_LABEL_LENGTH: usize = 3;

/// The directory, relative to the overlay root, that holds the monthly copies.
pub const EXPORT_DIR: &str = "data/finance/decisions";

// ---------------------------------------------------------------------------
// Wire shapes
// ---------------------------------------------------------------------------

/// One feed item, copied by value and bounded to four fields.
///
/// Only `id`, `title`, `url` and `day` -- never body text. Those four are C0 by
/// PRD:1198-1204 ("Published work, public sources"), and deciding that once here,
/// at the point of copy, is what keeps Principle 2 from being decided by accident
/// at a call site. `FeedListItem` carries no `data_class`, so nothing travels
/// with the text and the copy must be bounded rather than trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedEvidence {
    pub id: String,
    pub title: String,
    pub url: String,
    pub day: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// Every number the proposal rests on, named.
    pub numbers: BTreeMap<String, i64>,
    #[serde(default)]
    pub feed_items: Vec<FeedEvidence>,
    #[serde(default)]
    pub caveats: Vec<String>,
    /// Whether the risk model could say anything about this subject, and why not
    /// when it could not. `null` figures are the honest answer before a history
    /// exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub kind: String,
    pub subject: String,
    pub title: String,
    /// One sentence a human reads first. Never a restatement of the title.
    pub summary: String,
    pub rung: String,
    #[serde(default)]
    pub instrument: Option<String>,
    #[serde(default)]
    pub asset_class: Option<String>,
    #[serde(default)]
    pub actual_bp: Option<i64>,
    #[serde(default)]
    pub target_bp: Option<i64>,
    #[serde(default)]
    pub band_bp: Option<i64>,
    #[serde(default)]
    pub drift_bp: Option<i64>,
    #[serde(default)]
    pub amount_cents: Option<i64>,
    pub currency: String,
}

/// A minted proposal before it reaches the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MintedProposal {
    pub id: String,
    pub proposal: Proposal,
    pub evidence: Evidence,
    pub proposed_at: String,
}

impl MintedProposal {
    /// The row shape, with the class validated by the crate that owns the
    /// vocabulary. The column carries no CHECK precisely because this call does.
    pub fn to_row(&self) -> Result<StoredProposal, String> {
        if !content_item::valid(DATA_CLASS) {
            return Err(format!(
                "{DATA_CLASS} is not a class libs/content-item accepts"
            ));
        }
        Ok(StoredProposal {
            id: self.id.clone(),
            kind: self.proposal.kind.clone(),
            subject: self.proposal.subject.clone(),
            rung: self.proposal.rung.clone(),
            data_class: DATA_CLASS.into(),
            data_class_rationale: DATA_CLASS_RATIONALE.into(),
            proposal_json: serde_json::to_string(&self.proposal)
                .map_err(|error| error.to_string())?,
            evidence_json: serde_json::to_string(&self.evidence)
                .map_err(|error| error.to_string())?,
            model_revision: ENGINE_REVISION.into(),
            proposed_at: self.proposed_at.clone(),
        })
    }
}

/// The read model the HTTP layer serves. Status, verdict and outcome are derived
/// from the event rows; nothing here is a stored column.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecisionView {
    pub id: String,
    pub kind: String,
    pub subject: String,
    pub rung: String,
    pub data_class: String,
    pub data_class_rationale: String,
    pub model_revision: String,
    pub proposed_at: String,
    pub status: String,
    pub verdict: Option<String>,
    pub verdict_at: Option<String>,
    pub verdict_note: Option<String>,
    pub outcome_json: Option<serde_json::Value>,
    pub reviewed_at: Option<String>,
    pub proposal: serde_json::Value,
    pub evidence: serde_json::Value,
}

pub fn view(stored: &StoredDecision) -> DecisionView {
    let verdict = stored.latest_verdict();
    let outcome = stored.latest_outcome();
    DecisionView {
        id: stored.proposal.id.clone(),
        kind: stored.proposal.kind.clone(),
        subject: stored.proposal.subject.clone(),
        rung: stored.proposal.rung.clone(),
        data_class: stored.proposal.data_class.clone(),
        data_class_rationale: stored.proposal.data_class_rationale.clone(),
        model_revision: stored.proposal.model_revision.clone(),
        proposed_at: stored.proposal.proposed_at.clone(),
        status: stored.status().to_string(),
        verdict: verdict.and_then(|event| event.verdict.clone()),
        verdict_at: verdict.map(|event| event.recorded_at.clone()),
        verdict_note: verdict.map(|event| event.note.clone()),
        outcome_json: outcome
            .and_then(|event| event.outcome_json.as_deref())
            .and_then(|body| serde_json::from_str(body).ok()),
        reviewed_at: outcome.map(|event| event.recorded_at.clone()),
        proposal: serde_json::from_str(&stored.proposal.proposal_json)
            .unwrap_or(serde_json::Value::Null),
        evidence: serde_json::from_str(&stored.proposal.evidence_json)
            .unwrap_or(serde_json::Value::Null),
    }
}

// ---------------------------------------------------------------------------
// The rules
// ---------------------------------------------------------------------------

/// Everything the rules read. `report` is a `&PlanningReport` the caller already
/// produced through `planning::report`, which is public: this module never calls
/// planning, so `planning.rs` is not edited by this stream at all.
pub struct DecisionInputs<'a> {
    pub portfolio: &'a PortfolioReport,
    pub report: &'a PlanningReport,
    pub targets: Option<&'a TargetPolicy>,
    pub instruments: &'a [InstrumentProfile],
    pub subscriptions: &'a [crate::subscription::Subscription],
    pub feed_items: &'a [FeedEvidence],
    pub as_of: &'a str,
    pub proposed_at: &'a str,
    pub currency: &'a str,
    /// The risk figures, when a history existed to compute them from. Attached as
    /// evidence, never as a trigger: rung 2 explains, rung 1 proposes.
    pub risk: Option<&'a serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProposalRun {
    pub proposals: Vec<Proposal>,
    pub caveats: Vec<String>,
}

/// Mint every proposal the rules produce for one moment.
pub fn propose_all(inputs: &DecisionInputs<'_>) -> (Vec<MintedProposal>, Vec<String>) {
    let mut minted = Vec::new();
    let mut caveats = inputs.portfolio.caveats.clone();

    minted.extend(rebalance_proposals(inputs, &mut caveats));
    minted.extend(contribute_proposal(inputs, &mut caveats));
    minted.extend(review_proposals(inputs, &minted));

    (minted, caveats)
}

/// R1 -- an active allocation outside its band.
///
/// The drift arithmetic lives in `portfolio.rs`, which is the single definition
/// of what a share is; this rule only decides that a share outside its band is
/// worth a human's attention.
fn rebalance_proposals(
    inputs: &DecisionInputs<'_>,
    caveats: &mut Vec<String>,
) -> Vec<MintedProposal> {
    if !inputs.portfolio.targets_configured {
        return Vec::new();
    }
    if inputs.portfolio.coverage != "complete" || inputs.portfolio.unpriced_positions > 0 {
        caveats.push(
            "the portfolio is incomplete, so no rebalance is proposed: a share computed over part of a portfolio is a wrong number that looks right".into(),
        );
        return Vec::new();
    }
    let mut minted = Vec::new();
    for position in &inputs.portfolio.positions {
        let (Some(target_bp), Some(band_bp), Some(drift_bp)) =
            (position.target_bp, position.band_bp, position.drift_bp)
        else {
            continue;
        };
        if !position.outside_band {
            continue;
        }
        let subject = format!("instrument:{}", position.instrument);
        let direction = if drift_bp > 0 { "above" } else { "below" };
        let proposal = Proposal {
            kind: "rebalance".into(),
            subject: subject.clone(),
            title: format!("{} is {direction} its band", position.label),
            summary: format!(
                "{} holds {} bp against a target of {} bp with a band of {} bp, a drift of {} bp.",
                position.label, position.share_bp, target_bp, band_bp, drift_bp
            ),
            rung: "rule".into(),
            instrument: Some(position.instrument.clone()),
            asset_class: Some(position.asset_class.clone()),
            actual_bp: Some(position.share_bp),
            target_bp: Some(target_bp),
            band_bp: Some(band_bp),
            drift_bp: Some(drift_bp),
            amount_cents: None,
            currency: inputs.currency.to_string(),
        };
        let evidence = Evidence {
            numbers: BTreeMap::from([
                ("actual_bp".into(), position.share_bp),
                ("target_bp".into(), target_bp),
                ("band_bp".into(), band_bp),
                ("drift_bp".into(), drift_bp),
                (
                    "price_age_days".into(),
                    position.price_age_days.unwrap_or(-1),
                ),
            ]),
            feed_items: match_feed_items(inputs, &position.label),
            caveats: Vec::new(),
            risk: inputs.risk.cloned(),
        };
        minted.push(mint(
            "rebalance",
            &subject,
            &[drift_bp / DRIFT_BUCKET_BP, target_bp, band_bp],
            proposal,
            evidence,
            inputs.proposed_at,
        ));
    }

    // An asset-class target is drift too. Measured on a live fixture: with only
    // the per-position loop, a policy written as "60% equity, 40% bonds" showed
    // a 3,407 bp drift on /api/portfolio and produced no proposal at all, because
    // an asset-class target attaches to the class view and never to a position.
    // The two scopes are the ones `TargetAllocation.scope` declares, so both must
    // be able to mint.
    for class in &inputs.portfolio.asset_classes {
        let (Some(target_bp), Some(band_bp), Some(drift_bp)) =
            (class.target_bp, class.band_bp, class.drift_bp)
        else {
            continue;
        };
        if !class.outside_band {
            continue;
        }
        let subject = format!("asset_class:{}", class.asset_class);
        let direction = if drift_bp > 0 { "above" } else { "below" };
        let proposal = Proposal {
            kind: "rebalance".into(),
            subject: subject.clone(),
            title: format!("{} is {direction} its band", class.asset_class),
            summary: format!(
                "The {} allocation holds {} bp against a target of {target_bp} bp with a band of {band_bp} bp, a drift of {drift_bp} bp.",
                class.asset_class, class.share_bp
            ),
            rung: "rule".into(),
            instrument: None,
            asset_class: Some(class.asset_class.clone()),
            actual_bp: Some(class.share_bp),
            target_bp: Some(target_bp),
            band_bp: Some(band_bp),
            drift_bp: Some(drift_bp),
            amount_cents: None,
            currency: inputs.currency.to_string(),
        };
        let evidence = Evidence {
            numbers: BTreeMap::from([
                ("actual_bp".into(), class.share_bp),
                ("target_bp".into(), target_bp),
                ("band_bp".into(), band_bp),
                ("drift_bp".into(), drift_bp),
            ]),
            feed_items: Vec::new(),
            caveats: Vec::new(),
            risk: inputs.risk.cloned(),
        };
        minted.push(mint(
            "rebalance",
            &subject,
            &[drift_bp / DRIFT_BUCKET_BP, target_bp, band_bp],
            proposal,
            evidence,
            inputs.proposed_at,
        ));
    }
    minted
}

/// R2 -- the median monthly result is above the configured floor.
///
/// `baseline.monthly_result_cents` is the median monthly result `planning::report`
/// already computes. This rule does not recompute it and does not read
/// `planning.rs`'s private `baseline`, which is why nothing in that file changes.
fn contribute_proposal(
    inputs: &DecisionInputs<'_>,
    caveats: &mut Vec<String>,
) -> Vec<MintedProposal> {
    let Some(targets) = inputs.targets else {
        return Vec::new();
    };
    let surplus = inputs.report.baseline.monthly_result_cents;
    if surplus <= 0 {
        return Vec::new();
    }
    if surplus < targets.contribution_floor_cents {
        caveats.push(format!(
            "the median monthly result is below the configured contribution floor, so no contribution is proposed ({surplus} < {})",
            targets.contribution_floor_cents
        ));
        return Vec::new();
    }
    let Some(month) = clock::month_of(inputs.as_of) else {
        return Vec::new();
    };
    let subject = format!("month:{month}");
    let proposal = Proposal {
        kind: "contribute".into(),
        subject: subject.clone(),
        title: format!("A contribution for {month}"),
        summary: format!(
            "The median monthly result over {} month(s) is {} minor units, at or above the floor of {}.",
            inputs.report.baseline.months.len(),
            surplus,
            targets.contribution_floor_cents
        ),
        rung: "rule".into(),
        instrument: None,
        asset_class: None,
        actual_bp: None,
        target_bp: None,
        band_bp: None,
        drift_bp: None,
        amount_cents: Some(surplus),
        currency: inputs.currency.to_string(),
    };
    let evidence = Evidence {
        numbers: BTreeMap::from([
            ("median_monthly_result_cents".into(), surplus),
            (
                "contribution_floor_cents".into(),
                targets.contribution_floor_cents,
            ),
            (
                "months_observed".into(),
                inputs.report.baseline.months.len() as i64,
            ),
            (
                "monthly_income_cents".into(),
                inputs.report.baseline.monthly_income_cents,
            ),
            (
                "monthly_spending_cents".into(),
                inputs.report.baseline.monthly_spending_cents,
            ),
        ]),
        feed_items: Vec::new(),
        caveats: inputs.report.caveats.clone(),
        risk: inputs.risk.cloned(),
    };
    // Bucketed to whole currency units, so a one-cent move in the median does
    // not mint a second proposal.
    vec![mint(
        "contribute",
        &subject,
        &[surplus / 100],
        proposal,
        evidence,
        inputs.proposed_at,
    )]
}

/// R3 -- a lump-sum renewal falls in the month a contribution is proposed for.
///
/// The collision is the point, and it is arithmetic rather than a feeling: a
/// yearly or quarterly charge reaches the median monthly result as one twelfth
/// or one third of itself (`BillingCycle::monthly_cents`), while the cash leaves
/// the account whole in the month it is dated. A contribution sized from that
/// median is therefore sized against a month that has a lump in it, and the two
/// decisions arriving separately is how a month gets committed twice.
///
/// A monthly or weekly subscription is deliberately NOT a collision: it is in
/// every month equally, so the median already carries it at full weight.
fn review_proposals(
    inputs: &DecisionInputs<'_>,
    already_minted: &[MintedProposal],
) -> Vec<MintedProposal> {
    let Some(contribution) = already_minted
        .iter()
        .find(|minted| minted.proposal.kind == "contribute")
    else {
        return Vec::new();
    };
    let Some(month) = clock::month_of(inputs.as_of) else {
        return Vec::new();
    };
    // PRD Q103: `committed` below is a sum, and a sum needs one currency. Before this
    // ruling every `amount_cents` was added regardless of `price.currency` and the
    // total was labelled `inputs.currency`, which turns 45.00 USD into 45.00 EUR in a
    // proposal a person acts on. A renewal in another currency is named in a caveat
    // instead, because leaving it out silently is the same defect facing the other way.
    let mut renewals: Vec<(&str, i64)> = Vec::new();
    let mut foreign: Vec<String> = Vec::new();
    for subscription in inputs.subscriptions {
        if !subscription.state_at(inputs.as_of).is_billing() {
            continue;
        }
        let Some(price) = subscription.price_at(inputs.as_of) else {
            continue;
        };
        if !lump_renewal_lands_in(price, &month) {
            continue;
        }
        if price.currency.eq_ignore_ascii_case(inputs.currency) {
            renewals.push((subscription.name.as_str(), price.amount_cents));
        } else {
            foreign.push(format!(
                "{} renews in {month} and is priced in {}, which is not {}; it is not in the total",
                subscription.name, price.currency, inputs.currency
            ));
        }
    }
    if renewals.is_empty() {
        return Vec::new();
    }
    let committed: i64 = renewals.iter().map(|(_, amount)| amount).sum();
    let subject = format!("month:{month}");
    let proposal = Proposal {
        kind: "review".into(),
        subject: subject.clone(),
        title: format!("{} lump-sum renewal(s) fall in {month}", renewals.len()),
        summary: format!(
            "A contribution is proposed for {month} and {} renewal(s) worth {committed} minor units are dated in it, which the monthly median spreads across the year rather than into that month.",
            renewals.len()
        ),
        rung: "rule".into(),
        instrument: None,
        asset_class: None,
        actual_bp: None,
        target_bp: None,
        band_bp: None,
        drift_bp: None,
        amount_cents: Some(committed),
        currency: inputs.currency.to_string(),
    };
    let evidence = Evidence {
        numbers: BTreeMap::from([
            ("renewal_count".into(), renewals.len() as i64),
            ("committed_cents".into(), committed),
            (
                "proposed_contribution_cents".into(),
                contribution.proposal.amount_cents.unwrap_or(0),
            ),
        ]),
        feed_items: Vec::new(),
        caveats: foreign,
        risk: None,
    };
    vec![mint(
        "review",
        &subject,
        &[committed / 100, renewals.len() as i64],
        proposal,
        evidence,
        inputs.proposed_at,
    )]
}

/// Whether a lump-sum price point's anniversary falls in `month`.
///
/// Yearly renews in `valid_from`'s own month; quarterly every third month from
/// it. Monthly, weekly and one-off answer false: the first two are in every
/// month equally and the third has no anniversary at all.
fn lump_renewal_lands_in(price: &crate::subscription::PricePoint, month: &str) -> bool {
    let Some(start) = month_index(&price.valid_from) else {
        return false;
    };
    let Some(target) = month_index(&format!("{month}-01")) else {
        return false;
    };
    if target < start {
        return false;
    }
    match price.cycle {
        crate::subscription::BillingCycle::Yearly => (target - start) % 12 == 0,
        crate::subscription::BillingCycle::Quarterly => (target - start) % 3 == 0,
        _ => false,
    }
}

fn month_index(date: &str) -> Option<i64> {
    let year = date.get(..4)?.parse::<i64>().ok()?;
    let month = date.get(5..7)?.parse::<i64>().ok()?;
    (1..=12).contains(&month).then_some(year * 12 + month - 1)
}

/// `<kind>:<subject>:<sha256-16>` over the kind, the subject and the bucketed
/// numbers. A re-run that changes nothing material writes nothing; a material
/// change is a new id, so the old proposal is superseded rather than edited.
fn mint(
    kind: &str,
    subject: &str,
    buckets: &[i64],
    proposal: Proposal,
    evidence: Evidence,
    proposed_at: &str,
) -> MintedProposal {
    MintedProposal {
        id: proposal_id(kind, subject, buckets),
        proposal,
        evidence,
        proposed_at: proposed_at.to_string(),
    }
}

pub fn proposal_id(kind: &str, subject: &str, buckets: &[i64]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update([0]);
    hasher.update(subject.as_bytes());
    for bucket in buckets {
        hasher.update([0]);
        hasher.update(bucket.to_string().as_bytes());
    }
    let digest = hasher.finalize();
    let hex: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{kind}:{subject}:{hex}")
}

/// Feed items whose title mentions the instrument's configured label, on whole
/// tokens only.
///
/// The honest position, written here rather than only in a design: this is a
/// pointer to something worth reading, not a finding, and the UI must not present
/// it as a reason. The three-character floor and whole-token matching are what
/// bound the false positives; they do not remove them.
fn match_feed_items(inputs: &DecisionInputs<'_>, label: &str) -> Vec<FeedEvidence> {
    if label.chars().count() < MIN_LABEL_LENGTH {
        return Vec::new();
    }
    let needle = tokens(label);
    if needle.is_empty() {
        return Vec::new();
    }
    inputs
        .feed_items
        .iter()
        .filter(|item| contains_token_run(&tokens(&item.title), &needle))
        .take(MAX_FEED_ITEMS)
        .cloned()
        .collect()
}

/// One tokenizer for both sides. Two would drift, and the drift would show up as
/// evidence that silently stops matching.
fn tokens(value: &str) -> Vec<String> {
    value
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whole tokens, in order, and never a substring: a three-letter symbol must not
/// match inside a longer word.
fn contains_token_run(haystack: &[String], needle: &[String]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

/// Assemble every input the rules read and mint the proposals.
///
/// One assembly point for the HTTP handler and the CLI. Two would be two
/// definitions of what the rules see, which is the drift this whole stream keeps
/// removing rather than adding.
///
/// `feed_items` is passed in rather than fetched here: the loopback call is I/O
/// the caller owns, and a rule engine that opens sockets is a rule engine no test
/// can run offline.
pub fn recompute(
    store: &FinanceStore,
    config: &crate::config::Config,
    feed_items: &[FeedEvidence],
    as_of: &str,
    proposed_at: &str,
    currency: &str,
) -> Result<(Vec<MintedProposal>, Vec<String>), String> {
    let snapshot = store
        .holding_projection()
        .map_err(|error| error.to_string())?
        .ok_or("no reviewed holdings snapshot is in the projection; import one first")?;
    let latest_prices = store.latest_prices().map_err(|error| error.to_string())?;
    let rows = store
        .transaction_projection()
        .map_err(|error| error.to_string())?;
    let subscriptions = store.list().map_err(|error| error.to_string())?;
    let balance_snapshot = config
        .balance_snapshot
        .as_deref()
        .map(crate::balance::read_snapshot)
        .transpose()?
        .flatten();
    let portfolio = crate::portfolio::report(crate::portfolio::PortfolioInputs {
        snapshot: &snapshot,
        latest_prices: &latest_prices,
        instruments: &config.instruments,
        targets: config.targets.as_ref(),
        as_of,
        currency,
    })?;
    let portfolio_values =
        crate::investment::portfolio_valuations(&snapshot).map_err(|error| error.to_string())?;
    let planning = crate::planning::report(crate::planning::PlanningInputs {
        rows: &rows,
        commitments: &config.commitments,
        subscriptions: &subscriptions,
        balance_snapshot: balance_snapshot.as_ref(),
        investment_snapshot: Some(&snapshot),
        portfolio_values: &portfolio_values,
        config: &config.planning,
        as_of,
        currency,
    });
    // Rung 2, attached as evidence and never as a trigger. It returns `null`
    // figures until a history exists, and that is the correct answer rather than
    // a reason to withhold the proposal.
    let risk = crate::risk::report(store, &snapshot, config.targets.as_ref())
        .ok()
        .and_then(|report| serde_json::to_value(report).ok());
    let inputs = DecisionInputs {
        portfolio: &portfolio,
        report: &planning,
        targets: config.targets.as_ref(),
        instruments: &config.instruments,
        subscriptions: &subscriptions,
        feed_items,
        as_of,
        proposed_at,
        currency,
        risk: risk.as_ref(),
    };
    Ok(propose_all(&inputs))
}

/// Reconcile a run against the ledger and re-render the months it touched.
///
/// `overlay` is the overlay root; `None` skips the export, which is what a test
/// with no overlay does. A failure to write the copy is reported, never
/// swallowed: the copy is the point.
pub fn run(
    store: &FinanceStore,
    minted: &[MintedProposal],
    recorded_at: &str,
    overlay: Option<&std::path::Path>,
) -> Result<DecisionRunOutcome, String> {
    let rows: Vec<StoredProposal> = minted
        .iter()
        .map(MintedProposal::to_row)
        .collect::<Result<_, _>>()?;
    let outcome = store
        .reconcile_decisions(&rows, recorded_at)
        .map_err(|error| error.to_string())?;
    if let Some(overlay) = overlay {
        export_all(store, overlay)?;
    }
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// The human-readable copy
// ---------------------------------------------------------------------------

/// Re-render every month that has a row, whole-file.
pub fn export_all(store: &FinanceStore, overlay: &std::path::Path) -> Result<Vec<String>, String> {
    let decisions = store.decisions(None).map_err(|error| error.to_string())?;
    let mut months: BTreeMap<String, Vec<DecisionView>> = BTreeMap::new();
    for stored in &decisions {
        let view = view(stored);
        let Some(month) = clock::month_of(&view.proposed_at[..view.proposed_at.len().min(10)])
        else {
            continue;
        };
        months.entry(month).or_default().push(view);
    }
    let mut written = Vec::new();
    for (month, views) in months {
        written.push(write_month(overlay, &month, &views)?);
    }
    Ok(written)
}

/// Re-render one month.
pub fn export_month(
    store: &FinanceStore,
    overlay: &std::path::Path,
    month: &str,
) -> Result<String, String> {
    let decisions = store.decisions(None).map_err(|error| error.to_string())?;
    let views: Vec<DecisionView> = decisions
        .iter()
        .map(view)
        .filter(|view| view.proposed_at.starts_with(month))
        .collect();
    write_month(overlay, month, &views)
}

/// Whole-file, temp plus rename, mode 0600 -- the same write `investment.rs` uses
/// for the reviewed snapshot. A crash cannot truncate the copy that exists
/// because a crash is when the copy matters.
fn write_month(
    overlay: &std::path::Path,
    month: &str,
    views: &[DecisionView],
) -> Result<String, String> {
    let directory = overlay.join(EXPORT_DIR);
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join(format!("{month}.md"));
    let body = render_month(month, views);
    let temporary = directory.join(format!(".{month}.md.tmp"));
    {
        let mut file = std::fs::File::create(&temporary).map_err(|error| error.to_string())?;
        file.write_all(body.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    set_owner_only(&temporary)?;
    std::fs::rename(&temporary, &path).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

#[cfg(unix)]
fn set_owner_only(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn set_owner_only(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

/// Deterministic over the month's rows, so re-rendering is idempotent and a
/// diff between two runs shows only what changed.
pub fn render_month(month: &str, views: &[DecisionView]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Investment decisions — {month}\n\n"));
    out.push_str(
        "Written by the finance capability on every verdict and every proposal run. \
         The database is a rebuildable index; this file is the copy that survives it. \
         Accepting a proposal records a decision and moves no money.\n\n",
    );
    if views.is_empty() {
        out.push_str("No proposals were recorded in this month.\n");
        return out;
    }
    let mut sorted: Vec<&DecisionView> = views.iter().collect();
    sorted.sort_by(|left, right| {
        left.proposed_at
            .cmp(&right.proposed_at)
            .then(left.id.cmp(&right.id))
    });
    out.push_str("| Proposed | Kind | Subject | Rung | Status | Verdict at | Note |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
    for view in &sorted {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            view.proposed_at,
            view.kind,
            escape_cell(&view.subject),
            view.rung,
            view.status,
            view.verdict_at.as_deref().unwrap_or("—"),
            escape_cell(view.verdict_note.as_deref().unwrap_or("")),
        ));
    }
    out.push('\n');
    for view in &sorted {
        out.push_str(&format!("## {}\n\n", view.id));
        if let Some(title) = view.proposal.get("title").and_then(|v| v.as_str()) {
            out.push_str(&format!("**{title}**\n\n"));
        }
        if let Some(summary) = view.proposal.get("summary").and_then(|v| v.as_str()) {
            out.push_str(&format!("{summary}\n\n"));
        }
        out.push_str(&format!(
            "- status: {}\n- class: {} — {}\n- engine: {}\n",
            view.status, view.data_class, view.data_class_rationale, view.model_revision
        ));
        if let Some(verdict) = &view.verdict {
            out.push_str(&format!(
                "- verdict: **{verdict}** at {}\n",
                view.verdict_at.as_deref().unwrap_or("—")
            ));
            let note = view.verdict_note.as_deref().unwrap_or("");
            if !note.is_empty() {
                out.push_str(&format!("- note: {note}\n"));
            }
        }
        if let Some(numbers) = view.evidence.get("numbers").and_then(|v| v.as_object()) {
            out.push_str("- numbers:\n");
            for (name, value) in numbers {
                out.push_str(&format!("  - {name}: {value}\n"));
            }
        }
        if let Some(items) = view.evidence.get("feed_items").and_then(|v| v.as_array()) {
            if !items.is_empty() {
                out.push_str("- reading (a pointer, not a reason):\n");
                for item in items {
                    let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    out.push_str(&format!("  - [{}]({})\n", escape_cell(title), url));
                }
            }
        }
        out.push('\n');
    }
    out
}

fn escape_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planning::{MonthlyBaseline, SubscriptionPortfolio};

    fn baseline(result_cents: i64) -> MonthlyBaseline {
        MonthlyBaseline {
            months: vec!["2026-07".into(), "2026-08".into()],
            monthly_income_cents: 400_000,
            monthly_spending_cents: 400_000 - result_cents,
            forecast_base_cents: 300_000,
            monthly_result_cents: result_cents,
            savings_rate_percent: None,
            behavior: Vec::new(),
            classified_value_percent: None,
        }
    }

    fn planning_report(result_cents: i64) -> PlanningReport {
        PlanningReport {
            as_of: "2026-09-05".into(),
            currency: "EUR".into(),
            baseline: baseline(result_cents),
            forecasts: Vec::new(),
            liquidity: None,
            subscriptions: SubscriptionPortfolio {
                monthly_cents: 0,
                annual_cents: 0,
                billing_count: 0,
                covered_count: 0,
                unknown_price_count: 0,
                anomalies: Vec::new(),
            },
            card_decision: None,
            loyalty: Vec::new(),
            caveats: Vec::new(),
        }
    }

    fn portfolio(
        targets_configured: bool,
        positions: Vec<crate::portfolio::PositionView>,
    ) -> PortfolioReport {
        PortfolioReport {
            as_of: "2026-09-05".into(),
            currency: "EUR".into(),
            coverage: "complete".into(),
            targets_configured,
            total: crate::investment::from_minor_units(1_000_000),
            priced_positions: positions.len(),
            unpriced_positions: 0,
            positions,
            asset_classes: Vec::new(),
            caveats: Vec::new(),
        }
    }

    fn position(
        instrument: &str,
        share_bp: i64,
        target_bp: i64,
        band_bp: i64,
    ) -> crate::portfolio::PositionView {
        let drift = share_bp - target_bp;
        crate::portfolio::PositionView {
            instrument: instrument.into(),
            label: format!("Synthetic {instrument}"),
            asset_class: "equity".into(),
            quantity: crate::investment::Quantity {
                mantissa: 100,
                scale: 0,
            },
            currency: "EUR".into(),
            review_price: None,
            market_price: None,
            market_price_source: None,
            market_price_observed_on: None,
            price_age_days: None,
            price_freshness: "none".into(),
            value: crate::investment::from_minor_units(500_000),
            value_basis: "review".into(),
            share_bp,
            target_bp: Some(target_bp),
            band_bp: Some(band_bp),
            drift_bp: Some(drift),
            outside_band: drift.abs() > band_bp,
            change_since_review: None,
            change_since_review_bp: None,
        }
    }

    fn inputs<'a>(
        portfolio: &'a PortfolioReport,
        report: &'a PlanningReport,
        targets: Option<&'a TargetPolicy>,
        feed: &'a [FeedEvidence],
    ) -> DecisionInputs<'a> {
        DecisionInputs {
            portfolio,
            report,
            targets,
            instruments: &[],
            subscriptions: &[],
            feed_items: feed,
            as_of: "2026-09-05",
            proposed_at: "2026-09-05T08:00:00Z",
            currency: "EUR",
            risk: None,
        }
    }

    #[test]
    fn drift_inside_the_band_proposes_nothing_and_outside_it_names_both_numbers() {
        let inside = portfolio(true, vec![position("SYN-A", 5_100, 5_000, 200)]);
        let report = planning_report(0);
        let (minted, _) = propose_all(&inputs(&inside, &report, None, &[]));
        assert!(
            minted.is_empty(),
            "a share inside its band is not a decision"
        );

        let outside = portfolio(true, vec![position("SYN-A", 6_000, 5_000, 200)]);
        let (minted, _) = propose_all(&inputs(&outside, &report, None, &[]));
        assert_eq!(minted.len(), 1);
        assert_eq!(minted[0].proposal.kind, "rebalance");
        assert_eq!(minted[0].proposal.actual_bp, Some(6_000));
        assert_eq!(minted[0].proposal.target_bp, Some(5_000));
        assert_eq!(minted[0].proposal.band_bp, Some(200));
        assert_eq!(minted[0].proposal.drift_bp, Some(1_000));
    }

    /// Measured on a live fixture: with only the per-position loop, an
    /// asset-class policy showed a four-figure drift on /api/portfolio and minted
    /// nothing at all.
    #[test]
    fn an_asset_class_target_outside_its_band_mints_a_rebalance_too() {
        let mut books = portfolio(true, vec![position("SYN-A", 9_407, 0, 0)]);
        books.positions[0].target_bp = None;
        books.positions[0].band_bp = None;
        books.positions[0].drift_bp = None;
        books.positions[0].outside_band = false;
        books.asset_classes = vec![crate::portfolio::AssetClassView {
            asset_class: "equity".into(),
            value: crate::investment::from_minor_units(940_700),
            share_bp: 9_407,
            target_bp: Some(6_000),
            band_bp: Some(300),
            drift_bp: Some(3_407),
            outside_band: true,
        }];
        let report = planning_report(0);
        let (minted, _) = propose_all(&inputs(&books, &report, None, &[]));
        assert_eq!(minted.len(), 1);
        assert_eq!(minted[0].proposal.kind, "rebalance");
        assert_eq!(minted[0].proposal.subject, "asset_class:equity");
        assert_eq!(minted[0].proposal.actual_bp, Some(9_407));
        assert_eq!(minted[0].proposal.target_bp, Some(6_000));
        assert_eq!(minted[0].proposal.drift_bp, Some(3_407));
    }

    #[test]
    fn a_partial_portfolio_proposes_no_rebalance() {
        let mut incomplete = portfolio(true, vec![position("SYN-A", 6_000, 5_000, 200)]);
        incomplete.coverage = "partial".into();
        let report = planning_report(0);
        let (minted, caveats) = propose_all(&inputs(&incomplete, &report, None, &[]));
        assert!(minted.is_empty());
        assert!(caveats.iter().any(|caveat| caveat.contains("incomplete")));
    }

    #[test]
    fn a_proposal_id_is_stable_across_a_sub_bucket_move() {
        let report = planning_report(0);
        let a = portfolio(true, vec![position("SYN-A", 6_000, 5_000, 200)]);
        let b = portfolio(true, vec![position("SYN-A", 6_003, 5_000, 200)]);
        let c = portfolio(true, vec![position("SYN-A", 6_040, 5_000, 200)]);
        let id = |p: &PortfolioReport| propose_all(&inputs(p, &report, None, &[])).0[0].id.clone();
        assert_eq!(id(&a), id(&b), "a 3 bp move is inside the bucket");
        assert_ne!(id(&a), id(&c), "a 40 bp move is a new proposal");
    }

    #[test]
    fn every_proposal_names_its_rung_and_none_is_model() {
        let targets = TargetPolicy {
            contribution_floor_cents: 10_000,
            ..TargetPolicy::default()
        };
        let report = planning_report(50_000);
        let books = portfolio(true, vec![position("SYN-A", 6_000, 5_000, 200)]);
        let (minted, _) = propose_all(&inputs(&books, &report, Some(&targets), &[]));
        assert!(minted.len() >= 2);
        for proposal in &minted {
            assert_eq!(proposal.proposal.rung, "rule");
        }
        // The line the deferred model must consciously cross.
        assert!(minted.iter().all(|p| p.proposal.rung != "model"));
    }

    #[test]
    fn no_rule_mints_a_sell() {
        let targets = TargetPolicy {
            contribution_floor_cents: 0,
            ..TargetPolicy::default()
        };
        let report = planning_report(50_000);
        let books = portfolio(true, vec![position("SYN-A", 9_000, 5_000, 200)]);
        let (minted, _) = propose_all(&inputs(&books, &report, Some(&targets), &[]));
        assert!(minted.iter().all(|p| p.proposal.kind != "sell"));
    }

    #[test]
    fn every_decision_row_carries_a_class_the_shared_crate_accepts() {
        let report = planning_report(0);
        let books = portfolio(true, vec![position("SYN-A", 6_000, 5_000, 200)]);
        let (minted, _) = propose_all(&inputs(&books, &report, None, &[]));
        for proposal in &minted {
            let row = proposal.to_row().expect("a row");
            assert_eq!(row.data_class, "c1");
            assert!(content_item::valid(&row.data_class));
        }
    }

    #[test]
    fn a_contribution_below_the_floor_proposes_nothing_and_says_why() {
        let targets = TargetPolicy {
            contribution_floor_cents: 25_000,
            ..TargetPolicy::default()
        };
        let report = planning_report(10_000);
        let books = portfolio(false, Vec::new());
        let (minted, caveats) = propose_all(&inputs(&books, &report, Some(&targets), &[]));
        assert!(minted.is_empty());
        assert!(caveats.iter().any(|caveat| caveat.contains("floor")));
    }

    #[test]
    fn feed_evidence_matches_whole_tokens_only() {
        let items = vec![
            FeedEvidence {
                id: "a".into(),
                title: "Synthetic SYN-A posts results".into(),
                url: "https://example.invalid/a".into(),
                day: "2026-09-04".into(),
            },
            FeedEvidence {
                id: "b".into(),
                title: "Unsynthetic filings".into(),
                url: "https://example.invalid/b".into(),
                day: "2026-09-04".into(),
            },
        ];
        let report = planning_report(0);
        let books = portfolio(true, vec![position("SYN-A", 6_000, 5_000, 200)]);
        let (minted, _) = propose_all(&inputs(&books, &report, None, &items));
        let evidence = &minted[0].evidence.feed_items;
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].id, "a");
        // Only the four public fields travel; nothing here can carry body text.
        let encoded = serde_json::to_string(&evidence[0]).unwrap();
        assert!(encoded.contains("\"title\""));
        assert!(!encoded.contains("summary"));
    }

    #[test]
    fn a_label_under_three_characters_is_never_used_as_a_matcher() {
        let items = vec![FeedEvidence {
            id: "a".into(),
            title: "ab cd".into(),
            url: "https://example.invalid/a".into(),
            day: "2026-09-04".into(),
        }];
        let report = planning_report(0);
        let mut books = portfolio(true, vec![position("SYN-A", 6_000, 5_000, 200)]);
        books.positions[0].label = "ab".into();
        let (minted, _) = propose_all(&inputs(&books, &report, None, &items));
        assert!(minted[0].evidence.feed_items.is_empty());
    }

    #[test]
    fn a_rendered_month_is_byte_identical_on_a_re_render() {
        let view = DecisionView {
            id: "rebalance:instrument:SYN-A:00112233".into(),
            kind: "rebalance".into(),
            subject: "instrument:SYN-A".into(),
            rung: "rule".into(),
            data_class: "c1".into(),
            data_class_rationale: DATA_CLASS_RATIONALE.into(),
            model_revision: ENGINE_REVISION.into(),
            proposed_at: "2026-09-05T08:00:00Z".into(),
            status: "accepted".into(),
            verdict: Some("accepted".into()),
            verdict_at: Some("2026-09-05T09:00:00Z".into()),
            verdict_note: Some("Synthetic note".into()),
            outcome_json: None,
            reviewed_at: None,
            proposal: serde_json::json!({ "title": "T", "summary": "S" }),
            evidence: serde_json::json!({ "numbers": { "drift_bp": 1000 } }),
        };
        let first = render_month("2026-09", std::slice::from_ref(&view));
        let second = render_month("2026-09", std::slice::from_ref(&view));
        assert_eq!(first, second);
        assert!(first.contains("accepted"));
        assert!(first.contains("Synthetic note"));
        assert!(first.contains("moves no money"));
    }
}
