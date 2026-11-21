//! Types, matrix dimension tokens, column formatting, and status pills for PacksPage.

use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, IntoElement, SharedString};

use crate::ely::data_display::{Badge, Tone};

pub(crate) struct ResolvedMatrixCell {
    pub(crate) harness_id: String,
    pub(crate) write_supported: bool,
    pub(crate) status: String,
    pub(crate) button_id: SharedString,
}

pub(crate) struct ResolvedMatrixRow {
    pub(crate) row_id: SharedString,
    pub(crate) pack: String,
    pub(crate) skill: String,
    pub(crate) needs_attention: bool,
    pub(crate) cells: Vec<ResolvedMatrixCell>,
}

/// The payload's reductions, kept until a new measurement settles.
pub(crate) struct Derived {
    pub(crate) loaded_at: Option<Instant>,
    pub(crate) resolved_rows: Vec<ResolvedMatrixRow>,
    pub(crate) pack_names: Vec<String>,
}

// Matrix column widths: share of the row and minimum. Header and body share them, so they align.
pub(crate) const PACK_COL: f32 = 0.16;
pub(crate) const SKILL_COL: f32 = 0.26;
pub(crate) const PACK_MIN: f32 = 96.;
pub(crate) const SKILL_MIN: f32 = 132.;
pub(crate) const HARNESS_MIN: f32 = 84.;

/// One matrix column, sized by its share of the row rather than by what it holds.
pub(crate) fn column(basis: f32, min: f32, inner: AnyElement) -> impl IntoElement {
    div()
        .flex_basis(relative(basis))
        .flex_shrink_1()
        .min_w(px(min))
        .child(inner)
}

/// A status chip: the tone says what the value means, the label names it.
pub(crate) fn pill(tone: Tone, label: String) -> AnyElement {
    Badge::new(label).tone(tone).into_any_element()
}
