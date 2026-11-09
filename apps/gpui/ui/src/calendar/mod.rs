//! The Mac's Calendar pages: month grid, week view and proposals. The phone's day view is NSH (`crate::nsh_calendar`).

pub mod actions;
pub mod grid;
pub mod page;
pub mod proposals;
pub mod types;

pub use actions::*;
pub use page::*;
pub use types::*;

fn minute_clock<V: 'static>(cx: &mut gpui::Context<V>) -> gpui::Task<()> {
    cx.spawn(async move |view, cx| loop {
        cx.background_executor()
            .timer(std::time::Duration::from_secs(60))
            .await;
        if view.update(cx, |_, cx| cx.notify()).is_err() {
            break;
        }
    })
}

/// Backwards-compatibility alias for callers expecting `sjel_gpui_ui::calendar::page::*`.
pub mod page_compat {
    pub use super::*;
}
pub use page_compat as page_alias;
