pub use sjel_gpui_core::shell::{status_closes, TASK_STATUSES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskFilter {
    All,
    Open,
    Done,
}

impl TaskFilter {
    pub fn matches(self, done: bool) -> bool {
        match self {
            Self::All => true,
            Self::Open => !done,
            Self::Done => done,
        }
    }
}

/// Which statuses the cross-project table shows. `Active` hides closed tasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusFilter {
    Active,
    All,
    Is(&'static str),
}

impl StatusFilter {
    pub fn matches(self, status: &str) -> bool {
        match self {
            Self::Active => !status_closes(status),
            Self::All => true,
            Self::Is(wanted) => wanted == status,
        }
    }
}

/// The label a status gets on screen. The stored value stays the frontmatter word.
pub fn status_label(status: &str) -> &'static str {
    match status {
        "doing" => "In progress",
        "waiting" => "Waiting",
        "blocked" => "Blocked",
        "done" => "Done",
        "cancelled" => "Cancelled",
        _ => "Open",
    }
}

/// A column the cross-project table sorts by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskSort {
    /// Open first, then by due date with undated last, then by priority: the capability's order.
    Default,
    Title,
    Project,
    Status,
    Due,
    Priority,
}

/// A column the cross-project notes table sorts by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteSort {
    /// Project hubs first, then alphabetical by project and title.
    Default,
    Title,
    Project,
    Kind,
}
