//! Capture state and the phone's write to the replica outbox.

use std::sync::{Arc, Mutex, OnceLock};

use sjel_gpui_core::capture::{self, Draft};
use sjel_gpui_core::shell;
use sjel_gpui_core::store::{LocalCapture, TursoStore};
use tokio::sync::watch;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) captures: Vec<LocalCapture>,
    pub(crate) refusals: Vec<(String, String)>,
    pub(crate) error: Option<String>,
    pub(crate) saving: bool,
    pub(crate) has_store: bool,
    /// Bumped after each saved capture, so the screen clears its field.
    pub(crate) saved: u64,
}

struct Feed {
    store: Mutex<Option<Arc<TursoStore>>>,
    snapshot: watch::Sender<Snapshot>,
    wrote: Mutex<Option<Box<dyn Fn() + Send>>>,
    generation: Mutex<u64>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| Feed {
        store: Mutex::new(None),
        snapshot: watch::channel(Snapshot::default()).0,
        wrote: Mutex::new(None),
        generation: Mutex::new(0),
    })
}

fn update(change: impl FnOnce(&mut Snapshot)) {
    feed().snapshot.send_modify(change);
}

pub(crate) fn subscribe() -> watch::Receiver<Snapshot> {
    feed().snapshot.subscribe()
}

fn store() -> Option<Arc<TursoStore>> {
    feed()
        .store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// What the host does after a capture lands in the queue: sync, if this launch reached the Mac.
pub fn set_wrote(wrote: impl Fn() + Send + 'static) {
    *feed().wrote.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(wrote));
}

/// Hands the screen the phone's replica, or takes it away, and reads the captures.
pub fn set_store(store: Option<Arc<TursoStore>>) {
    let has_store = store.is_some();
    *feed().store.lock().unwrap_or_else(|e| e.into_inner()) = store;
    update(|snap| snap.has_store = has_store);
    load();
}

/// Reads this phone's captures and the Mac's refusals. A newer read makes an older one stale.
pub fn load() {
    let Some(store) = store() else {
        return;
    };
    let generation = {
        let mut generation = feed().generation.lock().unwrap_or_else(|e| e.into_inner());
        *generation += 1;
        *generation
    };
    shell::spawn(async move {
        let answer = async {
            let captures = store.captures().await.map_err(|e| e.to_string())?;
            let refusals = store.refusals().await.map_err(|e| e.to_string())?;
            Ok::<_, String>((captures, refusals))
        }
        .await;
        if *feed().generation.lock().unwrap_or_else(|e| e.into_inner()) != generation {
            return;
        }
        update(|snap| match answer {
            Ok((captures, refusals)) => {
                snap.captures = captures;
                snap.refusals = refusals;
            }
            Err(error) => snap.error = Some(error),
        });
    });
}

/// Keeps what was typed: written into the replica, then queued for the Mac.
pub(crate) fn keep(typed: String, tag: Option<&'static str>) {
    let draft = match Draft::parse(&typed, tag) {
        Ok(draft) => draft,
        Err(why) => return update(|snap| snap.error = Some(why)),
    };
    let Some(store) = store() else {
        return update(|snap| {
            snap.error =
                Some("Captures cannot be saved on this phone: its store did not open.".to_owned())
        });
    };
    let mut started = false;
    update(|snap| {
        if !snap.saving {
            snap.saving = true;
            snap.error = None;
            started = true;
        }
    });
    if !started {
        return;
    }
    shell::spawn(async move {
        let answer = store
            .save_capture(
                draft.text.as_deref(),
                draft.url.as_deref(),
                draft.tag.as_deref(),
            )
            .await
            .map_err(|e| e.to_string());
        let landed = answer.is_ok();
        update(|snap| {
            snap.saving = false;
            match answer {
                Ok(_) => snap.saved += 1,
                Err(error) => snap.error = Some(error),
            }
        });
        if landed {
            load();
            if let Some(wrote) = feed()
                .wrote
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                wrote();
            }
        }
    });
}

/// What the line under the field says.
pub(crate) fn preview_label(typed: &str, tag: Option<&str>) -> String {
    match Draft::parse(typed, tag) {
        Ok(draft) => draft.preview(),
        Err(_) => capture::route_for(tag, None)
            .map(|route| route.label().to_owned())
            .unwrap_or_else(|| "Stays in the inbox".to_owned()),
    }
}

/// Where a capture is, in the Mac's own terms once it has one.
fn state_of(capture: &LocalCapture) -> &'static str {
    if !capture.on_the_mac() {
        return "Waiting for the Mac";
    }
    match capture.status.as_deref() {
        Some("done") => "Done on the Mac",
        Some("dismissed") => "Dismissed on the Mac",
        _ => "On the Mac",
    }
}

pub(crate) fn headline(capture: &LocalCapture) -> String {
    capture
        .text
        .clone()
        .filter(|t| !t.is_empty())
        .or_else(|| capture.url.clone())
        .unwrap_or_else(|| "(empty)".to_owned())
}

pub(crate) fn second_line(capture: &LocalCapture, refusal: Option<&String>) -> String {
    let mut parts = Vec::new();
    if let Some(tag) = &capture.tag {
        parts.push(format!("#{tag}"));
    }
    if capture.text.as_deref().is_some_and(|t| !t.is_empty()) {
        if let Some(url) = &capture.url {
            parts.push(url.clone());
        }
    }
    parts.push(match refusal {
        Some(reason) => format!("Refused by the Mac: {reason}"),
        None => state_of(capture).to_owned(),
    });
    parts.join(" · ")
}

mod view;
pub use view::PhoneCapture;

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(id: &str, text: &str, remote: Option<&str>, status: Option<&str>) -> LocalCapture {
        LocalCapture {
            id: id.to_owned(),
            text: Some(text.to_owned()),
            url: None,
            tag: Some("buy".to_owned()),
            remote_id: remote.map(str::to_owned),
            created_at: "2026-11-09T10:00:00Z".to_owned(),
            status: status.map(str::to_owned),
            routed_to: None,
            client: None,
        }
    }

    #[test]
    fn a_row_says_whether_the_mac_has_it_and_why_it_refused() {
        let waiting = capture("a", "Rain jacket", None, None);
        assert_eq!(second_line(&waiting, None), "#buy · Waiting for the Mac");
        let done = capture("b", "Shoes", Some("r1"), Some("done"));
        assert_eq!(second_line(&done, None), "#buy · Done on the Mac");
        let reason = "duplicate".to_owned();
        assert_eq!(
            second_line(&waiting, Some(&reason)),
            "#buy · Refused by the Mac: duplicate"
        );
        assert_eq!(preview_label("", None), "Stays in the inbox");
    }
}
