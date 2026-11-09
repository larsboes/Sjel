//! Shared GPUI design system, tokens, layout primitives, and navigation for Sjel.
//!
//! Shared across macOS desktop and iOS mobile clients.

// GPUI macros expand to `gpui::` paths. The Kit re-exports the GPUI it is built on.
#[cfg(feature = "gpui")]
extern crate gpui_kit as gpui;

#[cfg(feature = "gpui")]
pub mod accounts;
#[cfg(feature = "gpui")]
pub mod agent_seat;
#[cfg(feature = "gpui")]
pub mod ask;
#[cfg(feature = "gpui")]
pub mod backup;
#[cfg(feature = "gpui")]
pub mod block_menu;
#[cfg(feature = "gpui")]
pub mod calendar;
#[cfg(feature = "gpui")]
pub mod capabilities;
#[cfg(feature = "gpui")]
pub mod clipper;
#[cfg(feature = "gpui")]
pub mod context;
#[cfg(feature = "gpui")]
pub mod cv;
#[cfg(feature = "gpui")]
pub mod data;
#[cfg(feature = "gpui")]
pub mod doc_view;
#[cfg(feature = "gpui")]
pub mod docs;
#[cfg(feature = "gpui")]
pub mod documents;
#[cfg(feature = "gpui")]
pub mod editor;
#[cfg(feature = "gpui")]
pub mod feed;
#[cfg(feature = "gpui")]
pub mod finance;
#[cfg(feature = "gpui")]
pub mod import;
#[cfg(feature = "gpui")]
pub mod inbox;
#[cfg(feature = "gpui")]
pub mod interior;
#[cfg(feature = "gpui")]
pub mod inventory;
#[cfg(feature = "gpui")]
pub mod mail;
#[cfg(feature = "gpui")]
pub mod media;
#[cfg(feature = "gpui")]
pub mod music;
#[cfg(feature = "gpui")]
pub mod nav;
#[cfg(feature = "nsh")]
pub mod nsh_agent;
#[cfg(feature = "nsh")]
pub mod nsh_calendar;
#[cfg(feature = "nsh")]
pub mod nsh_capture;
#[cfg(feature = "nsh")]
pub mod nsh_documents;
#[cfg(feature = "nsh")]
pub mod nsh_edge;
#[cfg(feature = "nsh")]
pub mod nsh_interior;
#[cfg(feature = "nsh")]
pub mod nsh_mail;
#[cfg(feature = "nsh")]
pub mod nsh_map;
#[cfg(feature = "nsh")]
pub mod nsh_people;
#[cfg(feature = "nsh")]
pub mod nsh_projects;
#[cfg(feature = "nsh")]
pub mod nsh_settings;
#[cfg(feature = "nsh")]
pub mod nsh_soundscape;
#[cfg(feature = "nsh")]
pub mod nsh_terminal;
#[cfg(feature = "nsh")]
pub mod nsh_today;
#[cfg(feature = "nsh")]
pub mod nsh_travel;
#[cfg(feature = "nsh")]
mod nsh_ui;
#[cfg(feature = "gpui")]
pub mod packs;
#[cfg(feature = "gpui")]
pub mod pdf_kit;
#[cfg(feature = "gpui")]
pub mod people;
#[cfg(feature = "gpui")]
pub mod primitives;
#[cfg(feature = "gpui")]
pub mod projects;
#[cfg(feature = "gpui")]
pub mod record;
#[cfg(feature = "gpui")]
pub mod research;
#[cfg(feature = "gpui")]
pub mod scout;
#[cfg(feature = "gpui")]
pub mod selection;
#[cfg(feature = "gpui")]
pub mod self_view;
#[cfg(feature = "gpui")]
pub mod setup;
#[cfg(feature = "gpui")]
pub mod sidebar;
#[cfg(feature = "gpui")]
pub mod software;
#[cfg(feature = "gpui")]
pub mod soundscape;
#[cfg(feature = "gpui")]
pub mod systems;
#[cfg(feature = "gpui")]
pub mod tasks;
#[cfg(feature = "gpui")]
pub mod terminal;
mod terminal_screen;
pub mod theme;
#[cfg(feature = "gpui")]
pub mod travel;
#[cfg(feature = "gpui")]
pub mod viz;
#[cfg(feature = "gpui")]
pub mod web_links;

#[cfg(feature = "gpui")]
pub use doc_view::{render_block, render_doc, render_markdown, text_view_style};
#[cfg(feature = "gpui")]
pub use docs::MermaidPlugin;
#[cfg(feature = "gpui")]
pub use nav::{mobile_tab_bar, nav_header, NavItem};
#[cfg(feature = "gpui")]
pub use primitives::{
    back_button, card, column_width, cycle_column_width, data_row, dot, due_badge, due_status,
    filter_pill, key_bar, list, mono_num, note, page_scroll, placeholder, priority_pill,
    progress_bar, remote_state, resizable_table_header, row, row_for_density, section,
    shortcut_hint, sortable_table_header, stat_card, stat_card_interactive, stat_strip, sub_tabs,
    sync_status_badge, table_header, tabular_cell, tone_chip, virtual_list, DueStatus, SortOrder,
    SortableColumn, RADIUS_LG, RADIUS_MD, RADIUS_SM,
};
#[cfg(feature = "gpui")]
pub use sidebar::{sidebar_frame, sidebar_scrim, Seat, Sidebar};
pub use theme::{
    step_terminal_font_size, tint, AccentColor, AppearanceSettings, BackgroundTone, Density,
    GlassStyle, NotifyPrefs, Palette, SidebarPrefs, SoundscapePrefs, TerminalFont, ThemeMode,
    WindowOpacity, TERMINAL_FONT_SIZES,
};

#[cfg(feature = "gpui")]
pub use theme::ThemeContext;
