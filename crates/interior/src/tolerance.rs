//! How much measurement error a verdict survives.
//!
//! The inventory marks guessed sizes (`estimated`), and the check used to compute with them as if
//! measured. This answers in centimetres: up to which error does this layout still pass?
//!
//! Bisection is valid because passing falls monotonically: the disturbance only grows sizes, and a
//! bigger rect can only shrink free depths, corridor widths and gaps. Sizes grow upward only,
//! because an underestimate is what ruins a move.
//!
//! Not varied, and named in the report: the flat (its uncertainty is prose in `room.toml`), and
//! pieces whose size the layout itself carries (`size = [w, d]`).

use crate::clearance::{check_layout, Uncertainty};
use crate::model::{Layout, Model, ModelError};
use serde::Serialize;

/// Upper end of the search, in cm. Past it the question is no longer about measuring.
pub const HORIZON_CM: i32 = 50;

/// How much measurement error a verdict survives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Holds {
    /// Fails with the recorded sizes already.
    Fails,
    /// No size in this layout is marked estimated. That says nothing was marked, not that everything
    /// was measured.
    NothingGuessed,
    /// Passes up to this error in cm, and fails one cm beyond.
    UpTo { cm: i32 },
    /// Still passes at `HORIZON_CM`.
    BeyondHorizon { horizon_cm: i32 },
}

#[derive(Debug, Clone, Serialize)]
pub struct Robustness {
    pub layout: String,
    pub nominal_pass: bool,
    /// The tightest hard measurement with the recorded sizes.
    pub tightest_reserve_cm: Option<i32>,
    pub holds: Holds,
    /// Which rules break first, one cm past `holds`: what to measure again, and why.
    pub tips_at: Vec<String>,
    /// The pieces with estimated sizes; they carry the risk.
    pub guessed: Vec<Uncertainty>,
    /// Pieces with `size:` in the layout: they carry their own size and are not varied.
    pub not_varied: Vec<String>,
}

/// The same model with estimated sizes grown by `tol`. Only the named dimensions grow: `["height"]`
/// means the height is a guess, not the whole piece.
fn disturbed(model: &Model, tol: i32) -> Model {
    let mut catalogue = model.catalogue.clone();
    for it in catalogue.values_mut() {
        // A blanket increase would answer what happens if everything is wrong, a different question.
        let estimated = std::mem::take(&mut it.estimated);
        for field in &estimated {
            let target = match field.as_str() {
                "width" => &mut it.width,
                "depth" => &mut it.depth,
                "height" => &mut it.height,
                // Only extents are varied; an estimated price changes no clearance.
                _ => continue,
            };
            if let Some(x) = target.as_mut() {
                *x += tol;
            }
        }
        it.estimated = estimated;
    }
    Model {
        room: model.room.clone(),
        rules: model.rules.clone(),
        catalogue,
        states: model.states.clone(),
        flat_dir: model.flat_dir.clone(),
    }
}

/// Does this layout still pass when every estimated size is `tol` cm off?
fn passes_at(model: &Model, layout: &Layout, tol: i32) -> Result<bool, ModelError> {
    Ok(check_layout(&disturbed(model, tol), layout)?.pass)
}

/// The same call, public so a test can check monotonicity on real numbers.
pub fn passes_at_for_test(model: &Model, layout: &Layout, tol: i32) -> Result<bool, ModelError> {
    passes_at(model, layout, tol)
}

pub fn robustness(model: &Model, layout: &Layout) -> Result<Robustness, ModelError> {
    let nominal = check_layout(model, layout)?;
    let refs: Vec<&str> = layout.items.iter().map(|i| i.reference.as_str()).collect();

    let guessed: Vec<Uncertainty> = nominal.uncertainties.clone();
    let not_varied: Vec<String> = layout
        .items
        .iter()
        .filter(|i| i.size.is_some())
        .map(|i| i.reference.clone())
        .collect();
    // An estimated size on a piece that brings its size in the layout is not disturbed: `footprint`
    // reads `size:` then.
    let effective: Vec<&Uncertainty> = guessed
        .iter()
        .filter(|u| !not_varied.contains(&u.reference) && refs.contains(&u.reference.as_str()))
        .collect();

    let holds = if !nominal.pass {
        Holds::Fails
    } else if effective.is_empty() {
        Holds::NothingGuessed
    } else if passes_at(model, layout, HORIZON_CM)? {
        Holds::BeyondHorizon {
            horizon_cm: HORIZON_CM,
        }
    } else {
        // Bisection over [0, HORIZON]: `lo` always passes, `hi` never.
        let (mut lo, mut hi) = (0, HORIZON_CM);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if passes_at(model, layout, mid)? {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Holds::UpTo { cm: lo }
    };

    let tips_at = match holds {
        Holds::UpTo { cm } => {
            let mut ids: Vec<String> = check_layout(&disturbed(model, cm + 1), layout)?
                .hard
                .into_iter()
                .map(|v| v.rule)
                .collect();
            ids.sort();
            ids.dedup();
            ids
        }
        _ => Vec::new(),
    };

    Ok(Robustness {
        layout: layout.name.clone(),
        nominal_pass: nominal.pass,
        tightest_reserve_cm: nominal.tightest_reserve_cm,
        holds,
        tips_at,
        guessed,
        not_varied,
    })
}
