//! What calendar answers on `GET /api/links` (libs/links/ISA.md, LNK-8).
//!
//! Calendar holds one kind of reference: an entry trips wrote carries its plan as
//! `payload.plan_id`. The store reads that key only on `source = 'trips'` rows
//! (`CalendarStore::entries_for_plan`). Every entry has a typed id, so `unlinkable` is always 0.

use sjel_links::{Link, Links, TypedId};

use crate::model::Entry;

/// The id kinds calendar answers for. Declared again as `links_to` in `service.toml`; the test
/// below holds the two equal.
pub const LINKS_TO: &[&str] = &["trip:plan"];

/// Refuses a kind calendar holds no reference to, before the store is asked.
pub fn check(target: &TypedId<'_>) -> Result<(), String> {
    if LINKS_TO.contains(&target.kind) {
        Ok(())
    } else {
        Err(format!(
            "calendar holds no references to `{}`; it answers {LINKS_TO:?}",
            target.kind
        ))
    }
}

pub fn links(target: &TypedId<'_>, entries: Vec<Entry>) -> Links {
    Links {
        to: target.id.to_owned(),
        links: entries
            .into_iter()
            .map(|entry| Link {
                kind: "cal:entry".into(),
                at: Some(entry.starts_at),
                meta: entry.location,
                title: entry.title,
                id: entry.id,
                via: "payload.plan_id",
            })
            .collect(),
        unlinkable: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undeclared_kind_is_refused() {
        assert!(check(&TypedId::parse("ent:1").unwrap()).is_err());
        assert!(check(&TypedId::parse("trip:plan:1").unwrap()).is_ok());
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
