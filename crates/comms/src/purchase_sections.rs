//! Strict purchase-section parsing; unsupported layouts abstain as a whole.

use crate::mail_evidence::{self, EvidenceError, Parsed, Proposal, Stage};

/// The deterministic adapter revision retained with each proposal.
pub const REVISION: &str = "purchase-sections-v1";

fn price(line: &str) -> bool {
    let amount = line.strip_prefix(['€', '$', '£']);
    amount.is_some_and(|s| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '.' | ','))
            && s.chars().any(|c| c.is_ascii_digit())
    })
}

/// Extract a complete supported order/shipment section, otherwise abstain.
///
/// # Errors
/// Refuses disallowed classification, oversized input and invalid source identity.
pub fn extract(source: &str, class: &str, mail: &str) -> Result<Option<Parsed>, EvidenceError> {
    mail_evidence::parse(Stage::Inventory, source, class, mail, &[], "[]")?;
    let lines: Vec<_> = mail.lines().map(str::trim).collect();
    let sections = [
        (
            "Bestellübersicht",
            "Vielen Dank für deinen Einkauf!",
            "Zwischensumme",
            "order",
        ),
        (
            "Order summary",
            "Thank you for your purchase!",
            "Subtotal",
            "order",
        ),
        (
            "Artikel in dieser Lieferung",
            "Deine Bestellung ist auf dem Weg",
            "Wenn du Fragen",
            "shipment",
        ),
        (
            "Items in this shipment",
            "Your order is on the way",
            "If you have any questions",
            "shipment",
        ),
    ];
    let frames: Vec<_> = sections
        .iter()
        .filter(|(heading, statement, _, _)| lines.contains(heading) && lines.contains(statement))
        .collect();
    if frames.len() != 1 {
        return Ok(None);
    }
    let (heading, statement, end, event) = *frames[0];
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == heading)
        .map(|(i, _)| i)
        .collect();
    if starts.len() != 1 {
        return Ok(None);
    }
    let start = starts[0];
    if !lines[..start].contains(&statement) {
        return Ok(None);
    }
    let mut rows = Vec::new();
    let mut ended = false;
    for line in &lines[start + 1..] {
        if line.starts_with(end) {
            ended = true;
            break;
        }
        if line.is_empty() || line.chars().all(|c| c == '-') || price(line) {
            continue;
        }
        let Some((title, quantity)) = line.rsplit_once(" × ") else {
            return Ok(None);
        };
        if title.is_empty()
            || quantity
                .parse::<u32>()
                .ok()
                .is_none_or(|n| n == 0 || n > 999)
        {
            return Ok(None);
        }
        rows.push(serde_json::json!({"event":event, "title":title, "evidence":line}));
        if rows.len() > mail_evidence::MAX_PROPOSALS {
            return Ok(None);
        }
    }
    if !ended || rows.is_empty() {
        return Ok(None);
    }
    let json = serde_json::to_string(&rows).map_err(|_| EvidenceError::Response)?;
    let mut parsed = mail_evidence::parse(Stage::Inventory, source, class, mail, &[], &json)?;
    if !parsed.dropped.is_empty() || parsed.duplicates != 0 {
        return Ok(None);
    }
    for proposal in &mut parsed.proposals {
        if let Proposal::Inventory(p) = proposal {
            p.extraction_version = Some(REVISION.into());
        }
    }
    Ok(Some(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER: &str = "Vielen Dank für deinen Einkauf!\nBestellübersicht\n---\nBlue mug × 1\n€12,00\nRed plate × 2\n€9,00\nZwischensumme\n€21,00";

    #[test]
    fn extracts_bounded_order_lines_and_preserves_quantity_evidence() {
        let parsed = extract("s", "c1", ORDER).unwrap().unwrap();
        assert_eq!(parsed.proposals.len(), 2);
        let p = serde_json::to_value(&parsed.proposals[1]).unwrap();
        assert_eq!(p["title"], "Red plate");
        assert_eq!(p["evidence"], "Red plate × 2");
        assert_eq!(p["extraction_version"], REVISION);
    }

    #[test]
    fn unsupported_or_partial_sections_abstain_without_partial_proposals() {
        for mail in [
            ORDER.replace("Red plate × 2", "Red plate quantity unknown"),
            ORDER.replace("Zwischensumme", "not a section end"),
            ORDER.replace("Blue mug × 1", "Blue mug × 0"),
            ORDER.replace(
                "Vielen Dank für deinen Einkauf!",
                "Your review is requested",
            ),
            ORDER.replace("Red plate × 2", "Blue mug × 1"),
        ] {
            assert!(extract("s", "c1", &mail).unwrap().is_none());
        }
    }

    #[test]
    fn shipment_preparation_and_unrelated_mail_are_not_shipment_claims() {
        let mail = "Your order is on the way\nItems in this shipment\nBlue mug × 1\nIf you have any questions, contact the shop";
        let p = extract("s", "c1", mail).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&p.proposals[0]).unwrap()["event"],
            "shipment"
        );
        assert!(extract(
            "s",
            "c1",
            &mail.replace("Your order is on the way", "Preparing your order")
        )
        .unwrap()
        .is_none());
        assert!(extract("s", "c1", "Please review Blue mug")
            .unwrap()
            .is_none());
        assert!(matches!(
            extract("s", "c3", mail),
            Err(EvidenceError::DataClass)
        ));
    }
}
