//! Strict purchase-section parsing; unsupported layouts abstain as a whole.

use crate::mail_evidence::{self, EvidenceError, Parsed, Proposal, Stage};

/// The deterministic adapter revision retained with each proposal.
pub const REVISION: &str = "purchase-sections-v2";

fn quantity(value: &str) -> bool {
    value.parse::<u32>().is_ok_and(|n| (1..=999).contains(&n))
}

fn tracking_link(line: &str) -> bool {
    if let Some(token) = line
        .strip_prefix("<LINK_")
        .and_then(|s| s.strip_suffix('>'))
    {
        return !token.is_empty() && token.bytes().all(|b| b.is_ascii_alphanumeric());
    }
    reqwest::Url::parse(line)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn bullet_rows(lines: &[&str], event: &str, statement: &str) -> Option<Vec<serde_json::Value>> {
    let lines: Vec<_> = lines.iter().copied().filter(|l| !l.is_empty()).collect();
    if !tracking_link(lines.first()?) {
        return None;
    }
    let mut index = 1;
    let mut rows = Vec::new();
    while index < lines.len() {
        let title = lines[index].strip_prefix("* ")?;
        let count = lines.get(index + 1)?.strip_prefix("Menge: ")?;
        if title.is_empty() || !quantity(count) {
            return None;
        }
        let evidence = format!("{}\n{}", lines[index], lines[index + 1]);
        let mut row = serde_json::json!({"event": event, "title": title, "evidence": evidence});
        if event == "delivered" {
            row["claim"] = statement.into();
        }
        rows.push(row);
        if rows.len() > mail_evidence::MAX_PROPOSALS {
            return None;
        }
        index += 2;
        if event == "cancellation"
            && lines.get(index).is_some_and(|line| {
                line.strip_suffix(" EUR")
                    .is_some_and(|amount| price(&format!("€{amount}")))
            })
        {
            index += 1;
        }
    }
    (!rows.is_empty()).then_some(rows)
}

fn price(line: &str) -> bool {
    let amount = line.strip_prefix(['€', '$', '£']);
    amount.is_some_and(|s| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '.' | ','))
            && s.chars().any(|c| c.is_ascii_digit())
    })
}

/// Extract a complete supported purchase section, otherwise abstain.
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
        (
            "Lieferung verfolgen",
            "Dein Paket wurde zugestellt!",
            "Um Artikel zurückzusenden oder zu ersetzen, gehe zu Meine Bestellungen",
            "delivered",
        ),
        (
            "Bestelldetails anzeigen",
            "Deine Artikel wurden storniert",
            "Amazon.de ist ein Handelsname",
            "cancellation",
        ),
    ];
    let frames: Vec<_> = sections
        .iter()
        .filter(|(heading, statement, _, _)| lines.contains(heading) && lines.contains(statement))
        .collect();
    if frames.len() != 1
        || sections
            .iter()
            .filter(|(_, statement, _, _)| lines.contains(statement))
            .count()
            != 1
    {
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
    let statements: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == statement)
        .map(|(i, _)| i)
        .collect();
    if statements.len() != 1 || statements[0] >= start {
        return Ok(None);
    }
    let prefix = &lines[statements[0] + 1..start];
    let status = match event {
        "delivered" => Some("Heute zugestellt"),
        "cancellation" => Some("Storniert"),
        _ => None,
    };
    if status.is_some_and(|status| prefix.iter().filter(|line| **line == status).count() != 1)
        || (event == "cancellation" && !prefix.contains(&"Deine Bestellung wurde storniert. Diese Bestellung wurde dir nicht in Rechnung gestellt."))
    {
        return Ok(None);
    }
    let mut rows = Vec::new();
    let mut ended = false;
    if matches!(event, "delivered" | "cancellation") {
        let Some(end_index) = lines[start + 1..]
            .iter()
            .position(|line| line.starts_with(end))
        else {
            return Ok(None);
        };
        let Some(parsed) = bullet_rows(&lines[start + 1..start + 1 + end_index], event, statement)
        else {
            return Ok(None);
        };
        rows = parsed;
        ended = true;
    } else {
        for line in &lines[start + 1..] {
            if line.starts_with(end) {
                ended = true;
                break;
            }
            if line.is_empty() || line.chars().all(|c| c == '-') || price(line) {
                continue;
            }
            let Some((title, count)) = line.rsplit_once(" × ") else {
                return Ok(None);
            };
            if title.is_empty() || !quantity(count) {
                return Ok(None);
            }
            rows.push(serde_json::json!({"event":event, "title":title, "evidence":line}));
            if rows.len() > mail_evidence::MAX_PROPOSALS {
                return Ok(None);
            }
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
    const DELIVERED: &str = "Dein Paket wurde zugestellt!\nHeute zugestellt\nLieferung verfolgen\nhttps://shop.example.test/track/123\n* Blue mug\n  Menge: 1\nUm Artikel zurückzusenden oder zu ersetzen, gehe zu Meine Bestellungen";
    const CANCELLED: &str = "Deine Artikel wurden storniert\nStorniert\nDeine Bestellung wurde storniert. Diese Bestellung wurde dir nicht in Rechnung gestellt.\nBestelldetails anzeigen\nhttps://shop.example.test/order/123\n* Blue mug\n  Menge: 1\n12.00 EUR\nAmazon.de ist ein Handelsname";

    #[test]
    fn bullet_delivery_retains_separate_statement_and_quantity() {
        for mail in [
            DELIVERED.to_owned(),
            DELIVERED
                .replace("https://shop.example.test/track/123", "<LINK_abc123>")
                .replace('\n', "\r\n"),
        ] {
            let parsed = extract("s", "c1", &mail).unwrap().unwrap();
            let Proposal::Inventory(row) = &parsed.proposals[0] else {
                panic!("Inventory proposal required")
            };
            row.validate().unwrap();
            assert_eq!(
                row.event,
                inventory_api::mail_evidence::PurchaseEvent::Delivered
            );
            assert_eq!(row.claim.as_deref(), Some("Dein Paket wurde zugestellt!"));
            assert_eq!(row.evidence, "* Blue mug\nMenge: 1");
            assert_eq!(row.extraction_version.as_deref(), Some(REVISION));
        }
    }

    #[test]
    fn bullet_cancellation_is_a_review_claim_without_a_delivery_statement() {
        let mail = CANCELLED.replace("12.00 EUR", "12.00 EUR\n* Red plate\nMenge: 2");
        let parsed = extract("s", "c1", &mail).unwrap().unwrap();
        assert_eq!(parsed.proposals.len(), 2);
        for proposal in parsed.proposals {
            let Proposal::Inventory(row) = proposal else {
                panic!("Inventory proposal required")
            };
            row.validate().unwrap();
            assert_eq!(
                row.event,
                inventory_api::mail_evidence::PurchaseEvent::Cancellation
            );
            assert!(row.claim.is_none());
            assert!(row.order_ref.is_none());
        }
    }

    #[test]
    fn partial_or_unfamiliar_bullet_sections_abstain_as_a_whole() {
        for mail in [
            DELIVERED.replace("Heute zugestellt", "Wird heute zugestellt"),
            DELIVERED.replace("Dein Paket wurde zugestellt!", "Dein Paket wird zugestellt!"),
            DELIVERED.replace("Menge: 1", "Menge: 0"),
            DELIVERED.replace("Menge: 1", "Menge: unknown"),
            DELIVERED.replace("Menge: 1", "Menge: 1\nUnknown charge"),
            DELIVERED.replace("Menge: 1", "Menge: 1\n* Red plate"),
            DELIVERED.replace("Menge: 1", "Menge: 1\n* Blue mug\nMenge: 1"),
            DELIVERED.replace("https://shop.example.test/track/123", "not a tracking link"),
            DELIVERED.replace("Um Artikel zurückzusenden oder zu ersetzen, gehe zu Meine Bestellungen", "Footer missing"),
            CANCELLED.replace("Storniert\n", "Stornierung angefragt\n"),
            CANCELLED.replace("Deine Bestellung wurde storniert. Diese Bestellung wurde dir nicht in Rechnung gestellt.", "Deine Stornierung wird geprüft."),
            CANCELLED.replace("Amazon.de ist ein Handelsname", "Footer missing"),
        ] {
            assert!(extract("s", "c1", &mail).unwrap().is_none(), "{mail}");
        }
    }

    #[test]
    fn competing_frames_and_repeated_statements_abstain() {
        for mail in [
            format!("{DELIVERED}\n{CANCELLED}"),
            format!("{DELIVERED}\nDeine Artikel wurden storniert"),
            format!("{DELIVERED}\nDein Paket wurde zugestellt!"),
            format!("{ORDER}\n{DELIVERED}"),
        ] {
            assert!(extract("s", "c1", &mail).unwrap().is_none());
        }
    }

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
