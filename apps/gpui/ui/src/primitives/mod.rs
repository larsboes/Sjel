//! Shared UI primitives: sections, framed lists, cards, badges, pills, progress bars, and density-aware rows.

pub mod badges;
pub mod cards;
pub mod chrome;
pub mod tokens;

pub use badges::{
    due_badge, due_status, filter_pill, mono_num, priority_pill, status_pill, sync_status_badge,
    tone_chip, DueStatus,
};
pub use cards::{
    back_button, card, list, progress_bar, row, row_for_density, section, stat_card,
    stat_card_interactive, stat_strip, virtual_list, InteractiveStatCard,
};
pub use chrome::{
    column_width, cycle_column_width, key_bar, shortcut_hint, sub_tabs, tabular_cell, SortOrder,
};
pub use tokens::{dot, note, placeholder, FONT_MONO_TABULAR, RADIUS_LG, RADIUS_MD, RADIUS_SM};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_due_status_logic() {
        assert_eq!(due_status(None, false, "2026-11-05"), DueStatus::None);
        assert_eq!(due_status(Some(""), false, "2026-11-05"), DueStatus::None);
        assert_eq!(
            due_status(Some("2026-11-04"), true, "2026-11-05"),
            DueStatus::Upcoming("2026-11-04".to_string())
        );
        assert_eq!(
            due_status(Some("2026-11-04"), false, "2026-11-05"),
            DueStatus::Overdue("2026-11-04".to_string())
        );
        assert_eq!(
            due_status(Some("2026-11-05"), false, "2026-11-05"),
            DueStatus::Today
        );
        assert_eq!(
            due_status(Some("2026-11-06"), false, "2026-11-05"),
            DueStatus::Tomorrow
        );
        assert_eq!(
            due_status(Some("2026-11-10"), false, "2026-11-05"),
            DueStatus::Upcoming("2026-11-10".to_string())
        );
    }

    #[test]
    fn test_tokens_dimensions() {
        assert!(RADIUS_SM < RADIUS_MD);
        assert!(RADIUS_MD < RADIUS_LG);
        assert_eq!(SortOrder::Ascending.glyph(), " ↑");
        assert_eq!(SortOrder::Descending.glyph(), " ↓");
        assert_eq!(SortOrder::Ascending.flip(), SortOrder::Descending);
        assert_eq!(SortOrder::from_bool(true), SortOrder::Ascending);
        let mut widths = std::collections::HashMap::new();
        assert_eq!(column_width(&widths, "due", 100.), 100.);
        assert_eq!(cycle_column_width(&mut widths, "due", 100.), 135.);
        assert_eq!(cycle_column_width(&mut widths, "due", 100.), 170.);
        assert_eq!(cycle_column_width(&mut widths, "due", 100.), 100.);
        assert!(widths.is_empty());
    }
}
