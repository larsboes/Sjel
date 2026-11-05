//! Types, matrix dimension tokens, column formatting, and status pills for PacksPage.

use std::time::Instant;

use gpui_kit::prelude::*;
use gpui_kit::{div, px, relative, AnyElement, IntoElement};
use sjel_gpui_core::packs::MatrixRow;

use crate::primitives::tone_chip;

/// The payload's reductions, kept until a new measurement settles.
pub(crate) struct Derived {
    pub(crate) loaded_at: Option<Instant>,
    pub(crate) rows: Vec<MatrixRow>,
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
pub(crate) fn pill(tone: u32, label: String) -> AnyElement {
    tone_chip(label, tone)
}
