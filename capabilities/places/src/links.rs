//! What places answers on `GET /api/links` (libs/links/ISA.md, LNK-9 and LNK-16).
//!
//! Places holds two kinds of reference. `transaction_places.source_id` is finance's fingerprint
//! of a transaction, and finance's typed id for it is `fin:tx:<source_id>`, so the target's last
//! segment is the key places stores. A person (`ent:`) is reached through the name decisions in
//! `people.rs`.

use sjel_links::{Link, Links, TypedId};

use crate::store::Place;

/// The id kinds places answers for. Declared again as `links_to` in `service.toml`; the test
/// below holds the two equal.
pub const LINKS_TO: &[&str] = &["fin:tx", "ent"];

/// The `source_id` to look up, or why the target is not one places answers for.
pub fn source_id<'a>(target: &TypedId<'a>) -> Result<&'a str, String> {
    if target.kind != "fin:tx" {
        return Err(format!(
            "places holds no references to `{}`; it answers {LINKS_TO:?}",
            target.kind
        ));
    }
    Ok(&target.id[target.kind.len() + 1..])
}

pub fn links(target: &TypedId<'_>, places: Vec<(Place, String)>) -> Links {
    Links {
        to: target.id.to_owned(),
        links: places
            .into_iter()
            .map(|(place, precision)| Link {
                kind: "place".into(),
                at: None,
                // "venue · Bonn": how sure the link is, then where.
                meta: Some(match place.city {
                    Some(city) if city != place.name => format!("{precision} · {city}"),
                    _ => precision,
                }),
                title: place.name,
                id: place.id,
                via: "transaction_places.source_id",
            })
            .collect(),
        unlinkable: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_id_is_the_last_segment_of_a_finance_id() {
        let target = TypedId::parse("fin:tx:ab12cd").unwrap();
        assert_eq!(source_id(&target), Ok("ab12cd"));
        assert!(source_id(&TypedId::parse("trip:plan:1").unwrap()).is_err());
        assert!(source_id(&TypedId::parse("ent:1").unwrap()).is_err());
    }

    #[test]
    fn links_to_matches_the_manifest() {
        let manifest = include_str!("../service.toml");
        let line = manifest
            .lines()
            .find(|line| line.starts_with("links_to"))
            .expect("service.toml declares links_to");
        for kind in LINKS_TO {
            assert!(
                line.contains(&format!("\"{kind}\"")),
                "{kind} missing from {line}"
            );
        }
    }
}
