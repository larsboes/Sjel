//! Which items are still judged by their name, and what changes if they declare themselves?
//!
//! The name heuristic (`bett*` is a bed) stays for items that declare nothing, so no verdict moves
//! without someone checking the numbers. This shows per item: whether it declares itself, else what
//! the heuristic takes it for and which thresholds it inherits; the declaration that writes down
//! exactly that, as TOML; and which layout verdicts move if it is adopted.
//!
//! A proposal is not neutral: the name version checks a bed's two long sides, the declared version
//! counts every deep enough side. Adopting it is a decision, which is why this writes nothing.

use crate::clearance::{check_layout, kind_of_name, Kind};
use crate::model::{Model, ModelError};
use crate::store::Item;
use serde::Serialize;

/// The fields a piece can state about itself.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Proposal {
    pub open_clear: Option<i32>,
    pub access_sides: Option<i32>,
    pub access_clear: Option<i32>,
    pub expands_dir: Option<String>,
    pub expands_to: Option<i32>,
    /// The same fields as TOML, ready to paste into `inventory/*.toml`.
    pub toml: String,
}

/// What moves in the verdicts if the proposal is adopted.
#[derive(Debug, Clone, Serialize)]
pub struct Consequences {
    pub layouts: usize,
    pub passing_before: usize,
    pub passing_after: usize,
    /// Layouts whose violation list changes, even if the verdict stays.
    pub changed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stand {
    pub id: String,
    pub label: String,
    /// Does the piece already declare what it needs?
    pub declared: bool,
    /// What the name heuristic takes it for while it does not.
    pub guessed_as: &'static str,
    /// The `rules.toml` thresholds it inherits that way.
    pub inherited_thresholds: Vec<(String, i32)>,
    /// Layouts it stands in: that many verdicts depend on it.
    pub in_layouts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Proposal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consequences: Option<Consequences>,
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Bed => "Bett",
        Kind::Desk => "Schreibtisch",
        Kind::Couch => "Sofa",
        Kind::Wardrobe => "Schrank",
        Kind::CoffeeTable => "Couchtisch",
        Kind::Table => "Tisch",
        Kind::Shelf => "Regal",
        Kind::Other => "nichts davon",
    }
}

/// Same condition as in `clearance.rs`.
pub fn declared(i: &Item) -> bool {
    i.open_clear.is_some() || i.access_sides.is_some() || i.expands_to.is_some()
}

/// The fields that write down what the heuristic already assumes. `None` for coffee tables,
/// shelves and others: the heuristic assumes nothing there, and a proposal would be a new rule.
pub fn proposal(model: &Model, i: &Item) -> Option<Proposal> {
    let r = &model.rules;
    let mut v = Proposal::default();
    match kind_of_name(&i.id) {
        Kind::Bed => {
            let lengthwise = r.threshold("bett_zugang_laengsseite").ok()?;
            let second = r.threshold("bett_zugang_zweite_seite").ok()?;
            v.access_clear = Some(lengthwise);
            // A flat that asks nothing of the second long side means single occupancy: one side is enough.
            v.access_sides = Some(if second > 0 { 2 } else { 1 });
        }
        Kind::Desk => {
            v.access_sides = Some(1);
            v.access_clear = Some(r.threshold("schreibtisch_stuhlzone").ok()?);
        }
        Kind::Wardrobe => v.open_clear = Some(r.threshold("schrank_tuer_oeffnen").ok()?),
        Kind::Table => {
            v.access_sides = Some(2);
            v.access_clear = Some(r.threshold("esstisch_stuhl_ausziehen").ok()?);
        }
        Kind::Couch => {
            // The unfolded depth from the piece where measured, else the flat's norm, as a total.
            let depth = i.depth.unwrap_or(0);
            v.expands_to = Some(
                i.depth_unfolded
                    .unwrap_or(depth + r.threshold("couch_ausklapptiefe").ok()?),
            );
            v.expands_dir = Some("south".into());
        }
        Kind::CoffeeTable | Kind::Shelf | Kind::Other => return None,
    }
    v.toml = toml_lines(&v);
    Some(v)
}

fn toml_lines(v: &Proposal) -> String {
    let mut z = Vec::new();
    if let Some(x) = v.open_clear {
        z.push(format!("open_clear = {x}"));
    }
    if let Some(x) = v.access_sides {
        z.push(format!("access_sides = {x}"));
    }
    if let Some(x) = v.access_clear {
        z.push(format!("access_clear = {x}"));
    }
    if let (Some(d), Some(t)) = (v.expands_dir.as_ref(), v.expands_to) {
        z.push(format!("expands = {{ dir = \"{d}\", to = {t} }}"));
    }
    z.join("\n")
}

fn applied(i: &Item, v: &Proposal) -> Item {
    let mut n = i.clone();
    n.open_clear = v.open_clear.or(n.open_clear);
    n.access_sides = v.access_sides.or(n.access_sides);
    n.access_clear = v.access_clear.or(n.access_clear);
    n.expands_to = v.expands_to.or(n.expands_to);
    n.expands_dir = v
        .expands_dir
        .as_deref()
        .and_then(crate::model::Side::parse)
        .or(n.expands_dir);
    n
}

/// A layout, its verdict and the ids of what it flagged; the texts do not matter for comparing.
#[derive(PartialEq)]
struct Verdict {
    layout: String,
    pass: bool,
    hart: Vec<String>,
    soft: Vec<String>,
}

/// Every verdict of this flat, as the baseline.
fn verdicts(model: &Model) -> Result<Vec<Verdict>, ModelError> {
    let mut out = Vec::new();
    for name in model.layout_names()? {
        let l = model.load_layout(&name)?;
        let r = check_layout(model, &l)?;
        out.push(Verdict {
            layout: name,
            pass: r.pass,
            hart: r.hard.iter().map(|v| v.rule.clone()).collect(),
            soft: r.soft.iter().map(|v| v.rule.clone()).collect(),
        });
    }
    Ok(out)
}

/// How far the inventory lags behind the checker, item by item.
pub fn overview(model: &Model) -> Result<Vec<Stand>, ModelError> {
    let before = verdicts(model)?;
    let mut in_layouts: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for name in model.layout_names()? {
        for it in &model.load_layout(&name)?.items {
            in_layouts
                .entry(it.reference.clone())
                .or_default()
                .push(name.clone());
        }
    }

    let mut out = Vec::new();
    for i in model.catalogue.values() {
        let k = kind_of_name(&i.id);
        let already = declared(i);
        let v = if already { None } else { proposal(model, i) };

        let consequences = match &v {
            None => None,
            Some(v) => {
                // The same computation as `POST /api/items/:id/impact`, for the proposal.
                let mut probe = Model {
                    room: model.room.clone(),
                    rules: model.rules.clone(),
                    catalogue: model.catalogue.clone(),
                    states: model.states.clone(),
                    flat_dir: model.flat_dir.clone(),
                };
                probe.catalogue.insert(i.id.clone(), applied(i, v));
                let after = verdicts(&probe)?;
                let changed = before
                    .iter()
                    .zip(after.iter())
                    .filter(|(a, b)| a != b)
                    .map(|(a, _)| a.layout.clone())
                    .collect();
                Some(Consequences {
                    layouts: before.len(),
                    passing_before: before.iter().filter(|x| x.pass).count(),
                    passing_after: after.iter().filter(|x| x.pass).count(),
                    changed,
                })
            }
        };

        out.push(Stand {
            id: i.id.clone(),
            label: i.label.clone(),
            declared: already,
            guessed_as: kind_name(k),
            inherited_thresholds: inherited_thresholds(model, k),
            in_layouts: in_layouts.get(&i.id).cloned().unwrap_or_default(),
            proposal: v,
            consequences,
        });
    }
    Ok(out)
}

/// The `rules.toml` thresholds a piece inherits through its name.
fn inherited_thresholds(model: &Model, k: Kind) -> Vec<(String, i32)> {
    let names: &[&str] = match k {
        Kind::Bed => &["bett_zugang_laengsseite", "bett_zugang_zweite_seite"],
        Kind::Desk => &["schreibtisch_stuhlzone"],
        Kind::Wardrobe => &["schrank_tuer_oeffnen"],
        Kind::Couch => &["couch_ausklapptiefe"],
        Kind::Table => &["esstisch_stuhl_ausziehen"],
        Kind::CoffeeTable => &["couchtisch_vor_sofa"],
        Kind::Shelf | Kind::Other => &[],
    };
    names
        .iter()
        .filter_map(|n| model.rules.threshold(n).ok().map(|v| (n.to_string(), v)))
        .collect()
}
