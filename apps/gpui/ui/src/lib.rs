//! Shared GPUI design system, tokens, layout primitives, and navigation for Sjel.
//!
//! Shared across macOS desktop and iOS mobile clients.

pub mod doc_view;
pub mod nav;
pub mod primitives;
pub mod theme;

pub use doc_view::{render_block, render_doc};
pub use nav::{mobile_tab_bar, nav_header, NavItem};
pub use primitives::{
    appearance_panel, back_button, card, dot, due_badge, due_status, filter_pill, list, note,
    placeholder, priority_pill, progress_bar, row, row_for_density, section, sync_status_badge,
    DueStatus,
};
pub use theme::{
    AccentColor, AppearanceSettings, BackgroundTone, Density, GlassStyle, Palette, ThemeMode,
    WindowOpacity,
};
