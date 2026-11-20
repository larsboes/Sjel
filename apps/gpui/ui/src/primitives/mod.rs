//! Shared UI primitives: sections, framed lists, cards, badges, pills, progress bars, and density-aware rows.

pub mod badges;
pub mod cards;
pub mod chrome;
pub mod tokens;

pub use badges::{
    due_badge, due_status, filter_pill, interactive_pill, mono_num, priority_pill, status_pill,
    sync_status_badge, tone_chip, DueStatus,
};
pub use cards::{
    back_button, card, chart_card, dashboard_metric_tile, list, pill_tab, pill_tab_bar,
    progress_bar, quick_action_button, row, row_for_density, section, stat_card,
    stat_card_interactive, stat_strip, surface_card, virtual_list, InteractiveStatCard, QuickAction,
};
pub use chrome::{
    column_width, cycle_column_width, key_bar, shortcut_hint, sub_tabs, tabular_cell, SortOrder,
};
pub use tokens::{dot, note, placeholder, FONT_MONO_TABULAR, RADIUS_LG, RADIUS_MD, RADIUS_SM};

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn back_button_accepts_pointer_and_keyboard_activation(cx: &mut gpui::TestAppContext) {
        use gpui::{div, px, Context, IntoElement, ParentElement, Render, Window};

        struct BackView(usize);

        impl Render for BackView {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div().child(back_button(
                    "Back to projects",
                    cx.listener(|view, _, _, cx| {
                        view.0 += 1;
                        cx.notify();
                    }),
                    crate::theme::Palette::dark(),
                ))
            }
        }

        let (view, cx) = crate::test_support::mount_root(cx, None, |_, _| BackView(0));
        cx.update(|window, cx| window.focus_next(cx));
        cx.simulate_keystrokes("enter");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| assert_eq!(view.0, 1));
        cx.simulate_click(gpui::point(px(8.), px(8.)), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, _| assert_eq!(view.0, 2));
    }

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
    fn sort_order_and_column_width_cycling() {
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
