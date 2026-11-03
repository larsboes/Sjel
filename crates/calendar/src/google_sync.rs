//! Phase E's impure half: the credential, the Google Calendar API calls, and
//! the two runs that tie them to the store.
//!
//! Every *decision* lives in `google.rs` and is unit-tested against recorded
//! payloads with no token and no socket. What is left here is transport and
//! sequencing, deliberately thin.
//!
//! **Credentials.** Read from a plain `KEY=value` file in the private overlay,
//! the same shape and the same three keys `capabilities/comms` uses:
//! `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GOOGLE_REFRESH_TOKEN`. No
//! value is ever read from this repo, and none is ever logged — not in an
//! error, not in a report, not in a failed-refresh body (Google puts token
//! material in some of those).
//!
//! **When the credential is absent, this fails loudly.** Not a no-op, not an
//! empty result, not a placeholder event: a named error saying which key is
//! missing, which file it belongs in, and which setup step produces it. A
//! silent import that quietly does nothing is the failure mode that makes an
//! operator trust an empty calendar.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Config, GoogleConfig};
use crate::date;
use crate::google::{self, Action, GoogleEvent};
use crate::model::{Entry, ExportOptIn, NewEntry};
use crate::store::CalendarStore;
use crate::zone::HomeTimezone;

mod auth;
mod export;
mod import;
mod transport;

pub use auth::{access_token, Settings};
pub use export::{export, exportable, ExportOutcome, ExportReport};
pub use import::{
    import, import_selected, import_window, preview, ImportCandidate, ImportOutcome, ImportPreview,
    ImportReport, ReviewStatus, SelectedGoogleEvent, SkippedEvent,
};
pub use transport::{CalendarApi, ExportStore, HttpCalendarApi, ImportStore};

#[cfg(test)]
mod test_suite;
