//! Pack lists: which items go on which leg of a plan, and what is still
//! missing.
//!
//! ## The binding is the stage's DESTINATION, not its id
//!
//! `crate::store`'s `generated_stages` is the only stage-id producer in the
//! tree and it is `format!("stage:{}", sequence + 1)` — a pure function of
//! position. An id and an index are therefore one fact written twice, and
//! inserting or reordering a destination moves both consistently to the wrong
//! leg. `TripStage.destination` is a `PlaceRef` with an id that survives the
//! rebuild, so that is what a list records. `stage_sequence` is a tiebreak for
//! a plan that visits the same place twice, never a fallback. A destination
//! that is no longer on the plan reports `stage_binding: "lost"` rather than
//! attaching to a leg it is not sure about — the loud form `record_outcome`
//! already uses ("no stage {stage_id} on {plan_id}").
//!
//! ## `item_ref` is a soft reference with no foreign key
//!
//! It holds an `interior_item.id`. `libs/sjel-store` sets
//! `PRAGMA foreign_keys = ON` on every pooled connection, so a cross-prefix FK
//! would make interior's own deletes fail against a trips row and would order
//! trips' migration behind interior's. The shape follows `interior_placement`,
//! which binds the same way for the same reason.
//!
//! ## The attributes arrived on 2026-09-07
//!
//! Until B51, `interior_item` had no `weight_g`, `category`, `packable`,
//! `waterproof`, `quick_dry`, `pack_location` or `trip_types` column and a
//! `kind` that forced every gear row to claim it was furniture, so a resolved
//! item was a label and `gear_attributes` reported `false` with that reason.
//! The columns exist now, `kind` accepts `gear`, and this file reads them off
//! interior's own inventory contract.
//!
//! **A total is never a guess.** `total_weight_g` sums the weights that are
//! there and `weights_missing` counts the items that have none, because a sum
//! over an incomplete list, printed alone, is a number a reader will trust.

use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use sjel_store::QueryAll;

use crate::store::{TripStage, TripsStore};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct CreatePackList {
    pub name: String,
    /// The `PlaceRef.id` of the stage's destination. Null means the list covers
    /// the whole trip.
    #[serde(default)]
    pub stage_destination_id: Option<String>,
    /// Tiebreak for a plan that visits the same place twice.
    #[serde(default)]
    pub stage_sequence: Option<i64>,
    /// One value of the notes' own `trip_types` vocabulary. Free text on
    /// purpose: there is no template table, because "what belongs in this
    /// template" is a filter over item attributes the data already carries, and
    /// a second copy of a filter drifts.
    #[serde(default)]
    pub template_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PackItemInput {
    pub item_ref: String,
    #[serde(default)]
    pub packed: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct PutPackItems {
    pub items: Vec<PackItemInput>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackListRow {
    pub id: String,
    pub name: String,
    pub stage_destination_id: Option<String>,
    pub stage_sequence: Option<i64>,
    pub template_key: Option<String>,
    pub items: Vec<PackItemInput>,
}

/// The DDL, appended to `TripsStore::run_migration`'s batch as its own block.
/// Kept here so the shape and the queries that read it stay in one file.
pub const DDL: &str = "
    -- Pack lists (Q58 binding). See src/pack.rs for why the binding is the
    -- stage's destination place id and why item_ref carries no foreign key.
    CREATE TABLE IF NOT EXISTS {prefix}_pack_lists (
        id                    TEXT PRIMARY KEY,
        plan_id               TEXT NOT NULL REFERENCES {prefix}_plans(id) ON DELETE CASCADE,
        name                  TEXT NOT NULL,
        stage_destination_id  TEXT,
        stage_sequence        INTEGER,
        template_key          TEXT,
        created_at            TEXT NOT NULL,
        updated_at            TEXT NOT NULL,
        UNIQUE (plan_id, name)
    );
    CREATE INDEX IF NOT EXISTS {prefix}_idx_pack_list_plan
        ON {prefix}_pack_lists(plan_id);
    CREATE TABLE IF NOT EXISTS {prefix}_pack_list_items (
        id           TEXT PRIMARY KEY,
        pack_list_id TEXT NOT NULL REFERENCES {prefix}_pack_lists(id) ON DELETE CASCADE,
        item_ref     TEXT NOT NULL,
        packed       INTEGER NOT NULL DEFAULT 0,
        note         TEXT,
        created_at   TEXT NOT NULL,
        UNIQUE (pack_list_id, item_ref)
    );
";

pub fn lists_for_plan(store: &TripsStore, plan_id: &str) -> Fallible<Vec<PackListRow>> {
    let prefix = store.prefix();
    let conn = store.borrow_connection()?;
    let mut lists: Vec<PackListRow> = conn.query_all(
        &format!(
            "SELECT id, name, stage_destination_id, stage_sequence, template_key
             FROM {prefix}_pack_lists WHERE plan_id = ?1 ORDER BY stage_sequence, name"
        ),
        params![&plan_id],
        |row| {
            Ok(PackListRow {
                id: row.get(0)?,
                name: row.get(1)?,
                stage_destination_id: row.get(2)?,
                stage_sequence: row.get(3)?,
                template_key: row.get(4)?,
                items: Vec::new(),
            })
        },
    )?;
    for list in &mut lists {
        list.items = conn.query_all(
            &format!(
                "SELECT item_ref, packed, note FROM {prefix}_pack_list_items
                 WHERE pack_list_id = ?1 ORDER BY item_ref"
            ),
            params![&list.id],
            |row| {
                Ok(PackItemInput {
                    item_ref: row.get(0)?,
                    packed: row.get::<_, i64>(1)? != 0,
                    note: row.get(2)?,
                })
            },
        )?;
    }
    Ok(lists)
}

pub fn create_list(
    store: &TripsStore,
    plan_id: &str,
    input: &CreatePackList,
) -> Fallible<PackListRow> {
    if input.name.trim().is_empty() {
        return Err("name is required".into());
    }
    let prefix = store.prefix();
    let conn = store.borrow_connection()?;
    let id = crate::store::new_id("trip:pack");
    let now = crate::store::stamp();
    conn.execute(
        &format!(
            "INSERT INTO {prefix}_pack_lists
                (id, plan_id, name, stage_destination_id, stage_sequence,
                 template_key, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?7)
             ON CONFLICT (plan_id, name) DO UPDATE SET
                stage_destination_id = excluded.stage_destination_id,
                stage_sequence = excluded.stage_sequence,
                template_key = excluded.template_key,
                updated_at = excluded.updated_at"
        ),
        params![
            &id,
            &plan_id,
            input.name.trim(),
            &input.stage_destination_id,
            &input.stage_sequence,
            &input.template_key,
            &now,
        ],
    )?;
    // Re-read rather than assume: the conflict branch keeps the id that is
    // already there, and the caller needs the one it can address.
    let row = conn.query_row(
        &format!(
            "SELECT id, name, stage_destination_id, stage_sequence, template_key
             FROM {prefix}_pack_lists WHERE plan_id = ?1 AND name = ?2"
        ),
        params![&plan_id, input.name.trim()],
        |row| {
            Ok(PackListRow {
                id: row.get(0)?,
                name: row.get(1)?,
                stage_destination_id: row.get(2)?,
                stage_sequence: row.get(3)?,
                template_key: row.get(4)?,
                items: Vec::new(),
            })
        },
    )?;
    Ok(row)
}

pub fn delete_list(store: &TripsStore, plan_id: &str, list_id: &str) -> Fallible<bool> {
    let prefix = store.prefix();
    let conn = store.borrow_connection()?;
    // Scoped by plan_id as well as id, so a list id from another plan cannot be
    // deleted through this plan's route.
    let removed = conn.execute(
        &format!("DELETE FROM {prefix}_pack_lists WHERE id = ?1 AND plan_id = ?2"),
        params![&list_id, &plan_id],
    )?;
    Ok(removed > 0)
}

/// Replace a list's items wholesale.
///
/// One PUT rather than three per-item routes: the surface a page needs is "this
/// is the list now", and three routes would be three chances for the page and
/// the row to disagree.
pub fn replace_items(
    store: &TripsStore,
    plan_id: &str,
    list_id: &str,
    items: &[PackItemInput],
) -> Fallible<bool> {
    let prefix = store.prefix();
    let mut conn = store.borrow_connection()?;
    let owned: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM {prefix}_pack_lists WHERE id = ?1 AND plan_id = ?2"),
        params![&list_id, &plan_id],
        |row| row.get(0),
    )?;
    if owned == 0 {
        return Ok(false);
    }
    let now = crate::store::stamp();
    let transaction = sjel_store::write_transaction(&mut conn)?;
    transaction.execute(
        &format!("DELETE FROM {prefix}_pack_list_items WHERE pack_list_id = ?1"),
        params![&list_id],
    )?;
    for item in items {
        if item.item_ref.trim().is_empty() {
            continue;
        }
        transaction.execute(
            &format!(
                "INSERT INTO {prefix}_pack_list_items
                    (id, pack_list_id, item_ref, packed, note, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6)
                 ON CONFLICT (pack_list_id, item_ref) DO UPDATE SET
                    packed = excluded.packed, note = excluded.note"
            ),
            params![
                &crate::store::new_id("trip:packitem"),
                &list_id,
                item.item_ref.trim(),
                i64::from(item.packed),
                &item.note,
                &now,
            ],
        )?;
    }
    transaction.execute(
        &format!("UPDATE {prefix}_pack_lists SET updated_at = ?1 WHERE id = ?2"),
        params![&now, &list_id],
    )?;
    transaction.commit()?;
    Ok(true)
}

/// What the page renders. Every derived number is computed here, because the
/// frontend renders and does not compute.
///
/// One interior item, as a pack list needs it.
///
/// A struct rather than the `id -> label` map this took until 2026-09-07, because a pack list
/// sorts by pack location and adds up weights, and neither is a label.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InventoryItem {
    pub label: String,
    pub weight_g: Option<i64>,
    pub category: Option<String>,
    pub pack_location: Option<String>,
    pub packable: Option<bool>,
    pub waterproof: Option<bool>,
    pub quick_dry: Option<bool>,
    pub trip_types: Vec<String>,
}

/// `inventory` is `None` when interior could not be reached, which is a
/// different state from "the item is not there" and is reported as such.
pub fn render(
    lists: &[PackListRow],
    stages: &[TripStage],
    inventory: Option<&HashMap<String, InventoryItem>>,
    stage_filter: Option<&str>,
) -> Value {
    let mut unresolved: Vec<String> = Vec::new();
    let mut rendered = Vec::new();
    let mut missing_for_stage: Vec<Value> = Vec::new();

    for list in lists {
        let binding = match &list.stage_destination_id {
            None => "trip",
            Some(place_id) => {
                if stages.iter().any(|s| &s.destination.id == place_id) {
                    "place"
                } else {
                    "lost"
                }
            }
        };
        let in_scope = match (stage_filter, &list.stage_destination_id) {
            (None, _) => true,
            (Some(_), None) => true, // a whole-trip list is on every leg
            (Some(wanted), Some(bound)) => wanted == bound,
        };

        let mut items = Vec::new();
        let mut missing = Vec::new();
        let mut packed_count = 0usize;
        let mut total_weight_g: i64 = 0;
        let mut weights_missing = 0usize;
        for item in &list.items {
            let resolved = inventory.map(|index| index.contains_key(&item.item_ref));
            let known = inventory.and_then(|index| index.get(&item.item_ref));
            let label = known
                .map(|found| found.label.clone())
                .unwrap_or_else(|| item.item_ref.clone());
            if resolved == Some(false) && !unresolved.contains(&item.item_ref) {
                unresolved.push(item.item_ref.clone());
            }
            if item.packed {
                packed_count += 1;
            } else {
                let entry = json!({
                    "item_ref": item.item_ref,
                    "label": label,
                    "why": if resolved == Some(false) {
                        "not packed, and interior has no item with this id"
                    } else {
                        "not packed yet"
                    },
                });
                if in_scope {
                    missing_for_stage.push(entry.clone());
                }
                missing.push(entry);
            }
            match known.and_then(|found| found.weight_g) {
                Some(grams) => total_weight_g += grams,
                // Counted, not defaulted to zero: "weighs nothing" and "was never weighed"
                // are different facts and only one of them is in the database.
                None => weights_missing += 1,
            }
            items.push(json!({
                "item_ref": item.item_ref,
                "label": label,
                "packed": item.packed,
                "note": item.note,
                "resolved": resolved,
                "pack_location": known.and_then(|found| found.pack_location.clone()),
                "weight_g": known.and_then(|found| found.weight_g),
                "category": known.and_then(|found| found.category.clone()),
                "packable": known.and_then(|found| found.packable),
                "waterproof": known.and_then(|found| found.waterproof),
                "quick_dry": known.and_then(|found| found.quick_dry),
                "trip_types": known.map(|found| found.trip_types.clone()).unwrap_or_default(),
            }));
        }
        if binding == "lost" {
            let entry = json!({
                "item_ref": Value::Null,
                "label": list.name,
                "why": "the destination this list was bound to is no longer on the plan",
            });
            missing.push(entry.clone());
            if in_scope {
                missing_for_stage.push(entry);
            }
        }

        rendered.push(json!({
            "id": list.id,
            "name": list.name,
            "stage_destination_id": list.stage_destination_id,
            "stage_sequence": list.stage_sequence,
            "stage_binding": binding,
            "template_key": list.template_key,
            "items": items,
            "missing": missing,
            "packed_count": packed_count,
            "total_count": list.items.len(),
            "total_weight_g": total_weight_g,
            "weights_missing": weights_missing,
        }));
    }

    json!({
        "lists": rendered,
        "interior_reachable": inventory.is_some(),
        "unresolved_items": unresolved,
        "stage": stage_filter,
        "missing_for_stage": missing_for_stage,
        // True since B51 (2026-09-07). Kept as a field rather than removed: a consumer
        // written against the old answer must be able to see that it changed, and a client
        // reading a live deployment that has not migrated yet still gets a truthful `false`
        // when interior answers without the columns.
        "gear_attributes": inventory.is_some(),
        "gear_attributes_reason": if inventory.is_some() {
            Value::Null
        } else {
            json!("interior could not be reached, so no item resolved to more than its id")
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{PlaceKind, PlaceRef, StageStatus};

    fn stage(sequence: usize, place_id: &str) -> TripStage {
        let place = |id: &str| PlaceRef {
            id: id.into(),
            name: id.into(),
            kind: PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        };
        TripStage {
            id: format!("stage:{}", sequence + 1),
            sequence,
            origin: place("place:home"),
            destination: place(place_id),
            date: None,
            transport_modes: Vec::new(),
            travelers: Vec::new(),
            status: StageStatus::default(),
            selected_option_id: None,
            branch_note: None,
        }
    }

    fn list(name: &str, bound_to: Option<&str>, items: &[(&str, bool)]) -> PackListRow {
        PackListRow {
            id: format!("trip:pack:{name}"),
            name: name.into(),
            stage_destination_id: bound_to.map(str::to_string),
            stage_sequence: Some(0),
            template_key: None,
            items: items
                .iter()
                .map(|(item_ref, packed)| PackItemInput {
                    item_ref: (*item_ref).into(),
                    packed: *packed,
                    note: None,
                })
                .collect(),
        }
    }

    /// A list bound to a destination that has left the plan says so, instead of
    /// attaching itself to whichever leg happens to sit at that index now.
    #[test]
    fn a_list_bound_to_a_departed_destination_is_lost_not_reattached() {
        let rendered = render(
            &[list("Hiking", Some("place:gone"), &[("item:boots", false)])],
            &[stage(0, "place:oslo")],
            None,
            None,
        );
        assert_eq!(rendered["lists"][0]["stage_binding"], "lost");
        assert!(rendered["lists"][0]["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["why"]
                .as_str()
                .unwrap()
                .contains("no longer on the plan")));
    }

    /// "Missing for this stage" is the unpacked items of the leg's own list plus
    /// the whole-trip list, computed here and not in the page.
    #[test]
    fn missing_for_a_stage_covers_the_leg_and_the_whole_trip_list() {
        let rendered = render(
            &[
                list("Oslo", Some("place:oslo"), &[("item:boots", false)]),
                list("Bergen", Some("place:bergen"), &[("item:map", false)]),
                list(
                    "Always",
                    None,
                    &[("item:passport", false), ("item:charger", true)],
                ),
            ],
            &[stage(0, "place:oslo"), stage(1, "place:bergen")],
            None,
            Some("place:oslo"),
        );
        let refs: Vec<&str> = rendered["missing_for_stage"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["item_ref"].as_str().unwrap())
            .collect();
        assert_eq!(refs, vec!["item:boots", "item:passport"]);
    }

    /// An unreachable interior is a third state, distinct from "the item is
    /// gone": nothing is reported unresolved, and the flag says why.
    #[test]
    fn an_unreachable_interior_is_not_reported_as_a_missing_item() {
        let unreachable = render(
            &[list("Always", None, &[("item:ghost", false)])],
            &[],
            None,
            None,
        );
        assert_eq!(unreachable["interior_reachable"], false);
        assert!(unreachable["unresolved_items"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(unreachable["lists"][0]["items"][0]["resolved"], Value::Null);

        let index: HashMap<String, InventoryItem> = HashMap::new();
        let reachable = render(
            &[list("Always", None, &[("item:ghost", false)])],
            &[],
            Some(&index),
            None,
        );
        assert_eq!(reachable["interior_reachable"], true);
        assert_eq!(
            reachable["unresolved_items"].as_array().unwrap().len(),
            1,
            "a resolvable index that does not hold the id is a real gap"
        );
    }

    /// With interior unreachable, nothing is known and the flag says so. This test asserted
    /// the opposite until B51 — that every weight was null because the COLUMN did not exist —
    /// and it is the same contract from the other side: a null weight always carries a reason.
    #[test]
    fn an_unreachable_interior_knows_no_weights_and_says_why() {
        let rendered = render(
            &[list("Always", None, &[("item:x", true)])],
            &[],
            None,
            None,
        );
        assert_eq!(rendered["lists"][0]["items"][0]["weight_g"], Value::Null);
        assert_eq!(rendered["gear_attributes"], false);
        assert!(rendered["gear_attributes_reason"]
            .as_str()
            .unwrap()
            .contains("interior could not be reached"));
    }

    /// A total sums what is there and counts what is not. The number alone would be read as
    /// the weight of the list, and it is the weight of the part that has been weighed.
    #[test]
    fn a_total_says_how_much_of_the_list_it_covers() {
        let mut index: HashMap<String, InventoryItem> = HashMap::new();
        index.insert(
            "item:tent".to_string(),
            InventoryItem {
                label: "Tent".into(),
                weight_g: Some(1_850),
                pack_location: Some("rucksack".into()),
                category: Some("schlafen".into()),
                trip_types: vec!["hiking".into()],
                ..InventoryItem::default()
            },
        );
        index.insert(
            "item:socks".to_string(),
            InventoryItem {
                label: "Socks".into(),
                ..InventoryItem::default()
            },
        );
        let rendered = render(
            &[list(
                "Always",
                None,
                &[("item:tent", true), ("item:socks", true)],
            )],
            &[],
            Some(&index),
            None,
        );
        let first = &rendered["lists"][0];
        assert_eq!(first["total_weight_g"], 1_850);
        assert_eq!(first["weights_missing"], 1);
        assert_eq!(first["items"][0]["pack_location"], "rucksack");
        assert_eq!(first["items"][0]["trip_types"][0], "hiking");
        // Present and null, not absent: a consumer must be able to tell "never weighed" from
        // "this build does not report weights".
        assert!(first["items"][1]["weight_g"].is_null());
        assert_eq!(rendered["gear_attributes"], true);
        assert!(rendered["gear_attributes_reason"].is_null());
    }
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::store::{CreatePlan, PlaceKind, PlaceRef};

    fn open_test_store(suffix: &str) -> TripsStore {
        let dir = std::env::temp_dir().join(format!("trips-pack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        let path = dir.join(format!("{suffix}.db"));
        for tail in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{tail}", path.display()));
        }
        TripsStore::open(&path).expect("a test store")
    }

    fn place(id: &str) -> PlaceRef {
        PlaceRef {
            id: id.into(),
            name: id.into(),
            kind: PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        }
    }

    fn a_plan(store: &TripsStore) -> String {
        let plan = store
            .create_plan(&CreatePlan {
                title: "Test plan".into(),
                origin: place("place:home"),
                destinations: vec![place("place:oslo")],
                date_start: "2026-10-05".into(),
                date_end: "2026-10-09".into(),
                interests: String::new(),
                travelers: Vec::new(),
                transport_modes: Vec::new(),
                stages: Vec::new(),
                cover_image_url: None,
                source: None,
            })
            .expect("a plan");
        plan.id
    }

    /// The PUT replaces rather than accumulates: sending two items after three
    /// leaves two, which is what "this is the list now" has to mean.
    #[test]
    fn putting_items_replaces_the_list_rather_than_appending_to_it() {
        let store = open_test_store("replace");
        let plan_id = a_plan(&store);
        let list = create_list(
            &store,
            &plan_id,
            &CreatePackList {
                name: "Hiking".into(),
                stage_destination_id: Some("place:oslo".into()),
                stage_sequence: Some(0),
                template_key: Some("hiking".into()),
            },
        )
        .expect("a list");

        let three = |packed: bool| {
            vec![
                PackItemInput {
                    item_ref: "item:a".into(),
                    packed,
                    note: None,
                },
                PackItemInput {
                    item_ref: "item:b".into(),
                    packed,
                    note: None,
                },
                PackItemInput {
                    item_ref: "item:c".into(),
                    packed,
                    note: None,
                },
            ]
        };
        assert!(replace_items(&store, &plan_id, &list.id, &three(false)).expect("a write"));
        assert_eq!(lists_for_plan(&store, &plan_id).unwrap()[0].items.len(), 3);

        assert!(replace_items(
            &store,
            &plan_id,
            &list.id,
            &[PackItemInput {
                item_ref: "item:a".into(),
                packed: true,
                note: Some("in the bag".into())
            }],
        )
        .expect("a write"));
        let after = lists_for_plan(&store, &plan_id).expect("a read");
        assert_eq!(after[0].items.len(), 1);
        assert!(after[0].items[0].packed);
        assert_eq!(after[0].items[0].note.as_deref(), Some("in the bag"));
    }

    /// A list id belonging to another plan is not addressable through this
    /// plan's route, so a stale id in an open tab cannot delete a stranger's
    /// list.
    #[test]
    fn a_list_from_another_plan_is_not_reachable_through_this_plan() {
        let store = open_test_store("scope");
        let mine = a_plan(&store);
        let list = create_list(
            &store,
            &mine,
            &CreatePackList {
                name: "Mine".into(),
                stage_destination_id: None,
                stage_sequence: None,
                template_key: None,
            },
        )
        .expect("a list");

        assert!(!delete_list(&store, "trip:plan:someone-else", &list.id).expect("a delete"));
        assert!(!replace_items(&store, "trip:plan:someone-else", &list.id, &[]).expect("a write"));
        assert_eq!(lists_for_plan(&store, &mine).unwrap().len(), 1);
        assert!(delete_list(&store, &mine, &list.id).expect("a delete"));
        assert!(lists_for_plan(&store, &mine).unwrap().is_empty());
    }

    /// Deleting a plan takes its pack lists and their items with it — the
    /// ON DELETE CASCADE the DDL declares, checked rather than assumed.
    #[test]
    fn deleting_a_plan_takes_its_pack_lists_with_it() {
        let store = open_test_store("cascade");
        let plan_id = a_plan(&store);
        let list = create_list(
            &store,
            &plan_id,
            &CreatePackList {
                name: "Always".into(),
                stage_destination_id: None,
                stage_sequence: None,
                template_key: None,
            },
        )
        .expect("a list");
        replace_items(
            &store,
            &plan_id,
            &list.id,
            &[PackItemInput {
                item_ref: "item:a".into(),
                packed: false,
                note: None,
            }],
        )
        .expect("a write");

        assert!(store.delete_plan(&plan_id).expect("a delete"));
        assert!(lists_for_plan(&store, &plan_id).unwrap().is_empty());
        let conn = store.borrow_connection().expect("a connection");
        let orphans: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM trips_pack_list_items WHERE pack_list_id = ?1",
                params![&list.id],
                |row| row.get(0),
            )
            .expect("a count");
        assert_eq!(orphans, 0, "items outlived the plan they belonged to");
    }
}
