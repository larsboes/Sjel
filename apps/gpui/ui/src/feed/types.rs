use crate::ely::forms::TextInput;
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
    pub(crate) title: Entity<TextInput>,
    pub(crate) start_date: Entity<TextInput>,
    pub(crate) end_date: Entity<TextInput>,
    pub(crate) start_time: Entity<TextInput>,
    pub(crate) end_time: Entity<TextInput>,
    pub(crate) location: Entity<TextInput>,
    pub(crate) notes: Entity<TextInput>,
    pub(crate) all_day: bool,
    pub(crate) kind: String,
    pub(crate) commitment: String,
    pub(crate) saving: bool,
    pub(crate) error: Option<String>,
}

pub(crate) struct DataClassForm {
    pub(crate) target: &'static str,
    pub(crate) rationale: Entity<TextInput>,
    pub(crate) error: Option<String>,
}

/// What `reading_order` was built from. The order is rebuilt when any part moves.
pub(crate) type ReadingKey = (u64, usize, Filter, Option<&'static str>);
