//! Shared GPUI design system, tokens, layout primitives, and navigation for Sjel.
//!
//! Shared across macOS desktop and iOS mobile clients.

pub mod block_menu;
pub mod calendar;
pub mod data;
pub mod doc_view;
pub mod editor;
pub mod finance;
pub mod inbox;
pub mod nav;
pub mod people;
pub mod primitives;
pub mod projects;
pub mod sidebar;
pub mod soundscape;
pub mod tasks;
pub mod theme;
pub mod today;
pub mod travel;

pub use doc_view::{render_block, render_doc};
pub use nav::{mobile_tab_bar, nav_header, NavItem};
pub use primitives::{
    back_button, card, dot, due_badge, due_status, filter_pill, list, note, placeholder,
    priority_pill, progress_bar, row, row_for_density, section, sync_status_badge, DueStatus,
};
pub use sidebar::{sidebar_frame, sidebar_scrim, Seat, Sidebar};
pub use theme::{
    tint, AccentColor, AppearanceSettings, BackgroundTone, Density, GlassStyle, NotifyPrefs,
    Palette, SidebarPrefs, SoundscapePrefs, ThemeMode, WindowOpacity,
};
