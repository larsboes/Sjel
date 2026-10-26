//! Shared GPUI design system, tokens, layout primitives, and navigation for Sjel.
//!
//! Shared across macOS desktop and iOS mobile clients.

pub mod apps;
pub mod backup;
pub mod block_menu;
pub mod calendar;
pub mod cv;
pub mod data;
pub mod doc_view;
pub mod editor;
pub mod finance;
pub mod inbox;
pub mod interior;
pub mod inventory;
pub mod mail;
pub mod media;
pub mod nav;
pub mod packs;
pub mod people;
pub mod primitives;
pub mod projects;
pub mod record;
pub mod research;
pub mod self_view;
pub mod setup;
pub mod sidebar;
pub mod software;
pub mod soundscape;
pub mod systems;
pub mod tasks;
pub mod terminal;
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
    step_terminal_font_size, tint, AccentColor, AppearanceSettings, BackgroundTone, Density,
    GlassStyle, NotifyPrefs, Palette, SidebarPrefs, SoundscapePrefs, TerminalFont, ThemeMode,
    WindowOpacity, TERMINAL_FONT_SIZES,
};
