//! Shared GPUI design system, tokens, layout primitives, and navigation for Sjel.
//!
//! Shared across macOS desktop and iOS mobile clients.

// GPUI macros expand to `gpui::` paths. The Kit re-exports the GPUI it is built on.
extern crate gpui_kit as gpui;

pub mod accounts;
pub mod agent_seat;
pub mod ask;
pub mod backup;
pub mod block_menu;
pub mod calendar;
pub mod capabilities;
pub mod clipper;
pub mod context;
pub mod cv;
pub mod data;
pub mod doc_view;
pub mod docs;
pub mod documents;
pub mod editor;
pub mod feed;
pub mod finance;
pub mod import;
pub mod inbox;
pub mod interior;
pub mod inventory;
pub mod mail;
pub mod media;
pub mod music;
pub mod nav;
#[cfg(feature = "nsh")]
pub mod nsh_documents;
#[cfg(feature = "nsh")]
pub mod nsh_mail;
#[cfg(feature = "nsh")]
pub mod nsh_today;
#[cfg(feature = "nsh")]
pub mod nsh_capture;
pub mod packs;
pub mod pdf_kit;
pub mod people;
pub mod primitives;
pub mod projects;
pub mod record;
pub mod research;
pub mod scout;
pub mod selection;
pub mod self_view;
pub mod setup;
pub mod sidebar;
pub mod software;
pub mod soundscape;
pub mod systems;
pub mod tasks;
pub mod terminal;
pub mod theme;
pub mod travel;
pub mod viz;
pub mod web_links;

pub use doc_view::{render_block, render_doc, render_markdown, text_view_style};
pub use docs::MermaidPlugin;
pub use nav::{mobile_tab_bar, nav_header, NavItem};
pub use primitives::{
    back_button, card, column_width, cycle_column_width, data_row, dot, due_badge, due_status,
    filter_pill, key_bar, list, mono_num, note, page_scroll, placeholder, priority_pill,
    progress_bar, remote_state, resizable_table_header, row, row_for_density, section,
    shortcut_hint, sortable_table_header, stat_card, stat_card_interactive, stat_strip, sub_tabs,
    sync_status_badge, table_header, tabular_cell, tone_chip, virtual_list, DueStatus, SortOrder,
    SortableColumn, RADIUS_LG, RADIUS_MD, RADIUS_SM,
};
pub use sidebar::{sidebar_frame, sidebar_scrim, Seat, Sidebar};
pub use theme::{
    step_terminal_font_size, tint, AccentColor, AppearanceSettings, BackgroundTone, Density,
    GlassStyle, NotifyPrefs, Palette, SidebarPrefs, SoundscapePrefs, TerminalFont, ThemeContext,
    ThemeMode, WindowOpacity, TERMINAL_FONT_SIZES,
};
