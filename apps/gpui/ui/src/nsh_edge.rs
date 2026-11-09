//! One NSH side panel for agent, terminal, soundscape and activity.
use crate::{
    nsh_documents::{self, NshPage},
    nsh_ui::{action, caption, heading, snapshot},
};
use nsh_core::prelude::*;
use std::sync::{Mutex, OnceLock};
use tokio::sync::watch;

/// The side panel's native seats.
pub use sjel_gpui_core::panel::Seat;
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Activity {
    pub connection: Vec<String>,
    pub reminders: Vec<String>,
}
#[derive(Clone)]
struct Snapshot {
    seat: Seat,
    previous: NshPage,
    activity: Activity,
}
type SeatHost = Box<dyn Fn(Seat) + Send>;
struct Feed {
    state: Mutex<Snapshot>,
    tx: watch::Sender<Snapshot>,
    host: Mutex<Option<SeatHost>>,
}
fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let value = Snapshot {
            seat: Seat::Agent,
            previous: NshPage::Today,
            activity: Activity::default(),
        };
        let (tx, _) = watch::channel(value.clone());
        Feed {
            state: Mutex::new(value),
            tx,
            host: Mutex::new(None),
        }
    })
}
/// Registers native audio and authentication work needed on entering a seat.
pub fn set_host(host: impl Fn(Seat) + Send + 'static) {
    *feed().host.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(host));
}
/// Whether the visible panel seat is the terminal.
pub fn is_terminal() -> bool {
    feed().state.lock().unwrap_or_else(|e| e.into_inner()).seat == Seat::Terminal
}
/// Opens a seat and remembers the underlying page until the panel closes.
pub fn open(seat: Seat) {
    {
        let mut state = feed().state.lock().unwrap_or_else(|e| e.into_inner());
        if nsh_documents::nsh_page() != NshPage::Edge {
            state.previous = nsh_documents::nsh_page();
        }
        state.seat = seat;
        feed().tx.send_replace(state.clone());
    }
    crate::nsh_soundscape::show(seat == Seat::Soundscape);
    if seat == Seat::Agent {
        crate::nsh_agent::refresh();
    }
    if let Some(host) = feed()
        .host
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        host(seat);
    }
    nsh_documents::set_nsh_page(NshPage::Edge);
    save(true, seat);
}
/// Closes the panel and pauses audio; changing seats leaves the sound playing.
pub fn close() {
    crate::nsh_soundscape::show(false);
    crate::nsh_soundscape::pause();
    let previous = feed()
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .previous;
    if previous == NshPage::Map {
        crate::nsh_map::show(None);
    } else {
        nsh_documents::set_nsh_page(previous);
    }
}
/// Stops seat polling and playback whenever navigation leaves the panel.
pub fn leaving() {
    crate::nsh_soundscape::show(false);
    crate::nsh_soundscape::pause();
    let seat = feed().state.lock().unwrap_or_else(|e| e.into_inner()).seat;
    save(false, seat);
}
fn save(open: bool, seat: Seat) {
    static WRITER: OnceLock<watch::Sender<Option<(bool, Seat)>>> = OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let (tx, mut rx) = watch::channel(None::<(bool, Seat)>);
        sjel_gpui_core::shell::spawn(async move {
            while rx.changed().await.is_ok() {
                let Some((open, seat)) = *rx.borrow_and_update() else {
                    continue;
                };
                let result = tokio::task::spawn_blocking(move || {
                    let mut prefs = crate::SidebarPrefs::load();
                    prefs.open = open;
                    prefs.seat = seat.key().into();
                    prefs.save()
                })
                .await;
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(e),
                    Err(e) => Some(e.to_string()),
                };
                if let Some(error) = error {
                    let mut state = feed().state.lock().unwrap_or_else(|e| e.into_inner());
                    state
                        .activity
                        .connection
                        .push(format!("Panel preferences were not saved: {error}"));
                    feed().tx.send_replace(state.clone());
                }
            }
        });
        tx
    });
    writer.send_replace(Some((open, seat)));
}

/// Updates the panel's connection, queue and notification summaries.
pub fn set_activity(activity: Activity) {
    let mut state = feed().state.lock().unwrap_or_else(|e| e.into_inner());
    state.activity = activity;
    feed().tx.send_replace(state.clone());
}
/// The shared side panel, with native text controls on every seat.
#[component]
pub fn EdgeScreen() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let seat = computed(move || snap.with(|s| s.seat));
    let mut tabs = view().flex_row().flex_wrap();
    for value in Seat::ALL {
        tabs = tabs.child(action(value.label(), move || open(value)));
    }
    view()
        .flex_1()
        .child(
            view()
                .flex_row()
                .child(tabs.flex_1())
                .child(action("Close panel", close)),
        )
        .child(view().flex_1().child(dynamic(move || {
            match seat.get() {
                Seat::Agent => crate::nsh_agent::agent_screen().into_element(),
                Seat::Terminal => crate::nsh_terminal::terminal_screen().into_element(),
                Seat::Soundscape => crate::nsh_soundscape::soundscape_screen().into_element(),
                Seat::Activity => scroll()
                    .flex_1()
                    .px_4()
                    .child(dynamic(move || {
                        let activity = snap.with(|s| s.activity.clone());
                        let mut out = view()
                            .gap_2()
                            .child(heading("Activity"))
                            .child(heading("With the Mac"));
                        for line in activity.connection {
                            out = out.child(caption(line));
                        }
                        out = out.child(heading("Reminders"));
                        for line in activity.reminders {
                            out = out.child(caption(line));
                        }
                        out
                    }))
                    .into_element(),
            }
        })))
}
