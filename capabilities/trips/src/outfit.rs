//! Outfits: which pieces are worn together, and on which day of the trip (2026-10-07).
//!
//! A pack list says what goes in the bag; an outfit says what is worn together, so the bag can
//! be checked against the days instead of guessed. `pieces` holds inventory item ids as soft
//! references, the same way `pack_list_items.item_ref` does.
//!
//! A table of its own rather than a new `plan_items.item_type`: that type set is a `CHECK` on
//! the one table holding rows that exist nowhere else, and widening it means rebuilding the
//! table (the reason meetups stayed `activity`, see `store::validate_meetup`).
//! `CREATE TABLE IF NOT EXISTS` rebuilds nothing.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use sjel_store::QueryAll;

use crate::pack::PackListRow;
use crate::store::TripsStore;

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

pub const DDL: &str = "
    -- Outfits (2026-10-07). See src/outfit.rs for why this is not a plan item type.
    CREATE TABLE IF NOT EXISTS {prefix}_outfits (
        id          TEXT PRIMARY KEY,
        plan_id     TEXT NOT NULL REFERENCES {prefix}_plans(id) ON DELETE CASCADE,
        position    INTEGER NOT NULL,
        name        TEXT NOT NULL,
        day         TEXT,
        pieces      TEXT NOT NULL DEFAULT '[]',
        note        TEXT,
        created_at  TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS {prefix}_idx_outfits_plan ON {prefix}_outfits(plan_id);
";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct OutfitInput {
    pub name: String,
    /// `YYYY-MM-DD`, or null for an outfit not bound to a day ("evening out").
    #[serde(default)]
    pub day: Option<String>,
    /// Inventory item ids worn together.
    #[serde(default)]
    pub pieces: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PutOutfits {
    pub outfits: Vec<OutfitInput>,
}

pub fn outfits_for_plan(store: &TripsStore, plan_id: &str) -> Fallible<Vec<OutfitInput>> {
    let prefix = store.prefix();
    let conn = store.borrow_connection()?;
    let rows = conn.query_all(
        &format!(
            "SELECT name, day, pieces, note FROM {prefix}_outfits
             WHERE plan_id = ?1 ORDER BY day IS NULL, day, position"
        ),
        params![&plan_id],
        |row| {
            Ok(OutfitInput {
                name: row.get(0)?,
                day: row.get(1)?,
                pieces: sjel_store::json_column(row, 2)?,
                note: row.get(3)?,
            })
        },
    )?;
    Ok(rows)
}

/// Replace a plan's outfits wholesale, like a pack list's items: the page says "these are the
/// outfits now", and one statement of that cannot disagree with itself.
pub fn replace_outfits(store: &TripsStore, plan_id: &str, outfits: &[OutfitInput]) -> Fallible<()> {
    for outfit in outfits {
        if outfit.name.trim().is_empty() {
            return Err("every outfit needs a name".into());
        }
        // Same shape check as `TripsStore::set_item_day`: a date the ORDER BY can sort.
        if let Some(day) = &outfit.day {
            let shaped = day.len() == 10
                && day.bytes().enumerate().all(|(i, b)| {
                    if i == 4 || i == 7 {
                        b == b'-'
                    } else {
                        b.is_ascii_digit()
                    }
                });
            if !shaped {
                return Err(format!("day must be YYYY-MM-DD or null, got {day:?}").into());
            }
        }
    }
    let prefix = store.prefix();
    let mut conn = store.borrow_connection()?;
    let now = crate::store::stamp();
    let transaction = sjel_store::write_transaction(&mut conn)?;
    transaction.execute(
        &format!("DELETE FROM {prefix}_outfits WHERE plan_id = ?1"),
        params![&plan_id],
    )?;
    for (position, outfit) in outfits.iter().enumerate() {
        let pieces: Vec<&str> = outfit
            .pieces
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect();
        transaction.execute(
            &format!(
                "INSERT INTO {prefix}_outfits
                    (id, plan_id, position, name, day, pieces, note, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)"
            ),
            params![
                &crate::store::new_id("trip:outfit"),
                &plan_id,
                position as i64,
                outfit.name.trim(),
                &outfit.day,
                serde_json::to_string(&pieces)?,
                &outfit.note,
                &now,
            ],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

/// What the page renders: each outfit with the pieces no pack list on this plan carries.
/// Computed here because the frontend renders and does not compute (`pack::render`).
pub fn render(outfits: &[OutfitInput], lists: &[PackListRow]) -> Value {
    let on_a_list: std::collections::HashSet<&str> = lists
        .iter()
        .flat_map(|list| list.items.iter().map(|item| item.item_ref.as_str()))
        .collect();
    json!({
        "outfits": outfits.iter().map(|o| {
            let missing: Vec<&String> =
                o.pieces.iter().filter(|p| !on_a_list.contains(p.as_str())).collect();
            json!({
                "name": o.name,
                "day": o.day,
                "pieces": o.pieces,
                "note": o.note,
                "not_on_pack_list": missing,
            })
        }).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::PackItemInput;

    fn outfit(name: &str, day: Option<&str>, pieces: &[&str]) -> OutfitInput {
        OutfitInput {
            name: name.into(),
            day: day.map(Into::into),
            pieces: pieces.iter().map(|p| p.to_string()).collect(),
            note: None,
        }
    }

    #[test]
    fn a_piece_no_pack_list_carries_is_named() {
        let lists = vec![PackListRow {
            id: "l".into(),
            name: "Bag".into(),
            stage_destination_id: None,
            stage_sequence: None,
            template_key: None,
            items: vec![PackItemInput {
                item_ref: "shirt".into(),
                packed: false,
                note: None,
            }],
        }];
        let rendered = render(&[outfit("Evening", None, &["shirt", "blazer"])], &lists);
        assert_eq!(
            rendered["outfits"][0]["not_on_pack_list"],
            json!(["blazer"])
        );
    }

    #[test]
    fn outfits_replace_and_read_back_in_day_order() {
        let dir = std::env::temp_dir().join(format!("trips-outfits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = TripsStore::open(&dir.join("t.db")).unwrap();
        let place = |id: &str| crate::store::PlaceRef {
            id: id.into(),
            name: id.into(),
            kind: crate::store::PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        };
        let plan = store
            .create_plan(&crate::store::CreatePlan {
                title: "Berlin".into(),
                origin: place("bonn"),
                destinations: vec![place("berlin")],
                date_start: "2026-10-08".into(),
                date_end: "2026-10-13".into(),
                interests: String::new(),
                travelers: Vec::new(),
                transport_modes: Vec::new(),
                stages: Vec::new(),
                cover_image_url: None,
                source: None,
            })
            .unwrap()
            .id;

        replace_outfits(
            &store,
            &plan,
            &[
                outfit("Sat", Some("2026-10-10"), &["a"]),
                outfit("Any evening", None, &["b"]),
                outfit("Fri", Some("2026-10-09"), &["a", " ", "c"]),
            ],
        )
        .unwrap();
        let read = outfits_for_plan(&store, &plan).unwrap();
        let names: Vec<&str> = read.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["Fri", "Sat", "Any evening"]);
        assert_eq!(read[0].pieces, ["a", "c"]);

        assert!(replace_outfits(&store, &plan, &[outfit("Bad", Some("9.10."), &[])]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
