use std::time::Duration;

use gpui_kit::Action;

use crate::route::Page;

/// How many results the list shows. More than a screenful is a list to scroll, not an answer.
pub const SHOWN: usize = 12;
/// Records older than this are read again when the finder opens.
pub const RECORDS_FRESH: Duration = Duration::from_secs(60);
pub const WIDTH: f32 = 640.;
/// How long typing has to pause before the server is asked: one request per word, not per key.
pub const SERVER_DEBOUNCE: Duration = Duration::from_millis(180);
/// Server results shown at most, below the local ones.
pub const SERVER_SHOWN: usize = 15;

/// Where a result goes. The workspace carries it out, so the finder never touches a page.
#[derive(Clone)]
pub enum Open {
    Page(Page),
    /// An app action, built fresh each time: an action is not `Clone`.
    Action(fn() -> Box<dyn Action>),
    Project(String),
    Record {
        project: String,
        path: String,
    },
    /// A server result's `sjel://` link, opened the way a deep link is.
    Link(String),
    /// A saved web destination, whose local usage counter advances on explicit open.
    WebDestination {
        id: String,
        url: String,
    },
}

/// One result row.
#[derive(Clone)]
pub struct Hit {
    pub kind: &'static str,
    pub title: String,
    pub detail: String,
    pub open: Open,
}

pub enum FinderEvent {
    Open(Open),
    AgentStarted,
    Closed,
}
