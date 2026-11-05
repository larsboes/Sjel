//! Calendar pages and views: month grid, week view, proposals, and mobile day view.

pub mod actions;
pub mod day;
pub mod grid;
pub mod page;
pub mod proposals;
pub mod types;

pub use actions::*;
pub use day::*;
pub use page::*;
pub use types::*;

/// Backwards-compatibility alias for callers expecting `sjel_gpui_ui::calendar::page::*`.
pub mod page_compat {
    pub use super::*;
}
pub use page_compat as page_alias;
