use gpui_kit::component::input::InputState;
use gpui_kit::Entity;

pub use sjel_gpui_core::route::FeedTab as Tab;

/// How far back the list reads. The dashboard's inbox reads 7 days by default; a native window
/// that stays open longer gets a month.
pub const DAYS: u32 = 30;

/// The library reads the whole archive, as the dashboard's library does.
pub const ARCHIVE_DAYS: u32 = 3650;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    New,
    Kept,
    Dismissed,
}

impl Filter {
    pub fn status(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Kept => "keeper",
            Self::Dismissed => "dismissed",
        }
    }
}

pub(crate) struct CalendarEditForm {
    pub(crate) id: String,
    pub(crate) title: Entity<InputState>,
    pub(crate) start_date: Entity<InputState>,
    pub(crate) end_date: Entity<InputState>,
    pub(crate) start_time: Entity<InputState>,
    pub(crate) end_time: Entity<InputState>,
    pub(crate) location: Entity<InputState>,
    pub(crate) notes: Entity<InputState>,
    pub(crate) all_day: bool,
    pub(crate) kind: String,
    pub(crate) commitment: String,
    pub(crate) saving: bool,
    pub(crate) error: Option<String>,
}

pub(crate) struct DataClassForm {
    pub(crate) target: &'static str,
    pub(crate) rationale: Entity<InputState>,
    pub(crate) error: Option<String>,
}

/// What `reading_order` was built from. The order is rebuilt when any part moves.
pub(crate) type ReadingKey = (u64, usize, Filter, Option<&'static str>);
