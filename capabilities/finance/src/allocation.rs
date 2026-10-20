//! Reviewed spending context and shared-cost accounting.
//!
//! Category answers what was bought. Purpose answers why, and the receivable
//! posting answers whose money it was. Keeping those independent means a meal on
//! a trip remains food without counting the part fronted for friends as personal
//! consumption.

use crate::import::{self, CandidateState, ImportError, ImportResult, TransactionCandidate};
use serde::{Deserialize, Serialize};

pub const SHARED_RECEIVABLE_ACCOUNT: &str = "assets:receivable:shared";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpendingPurpose {
    DayToDay,
    Trip,
    Work,
    Housing,
    Other,
}

impl SpendingPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DayToDay => "day_to_day",
            Self::Trip => "trip",
            Self::Work => "work",
            Self::Housing => "housing",
            Self::Other => "other",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "day_to_day" => Some(Self::DayToDay),
            "trip" => Some(Self::Trip),
            "work" => Some(Self::Work),
            "housing" => Some(Self::Housing),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpenseAllocation {
    pub personal_cents: i64,
    pub purpose: SpendingPurpose,
    #[serde(default)]
    pub trip_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReimbursementLink {
    pub expense_candidate_id: String,
}

pub fn rewrite_expense(
    journal: &str,
    candidate: &TransactionCandidate,
    allocation: &ExpenseAllocation,
) -> ImportResult<(String, bool)> {
    validate_expense(candidate, allocation)?;
    let replacement = render_expense(candidate, allocation)?;
    rewrite_owned_entry(journal, candidate, &replacement, |block| {
        let current = parse_expense(block)?;
        render_expense(candidate, &current)
    })
}

pub fn rewrite_reimbursement(
    journal: &str,
    candidate: &TransactionCandidate,
    expense_source_id: &str,
) -> ImportResult<(String, bool)> {
    validate_reimbursement(candidate, expense_source_id)?;
    let replacement = render_reimbursement(candidate, expense_source_id)?;
    rewrite_owned_entry(journal, candidate, &replacement, |block| {
        let current = tag(block, "sjel-reimbursement-for")
            .ok_or_else(|| ImportError("journal entry is not an Sjel reimbursement".into()))?;
        render_reimbursement(candidate, current)
    })
}

fn validate_expense(
    candidate: &TransactionCandidate,
    allocation: &ExpenseAllocation,
) -> ImportResult<()> {
    if candidate.state != CandidateState::Confirmed || candidate.amount_cents >= 0 {
        return Err(ImportError(
            "only a confirmed outflow can receive spending context".into(),
        ));
    }
    if !candidate.proposed_account.starts_with("expenses:") {
        return Err(ImportError(
            "a shared-cost allocation requires a reviewed expense account".into(),
        ));
    }
    import::validate_account(&candidate.source_account)?;
    import::validate_account(&candidate.proposed_account)?;
    let total = candidate
        .amount_cents
        .checked_abs()
        .ok_or_else(|| ImportError("amount is outside the supported range".into()))?;
    if !(0..=total).contains(&allocation.personal_cents) {
        return Err(ImportError(
            "personal share must be between zero and the full outflow".into(),
        ));
    }
    match (allocation.purpose, allocation.trip_id.as_deref()) {
        (SpendingPurpose::Trip, Some(id)) => validate_reference(id)?,
        (SpendingPurpose::Trip, None) => {
            return Err(ImportError("trip purpose requires a Trips plan id".into()))
        }
        (_, Some(_)) => {
            return Err(ImportError(
                "a Trips plan id is only valid for trip purpose".into(),
            ))
        }
        (_, None) => {}
    }
    Ok(())
}

fn validate_reimbursement(
    candidate: &TransactionCandidate,
    expense_source_id: &str,
) -> ImportResult<()> {
    if candidate.state != CandidateState::Confirmed || candidate.amount_cents <= 0 {
        return Err(ImportError(
            "only a confirmed inflow can be linked as a reimbursement".into(),
        ));
    }
    import::validate_account(&candidate.source_account)?;
    validate_reference(expense_source_id)
}

fn validate_reference(value: &str) -> ImportResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(ImportError(
            "context references must be bounded symbolic identifiers".into(),
        ))
    }
}

fn render_expense(
    candidate: &TransactionCandidate,
    allocation: &ExpenseAllocation,
) -> ImportResult<String> {
    validate_expense(candidate, allocation)?;
    let total = candidate.amount_cents.abs();
    let shared = total - allocation.personal_cents;
    let mut tags = vec![
        format!("sjel-purpose: {}", allocation.purpose.as_str()),
        format!("sjel-own-cents: {}", allocation.personal_cents),
        format!("sjel-shared-cents: {shared}"),
    ];
    if let Some(trip_id) = &allocation.trip_id {
        tags.push(format!("sjel-trip-id: {trip_id}"));
    }
    let mut entry = format!(
        "\n{} * {}  ; {}\n    {}  {} {}\n",
        candidate.booked_at,
        import::sanitize_description(&candidate.description),
        tags.join(", "),
        candidate.source_account,
        import::decimal(candidate.amount_cents),
        candidate.currency,
    );
    if allocation.personal_cents > 0 {
        entry.push_str(&format!(
            "    {}  {} {}\n",
            candidate.proposed_account,
            import::decimal(allocation.personal_cents),
            candidate.currency,
        ));
    }
    if shared > 0 {
        entry.push_str(&format!(
            "    {SHARED_RECEIVABLE_ACCOUNT}  {} {}\n",
            import::decimal(shared),
            candidate.currency,
        ));
    }
    entry.push_str(&format!("    ; source-id: {}\n", candidate.fingerprint));
    Ok(entry)
}

fn render_reimbursement(
    candidate: &TransactionCandidate,
    expense_source_id: &str,
) -> ImportResult<String> {
    validate_reimbursement(candidate, expense_source_id)?;
    Ok(format!(
        "\n{} * {}  ; sjel-reimbursement-for: {}\n    {}  {} {}\n    {}  {} {}\n    ; source-id: {}\n",
        candidate.booked_at,
        import::sanitize_description(&candidate.description),
        expense_source_id,
        candidate.source_account,
        import::decimal(candidate.amount_cents),
        candidate.currency,
        SHARED_RECEIVABLE_ACCOUNT,
        import::decimal(-candidate.amount_cents),
        candidate.currency,
        candidate.fingerprint,
    ))
}

fn rewrite_owned_entry<F>(
    journal: &str,
    candidate: &TransactionCandidate,
    replacement: &str,
    render_current: F,
) -> ImportResult<(String, bool)>
where
    F: FnOnce(&str) -> ImportResult<String>,
{
    let marker = format!("source-id: {}", candidate.fingerprint);
    if journal.matches(&marker).count() != 1 {
        return Err(ImportError(
            "confirmed source marker must occur exactly once in the journal".into(),
        ));
    }
    if journal.matches(replacement).count() == 1 {
        return Ok((journal.to_string(), false));
    }
    let base = import::render_journal_entry(candidate, &candidate.proposed_account)?;
    if journal.matches(&base).count() == 1 {
        return Ok((journal.replacen(&base, replacement, 1), true));
    }
    let block = owned_block(journal, candidate)?;
    if render_current(block)? != block {
        return Err(ImportError(
            "Sjel-owned journal entry drifted from its recorded allocation".into(),
        ));
    }
    Ok((journal.replacen(block, replacement, 1), true))
}

fn owned_block<'a>(journal: &'a str, candidate: &TransactionCandidate) -> ImportResult<&'a str> {
    let marker = format!("source-id: {}", candidate.fingerprint);
    let marker_at = journal
        .find(&marker)
        .ok_or_else(|| ImportError("confirmed source marker is absent".into()))?;
    let header = format!(
        "\n{} * {}",
        candidate.booked_at,
        import::sanitize_description(&candidate.description)
    );
    let start = journal[..marker_at]
        .rfind(&header)
        .ok_or_else(|| ImportError("Sjel-owned journal entry header is absent".into()))?;
    let marker_end = marker_at + marker.len();
    let end = journal[marker_end..]
        .find("\n\n")
        .map_or(journal.len(), |offset| marker_end + offset + 1);
    Ok(&journal[start..end])
}

fn parse_expense(block: &str) -> ImportResult<ExpenseAllocation> {
    let purpose = tag(block, "sjel-purpose")
        .and_then(SpendingPurpose::parse)
        .ok_or_else(|| ImportError("journal entry has no valid Sjel purpose".into()))?;
    let personal_cents = tag(block, "sjel-own-cents")
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ImportError("journal entry has no valid personal share".into()))?;
    let trip_id = tag(block, "sjel-trip-id").map(str::to_string);
    Ok(ExpenseAllocation {
        personal_cents,
        purpose,
        trip_id,
    })
}

fn tag<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let header = block.lines().find(|line| line.contains(" * "))?;
    let comment = header.split_once(';')?.1;
    comment.split(',').find_map(|part| {
        let (key, value) = part.trim().split_once(':')?;
        (key.trim() == name).then_some(value.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(amount_cents: i64) -> TransactionCandidate {
        TransactionCandidate {
            id: "candidate_synthetic".into(),
            fingerprint: "synthetic-source".into(),
            booked_at: "2026-01-02".into(),
            description: "Synthetic group meal".into(),
            amount_cents,
            currency: "EUR".into(),
            source_account: "assets:bank:checking".into(),
            source_reference: None,
            proposed_account: if amount_cents < 0 {
                "expenses:food:dining-out".into()
            } else {
                "income:uncategorized".into()
            },
            confidence_basis_points: 0,
            state: CandidateState::Confirmed,
            location_street: None,
            location_postal_code: None,
            location_city: None,
            location_country: None,
            location_mcc: None,
        }
    }

    #[test]
    fn a_shared_trip_expense_records_only_the_reviewed_personal_share_as_expense() {
        let candidate = candidate(-40_00);
        let base = import::render_journal_entry(&candidate, &candidate.proposed_account).unwrap();
        let allocation = ExpenseAllocation {
            personal_cents: 10_00,
            purpose: SpendingPurpose::Trip,
            trip_id: Some("trip:plan:synthetic".into()),
        };

        let (journal, changed) = rewrite_expense(&base, &candidate, &allocation).unwrap();
        assert!(changed);
        assert!(journal.contains("expenses:food:dining-out  10.00 EUR"));
        assert!(journal.contains("assets:receivable:shared  30.00 EUR"));
        assert!(journal.contains("sjel-purpose: trip"));
        assert!(journal.contains("sjel-trip-id: trip:plan:synthetic"));

        let (same, changed) = rewrite_expense(&journal, &candidate, &allocation).unwrap();
        assert!(!changed);
        assert_eq!(same, journal);
    }

    #[test]
    fn a_reviewed_allocation_can_be_corrected_without_accepting_manual_drift() {
        let candidate = candidate(-40_00);
        let base = import::render_journal_entry(&candidate, &candidate.proposed_account).unwrap();
        let first = ExpenseAllocation {
            personal_cents: 10_00,
            purpose: SpendingPurpose::Trip,
            trip_id: Some("trip:plan:synthetic".into()),
        };
        let corrected = ExpenseAllocation {
            personal_cents: 12_00,
            purpose: SpendingPurpose::Trip,
            trip_id: Some("trip:plan:synthetic".into()),
        };
        let (journal, _) = rewrite_expense(&base, &candidate, &first).unwrap();
        let (journal, changed) = rewrite_expense(&journal, &candidate, &corrected).unwrap();
        assert!(changed);
        assert!(journal.contains("expenses:food:dining-out  12.00 EUR"));
        assert!(rewrite_expense(
            &journal.replace("12.00 EUR", "13.00 EUR"),
            &candidate,
            &first
        )
        .is_err());
    }

    #[test]
    fn trip_context_requires_an_opaque_plan_reference() {
        let candidate = candidate(-40_00);
        let allocation = ExpenseAllocation {
            personal_cents: 10_00,
            purpose: SpendingPurpose::Trip,
            trip_id: None,
        };
        assert!(validate_expense(&candidate, &allocation).is_err());
    }

    #[test]
    fn reimbursement_is_a_receivable_transfer_not_income() {
        let candidate = candidate(30_00);
        let base = import::render_journal_entry(&candidate, &candidate.proposed_account).unwrap();
        let (journal, changed) =
            rewrite_reimbursement(&base, &candidate, "synthetic-expense").unwrap();
        assert!(changed);
        assert!(journal.contains("sjel-reimbursement-for: synthetic-expense"));
        assert!(journal.contains("assets:receivable:shared  -30.00 EUR"));
        assert!(!journal.contains("income:uncategorized"));
    }
}
