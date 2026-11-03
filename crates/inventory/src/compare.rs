//! Wish comparison: the candidates for one purchase, side by side.
//!
//! A wish group holds the candidates. Each candidate wish carries numeric specs (`memory_gb`,
//! `bandwidth_gb_s`, ...) and a history of dated price observations. The group carries its
//! needs, in the same key vocabulary, so a reader can draw "what the job needs" next to "what
//! each candidate offers". Scoring and drawing belong to the reader (`apps/gpui/core`); this
//! module stores and validates.
//!
//! An observation is idempotent per wish, source and calendar day, so a daily job that runs twice
//! records once. The first observation at or below the wish's `cost_max_cents` is the alert, and
//! no later one is: the alert means "the limit was reached", not "the price moved".

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{Error, Item, Store};

/// One numeric property: a candidate's spec, or a group's need.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Spec {
    pub key: String,
    pub value: f64,
    #[serde(default)]
    pub unit: String,
}

/// One dated price for one candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Observation {
    pub price_cents: i64,
    /// A product URL, or a short label such as `kleinanzeigen` for a manual entry.
    pub source: String,
    #[serde(default)]
    pub configuration: String,
    pub observed_at: String,
    /// The first observation at or below the wish's price limit.
    #[serde(default)]
    pub alert: bool,
}

/// What recording an observation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Recorded {
    /// False when this wish already has an observation from this source on this day.
    pub added: bool,
    pub alert: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Candidate {
    pub id: String,
    pub label: String,
    pub link: Option<String>,
    pub cost_max_cents: Option<i64>,
    pub specs: Vec<Spec>,
    /// Oldest first.
    pub observations: Vec<Observation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GroupComparison {
    pub id: String,
    pub title: String,
    pub constraint_text: String,
    pub needs: Vec<Spec>,
    pub candidates: Vec<Candidate>,
}

/// The tables, created next to the item tables. Idempotent.
pub(super) fn migrate(conn: &Connection, prefix: &str) -> Result<(), Error> {
    conn.execute_batch(&format!(
        "
        CREATE TABLE IF NOT EXISTS {prefix}_wish_spec (
            item_id TEXT NOT NULL REFERENCES {prefix}_item(id) ON DELETE CASCADE,
            key     TEXT NOT NULL,
            value   REAL NOT NULL,
            unit    TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (item_id, key)
        );
        CREATE TABLE IF NOT EXISTS {prefix}_wish_group_need (
            group_id TEXT NOT NULL REFERENCES {prefix}_wish_group(id) ON DELETE CASCADE,
            key      TEXT NOT NULL,
            value    REAL NOT NULL,
            unit     TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (group_id, key)
        );
        CREATE TABLE IF NOT EXISTS {prefix}_price_observation (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            item_id       TEXT NOT NULL REFERENCES {prefix}_item(id) ON DELETE CASCADE,
            source        TEXT NOT NULL,
            price_cents   INTEGER NOT NULL CHECK (price_cents > 0),
            configuration TEXT NOT NULL DEFAULT '',
            observed_at   TEXT NOT NULL,
            observed_on   TEXT NOT NULL,
            alert         INTEGER NOT NULL DEFAULT 0,
            UNIQUE (item_id, source, observed_on)
        );
        "
    ))?;
    let present: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = 'alternatives'",
        params![format!("{prefix}_wish_group")],
        |row| row.get(0),
    )?;
    if present == 0 {
        conn.execute_batch(&format!(
            "ALTER TABLE {prefix}_wish_group ADD COLUMN alternatives INTEGER NOT NULL DEFAULT 0;"
        ))?;
    }
    Ok(())
}

/// What a wish costs at the low end: its price, else its lowest estimate.
fn low_cents(item: &Item) -> Option<i64> {
    item.price_cents.or(item.cost_min_cents)
}

/// The wishes a budget counts. A group marked as alternatives is one purchase, so of its members
/// only the cheapest priced one counts. With no priced member, the first one counts, unpriced.
/// Groups without the mark add up as before. `apps/gpui/core` applies the same rule
/// (`inventory::counted`) to the lists it narrows on the client.
pub fn counted<'a>(items: Vec<&'a Item>, alternatives: &[Vec<String>]) -> Vec<&'a Item> {
    let mut dropped = std::collections::BTreeSet::new();
    for members in alternatives {
        let present: Vec<&Item> = items
            .iter()
            .copied()
            .filter(|i| members.contains(&i.id))
            .collect();
        let keep = present
            .iter()
            .filter(|i| low_cents(i).is_some())
            .min_by_key(|i| low_cents(i))
            .or(present.first())
            .map(|i| i.id.clone());
        for item in present {
            if Some(&item.id) != keep.as_ref() {
                dropped.insert(item.id.clone());
            }
        }
    }
    items
        .into_iter()
        .filter(|i| !dropped.contains(&i.id))
        .collect()
}

/// A key is lower-case ASCII, digits and `_`, so it can name a chart axis and a CLI argument.
pub fn check_specs(specs: &[Spec]) -> Result<(), String> {
    for spec in specs {
        if spec.key.is_empty()
            || spec.key.len() > 40
            || !spec
                .key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(format!(
                "spec key `{}` must be 1 to 40 characters of a-z, 0-9 and _",
                spec.key
            ));
        }
        if !spec.value.is_finite() {
            return Err(format!("spec `{}` has no finite value", spec.key));
        }
        if spec.unit.len() > 16 {
            return Err(format!("unit of `{}` is longer than 16 bytes", spec.key));
        }
    }
    Ok(())
}

/// `memory_gb=32`, `bandwidth_gb_s=170:GB/s`: a key, a number, and an optional unit after `:`.
pub fn parse_spec(arg: &str) -> Result<Spec, String> {
    let (key, rest) = arg
        .split_once('=')
        .ok_or_else(|| format!("`{arg}` is not key=value"))?;
    let (value, unit) = rest.split_once(':').unwrap_or((rest, ""));
    let value: f64 = value
        .trim()
        .parse()
        .map_err(|_| format!("`{value}` in `{arg}` is not a number"))?;
    let spec = Spec {
        key: key.trim().to_string(),
        value,
        unit: unit.trim().to_string(),
    };
    check_specs(std::slice::from_ref(&spec))?;
    Ok(spec)
}

impl Store {
    /// Replace a wish's specs. An empty list clears them.
    pub fn set_specs(&self, item_id: &str, specs: &[Spec]) -> Result<(), Error> {
        check_specs(specs)?;
        if self.item(item_id)?.is_none() {
            return Err(format!("no entry `{item_id}`").into());
        }
        self.replace_numbers("wish_spec", "item_id", item_id, specs)
    }

    /// Mark a wish group as alternatives (one of its members gets bought) or not.
    pub fn set_alternatives(&self, group_id: &str, alternatives: bool) -> Result<(), Error> {
        let conn = self.conn()?;
        let changed = conn.execute(
            &format!(
                "UPDATE {}_wish_group SET alternatives = ?2 WHERE id = ?1",
                self.prefix
            ),
            params![group_id, alternatives],
        )?;
        if changed == 0 {
            return Err(format!("no wish group `{group_id}`").into());
        }
        Ok(())
    }

    /// The member ids of every group marked as alternatives.
    pub fn alternative_groups(&self) -> Result<Vec<Vec<String>>, Error> {
        let p = &self.prefix;
        let conn = self.conn()?;
        let mut groups = conn.prepare(&format!(
            "SELECT id FROM {p}_wish_group WHERE alternatives = 1 ORDER BY id"
        ))?;
        let ids: Vec<String> = groups
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        let mut members = conn.prepare(&format!(
            "SELECT item_id FROM {p}_wish_group_item WHERE group_id = ?1 ORDER BY item_id"
        ))?;
        let mut out = Vec::new();
        for id in ids {
            out.push(
                members
                    .query_map(params![id], |row| row.get(0))?
                    .collect::<Result<_, _>>()?,
            );
        }
        Ok(out)
    }

    /// Replace a wish group's needs. An empty list clears them.
    pub fn set_needs(&self, group_id: &str, needs: &[Spec]) -> Result<(), Error> {
        check_specs(needs)?;
        let conn = self.conn()?;
        let known: Option<String> = conn
            .query_row(
                &format!("SELECT id FROM {}_wish_group WHERE id = ?1", self.prefix),
                params![group_id],
                |row| row.get(0),
            )
            .optional()?;
        drop(conn);
        if known.is_none() {
            return Err(format!("no wish group `{group_id}`").into());
        }
        self.replace_numbers("wish_group_need", "group_id", group_id, needs)
    }

    fn replace_numbers(
        &self,
        table: &str,
        owner_column: &str,
        owner: &str,
        specs: &[Spec],
    ) -> Result<(), Error> {
        let p = &self.prefix;
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        tx.execute(
            &format!("DELETE FROM {p}_{table} WHERE {owner_column} = ?1"),
            params![owner],
        )?;
        for spec in specs {
            tx.execute(
                &format!(
                    "INSERT INTO {p}_{table} ({owner_column}, key, value, unit) VALUES (?1, ?2, ?3, ?4)"
                ),
                params![owner, spec.key, spec.value, spec.unit],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Record one price. `observed_at` defaults to now; a manual entry may pass a date
    /// (`2026-11-03`) or a full timestamp. A second observation from the same source on the same
    /// day is ignored.
    pub fn record_price(
        &self,
        item_id: &str,
        source: &str,
        price_cents: i64,
        configuration: &str,
        observed_at: Option<&str>,
    ) -> Result<Recorded, Error> {
        if price_cents <= 0 {
            return Err("a price must be above zero".into());
        }
        let source = source.trim();
        if source.is_empty() {
            return Err("a price needs a source".into());
        }
        let Some((item, _)) = self.item(item_id)? else {
            return Err(format!("no entry `{item_id}`").into());
        };
        let at = match observed_at.map(str::trim).filter(|s| !s.is_empty()) {
            Some(at) => {
                check_date(at)?;
                at.to_string()
            }
            None => {
                let conn = self.conn()?;
                let now: String = conn.query_row(
                    &format!("SELECT {}", sjel_store::now_offset("'+0 seconds'")),
                    [],
                    |row| row.get(0),
                )?;
                now
            }
        };
        let on = at[..10].to_string();

        let p = &self.prefix;
        let mut conn = self.conn()?;
        let tx = sjel_store::write_transaction(&mut conn)?;
        let alerted_before: i64 = tx.query_row(
            &format!("SELECT COUNT(*) FROM {p}_price_observation WHERE item_id = ?1 AND alert = 1"),
            params![item_id],
            |row| row.get(0),
        )?;
        let alert = alerted_before == 0
            && item
                .cost_max_cents
                .is_some_and(|limit| price_cents <= limit);
        let added = tx.execute(
            &format!(
                "INSERT OR IGNORE INTO {p}_price_observation
                    (item_id, source, price_cents, configuration, observed_at, observed_on, alert)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            ),
            params![item_id, source, price_cents, configuration, at, on, alert],
        )? == 1;
        tx.commit()?;
        Ok(Recorded {
            added,
            alert: added && alert,
        })
    }

    /// Every wish group with its needs and its candidates' specs and price histories.
    pub fn comparisons(&self) -> Result<Vec<GroupComparison>, Error> {
        let catalogue = self.catalogue()?;
        let p = &self.prefix;
        let conn = self.conn()?;
        let numbers = |table: &str, owner_column: &str, owner: &str| -> Result<Vec<Spec>, Error> {
            let mut stmt = conn.prepare(&format!(
                "SELECT key, value, unit FROM {p}_{table} WHERE {owner_column} = ?1 ORDER BY key"
            ))?;
            let rows = stmt.query_map(params![owner], |row| {
                Ok(Spec {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    unit: row.get(2)?,
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        };
        let mut groups = conn.prepare(&format!(
            "SELECT id, title, constraint_text FROM {p}_wish_group ORDER BY title"
        ))?;
        let groups: Vec<(String, String, String)> = groups
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<_, _>>()?;
        let mut out = Vec::new();
        for (id, title, constraint_text) in groups {
            let mut members = conn.prepare(&format!(
                "SELECT item_id FROM {p}_wish_group_item WHERE group_id = ?1 ORDER BY item_id"
            ))?;
            let members: Vec<String> = members
                .query_map(params![id], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            let mut candidates = Vec::new();
            for item_id in members {
                let Some((item, _)) = catalogue.get(&item_id) else {
                    continue;
                };
                let mut stmt = conn.prepare(&format!(
                    "SELECT price_cents, source, configuration, observed_at, alert
                     FROM {p}_price_observation WHERE item_id = ?1 ORDER BY observed_at, id"
                ))?;
                let observations = stmt
                    .query_map(params![item_id], |row| {
                        Ok(Observation {
                            price_cents: row.get(0)?,
                            source: row.get(1)?,
                            configuration: row.get(2)?,
                            observed_at: row.get(3)?,
                            alert: row.get::<_, i64>(4)? == 1,
                        })
                    })?
                    .collect::<Result<_, _>>()?;
                candidates.push(Candidate {
                    specs: numbers("wish_spec", "item_id", &item_id)?,
                    id: item_id,
                    label: item.label.clone(),
                    link: item.link.clone(),
                    cost_max_cents: item.cost_max_cents,
                    observations,
                });
            }
            out.push(GroupComparison {
                needs: numbers("wish_group_need", "group_id", &id)?,
                id,
                title,
                constraint_text,
                candidates,
            });
        }
        Ok(out)
    }
}

/// `YYYY-MM-DD`, optionally followed by a time. Only the shape is checked: the day is what the
/// idempotence key uses.
fn check_date(at: &str) -> Result<(), Error> {
    let b = at.as_bytes();
    let shape = b.len() >= 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit);
    if shape {
        Ok(())
    } else {
        Err(format!("`{at}` is not a date (YYYY-MM-DD)").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_argument_has_a_key_a_number_and_an_optional_unit() {
        assert_eq!(
            parse_spec("bandwidth_gb_s=170:GB/s").unwrap(),
            Spec {
                key: "bandwidth_gb_s".into(),
                value: 170.0,
                unit: "GB/s".into()
            }
        );
        assert_eq!(parse_spec("memory_gb=32").unwrap().unit, "");
        assert!(parse_spec("Memory=32").is_err());
        assert!(parse_spec("memory_gb=lots").is_err());
        assert!(parse_spec("memory_gb").is_err());
    }

    fn priced(id: &str, price: Option<i64>, min: Option<i64>) -> Item {
        Item {
            id: id.into(),
            price_cents: price,
            cost_min_cents: min,
            ..Item::default()
        }
    }

    #[test]
    fn alternatives_count_once_at_their_cheapest() {
        let items = [
            priced("m6", Some(140_102), None),
            priced("m5", Some(247_000), None),
            priced("estimate", None, Some(100_000)),
            priced("lamp", Some(3_000), None),
        ];
        let all: Vec<&Item> = items.iter().collect();
        let ids = |v: Vec<&Item>| v.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
        let group = vec!["m6".to_string(), "m5".to_string(), "estimate".to_string()];
        assert_eq!(
            ids(counted(all.clone(), &[group])),
            vec!["estimate", "lamp"]
        );
        assert_eq!(
            ids(counted(all.clone(), &[])),
            vec!["m6", "m5", "estimate", "lamp"]
        );
        // No priced member: the first counts, so the unpriced count stays honest.
        let unpriced = [priced("a", None, None), priced("b", None, None)];
        let refs: Vec<&Item> = unpriced.iter().collect();
        assert_eq!(
            ids(counted(refs, &[vec!["a".into(), "b".into()]])),
            vec!["a"]
        );
    }

    #[test]
    fn a_date_needs_the_day_shape() {
        assert!(check_date("2026-11-03").is_ok());
        assert!(check_date("2026-11-03T08:00:00.000Z").is_ok());
        assert!(check_date("03.11.2026").is_err());
        assert!(check_date("2026-11").is_err());
    }
}
