//! Capture, drawn by NSH (`docs/plans/gpui.md` §11): one field for the thing in your hand, and
//! the list of what this phone already kept.
//!
//! A capture is written into the phone's replica first and queued for the Mac second, so it works
//! with no Mac in reach. Each row says which of the two it is. Where the Mac routes a capture is
//! `crates/comms`'s decision; the line under the field only says where it will land.

use std::sync::{Arc, Mutex, OnceLock};

use nsh_base::{button, input};
use nsh_core::prelude::*;
use sjel_gpui_core::capture::{self, Draft, TAGS};
use sjel_gpui_core::shell;
use sjel_gpui_core::store::{LocalCapture, TursoStore};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

#[derive(Clone, Debug, Default, PartialEq)]
struct Snapshot {
    captures: Vec<LocalCapture>,
    refusals: Vec<(String, String)>,
    error: Option<String>,
    saving: bool,
    has_store: bool,
    /// Bumped after each saved capture, so the screen clears its field.
    saved: u64,
}

struct Feed {
    store: Mutex<Option<Arc<TursoStore>>>,
    snapshot: Mutex<Snapshot>,
    tx: Mutex<Option<UnboundedSender<Snapshot>>>,
    wrote: Mutex<Option<Box<dyn Fn() + Send>>>,
    generation: Mutex<u64>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| Feed {
        store: Mutex::new(None),
        snapshot: Mutex::new(Snapshot::default()),
        tx: Mutex::new(None),
        wrote: Mutex::new(None),
        generation: Mutex::new(0),
    })
}

fn update(change: impl FnOnce(&mut Snapshot)) {
    let snap = {
        let mut snap = feed().snapshot.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut snap);
        snap.clone()
    };
    if let Some(tx) = feed().tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(snap);
    }
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
fn keep(typed: String, tag: Option<&'static str>) {
    if feed()
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .saving
    {
        return;
    }
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
    update(|snap| {
        snap.saving = true;
        snap.error = None;
    });
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
fn preview_label(typed: &str, tag: Option<&str>) -> String {
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

fn headline(capture: &LocalCapture) -> String {
    capture
        .text
        .clone()
        .filter(|t| !t.is_empty())
        .or_else(|| capture.url.clone())
        .unwrap_or_else(|| "(empty)".to_owned())
}

fn second_line(capture: &LocalCapture, refusal: Option<&String>) -> String {
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

fn muted() -> AdaptiveColor {
    nsh_base::theme().muted_foreground
}

fn capture_row(capture: LocalCapture, refusal: Option<String>) -> impl IntoElement {
    let refused = refusal.is_some();
    view()
        .px_4()
        .py_3()
        .gap_1()
        .border_b(px(1.0))
        .border_b_color(move || nsh_base::theme().border)
        .child(text(headline(&capture)).text_base())
        .child(
            text(second_line(&capture, refusal.as_ref()))
                .text_xs()
                .text_color(move || {
                    if refused {
                        nsh_base::theme().destructive
                    } else {
                        muted()
                    }
                }),
        )
}

#[component]
pub fn CaptureScreen() -> impl IntoElement {
    let initial = feed()
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let seen_saves = initial.saved;
    let snap = signal(initial);
    let draft = signal(String::new());
    let tag = signal(None::<&'static str>);
    let last_save = signal(seen_saves);
    let (tx, mut rx) = unbounded_channel::<Snapshot>();
    *feed().tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    spawn(async move {
        while let Some(next) = rx.recv().await {
            // A new thought is not a correction of the last one: the field and chip clear.
            if next.saved != last_save.get() {
                last_save.set(next.saved);
                draft.set(String::new());
                tag.set(None);
            }
            snap.set(next);
        }
    });
    load();
    let submit = move || keep(draft.get(), tag.get());

    let mut chips = view().flex_row().gap_2();
    for name in TAGS {
        chips = chips.child(
            button()
                .px_3()
                .py_2()
                .rounded_lg()
                .border(px(1.0))
                .border_color(move || nsh_base::theme().border)
                .bg(move || {
                    if tag.get() == Some(name) {
                        nsh_base::theme().primary
                    } else {
                        nsh_base::theme().background
                    }
                })
                .on_press(move || {
                    tag.update(|t| *t = if *t == Some(name) { None } else { Some(name) })
                })
                .child(text(format!("#{name}")).text_xs().text_color(move || {
                    if tag.get() == Some(name) {
                        nsh_base::theme().primary_foreground
                    } else {
                        nsh_base::theme().foreground
                    }
                })),
        );
    }

    view()
        .flex_1()
        .child(
            view()
                .px_4()
                .pt_4()
                .pb_2()
                .gap_2()
                .child(text("Capture").text_2xl().font_weight(FontWeight::SEMIBOLD))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(
                            input()
                                .value(move || draft.get())
                                .on_change(move |value: &str| draft.set(value.to_owned()))
                                .on_submit(move |_: &str| submit())
                                .placeholder("Note, link or task…")
                                .flex_1()
                                .px_3()
                                .py_2()
                                .border(px(1.0))
                                .border_color(move || nsh_base::theme().input)
                                .rounded_lg(),
                        )
                        .child(
                            button()
                                .px_3()
                                .py_2()
                                .rounded_lg()
                                .bg(move || nsh_base::theme().primary)
                                .on_press(submit)
                                .child(
                                    text(move || {
                                        if snap.with(|s| s.saving) {
                                            "Saving…"
                                        } else {
                                            "Keep"
                                        }
                                    })
                                    .text_sm()
                                    .text_color(move || nsh_base::theme().primary_foreground),
                                ),
                        ),
                )
                .child(chips)
                .child(
                    text(move || preview_label(&draft.get(), tag.get()))
                        .text_xs()
                        .text_color(move || muted()),
                )
                .child(dynamic(move || {
                    snap.with(|s| s.error.clone()).map(|error| {
                        text(error)
                            .text_xs()
                            .text_color(move || nsh_base::theme().destructive)
                            .into_element()
                    })
                })),
        )
        .child(dynamic(move || {
            let (captures, refusals) = snap.with(|s| (s.captures.clone(), s.refusals.clone()));
            if captures.is_empty() {
                return view()
                    .px_4()
                    .py_3()
                    .child(
                        text("Nothing captured on this phone yet.")
                            .text_sm()
                            .text_color(move || muted()),
                    )
                    .into_element();
            }
            let waiting = captures.iter().filter(|c| !c.on_the_mac()).count();
            let mut list = view().child(
                text(match waiting {
                    0 => "Everything here is on the Mac.".to_owned(),
                    1 => "1 waiting for the Mac".to_owned(),
                    n => format!("{n} waiting for the Mac"),
                })
                .text_xs()
                .px_4()
                .pb_2()
                .text_color(move || muted()),
            );
            for capture in captures {
                let refusal = refusals
                    .iter()
                    .find(|(id, _)| id == &capture.id)
                    .map(|(_, reason)| reason.clone());
                list = list.child(capture_row(capture, refusal));
            }
            scroll().flex_1().child(list).into_element()
        }))
}

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

    #[test]
    fn the_screen_lists_captures_and_asks_for_a_store_to_keep_one() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;

        update(|snap| {
            snap.captures = vec![capture("a", "Rain jacket", None, None)];
            snap.refusals.clear();
        });
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<CaptureScreen>())
        })
        .expect("the Capture screen starts");
        app.wait_for("text:Rain jacket").unwrap();
        app.wait_for("text:1 waiting for the Mac").unwrap();
        app.press("text:Keep").unwrap();
        app.wait_for("text:Type what you want to keep, or paste a link.")
            .unwrap();
    }
}
