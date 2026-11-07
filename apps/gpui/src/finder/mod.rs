//! The finder: one input for a search and a prompt, over whatever page is open (⌘K).
//!
//! It replaces the command palette and keeps everything the palette offered -- pages and
//! actions -- as search results, next to the projects, tasks and notes the Mac's
//! database already holds. The same input also asks: in *Auto* the words decide
//! (`sjel_gpui_core::search::intent`), the decision is printed beside the input, and ⌘↩ goes the
//! other way. A question is answered in place through the app's one `Ask` session, and the whole
//! conversation shows above the results while the text is a question. There is no Ask page.

mod actions;
mod search;
mod types;

use std::time::Instant;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, MouseButton, Render, SharedString, Subscription, Window,
};
use sjel_gpui_core::search::Mode;
use sjel_gpui_ui::agent_seat::{AgentSeat, AgentSeatEvent, Host};
use sjel_gpui_ui::ask::{Ask, Model};
use sjel_gpui_ui::theme::{tint, ThemeContext};

use crate::data::Repo;

pub use actions::*;
pub use types::*;

pub struct Finder {
    pub(super) repo: Repo,
    pub(super) ask: Entity<Ask>,
    /// Pages and actions: what the palette offered.
    pub(super) commands: Vec<Hit>,
    /// Projects, open tasks and notes, read when the finder opens.
    pub(super) records: Vec<Hit>,
    pub(super) records_at: Option<Instant>,
    /// The server's answer, and the text it answered: shown only while that is still the text.
    pub(super) server: Vec<Hit>,
    pub(super) server_query: String,
    /// The text a server request is running for, if one is.
    pub(super) searching: Option<String>,
    pub(super) input: Option<(Entity<InputState>, Subscription)>,
    pub(super) query: String,
    pub(super) mode: Mode,
    pub(super) cursor: usize,
    /// Set when the arrow keys moved the cursor: a result picked by hand is what Enter opens,
    /// whatever the words look like.
    pub(super) picked: bool,
    /// True once a question was sent from here: the conversation shows until it is cleared.
    pub(super) asked: bool,
    /// The agent seat: the run this finder starts or steers, drawn on the right edge.
    pub agent: Entity<AgentSeat>,
    pub(super) thread_scroll: gpui::ScrollHandle,
    pub(super) open: bool,
    pub(super) focus_next: bool,
    pub(super) focus: FocusHandle,
    pub(super) _ask: Subscription,
    _agent: Subscription,
}

impl EventEmitter<FinderEvent> for Finder {}

impl Finder {
    pub fn new(repo: Repo, ask: Entity<Ask>, commands: Vec<Hit>, cx: &mut Context<Self>) -> Self {
        // The answer streams into the Ask session; the finder redraws with it and follows it down.
        let observe = cx.observe(&ask, |view: &mut Self, ask, cx| {
            if ask.read(cx).loading {
                view.thread_scroll.scroll_to_bottom();
            }
            cx.notify();
        });
        let host = repo.shell().ok().map(Host::Shell);
        let agent = cx.new(|_| {
            AgentSeat::new(
                host,
                false,
                "Sjel is not reachable, so the agent cannot start.",
            )
        });
        let agent_events =
            cx.subscribe(&agent, |view, _, event: &AgentSeatEvent, cx| match event {
                AgentSeatEvent::Started => cx.emit(FinderEvent::AgentStarted),
                AgentSeatEvent::NewSession => {
                    view.mode = Mode::Agent;
                    view.open(cx);
                }
            });
        Self {
            repo,
            ask,
            commands,
            records: Vec::new(),
            records_at: None,
            server: Vec::new(),
            server_query: String::new(),
            searching: None,
            input: None,
            query: String::new(),
            mode: Mode::Auto,
            cursor: 0,
            picked: false,
            asked: false,
            agent,
            thread_scroll: gpui::ScrollHandle::new(),
            open: false,
            focus_next: false,
            focus: cx.focus_handle(),
            _ask: observe,
            _agent: agent_events,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        self.open = true;
        self.focus_next = true;
        self.cursor = 0;
        self.picked = false;
        // A finder opens empty: the last text was for the last errand. The last answer stays,
        // so reopening shows what came back while it was closed.
        self.query.clear();
        self.ask.update(cx, |ask, cx| ask.discover_providers(cx));
        if self
            .records_at
            .is_none_or(|at| at.elapsed() > RECORDS_FRESH)
        {
            self.load_records(cx);
        }
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        cx.emit(FinderEvent::Closed);
        cx.notify();
    }

    /// Open the finder and ask `question` in the session. An empty question only opens it.
    pub fn ask(&mut self, question: String, cx: &mut Context<Self>) {
        if !self.open {
            self.open(cx);
        }
        if question.trim().is_empty() {
            return;
        }
        self.ask.update(cx, |ask, cx| ask.submit(question, cx));
        self.asked = true;
        self.thread_scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.close(cx);
        } else {
            self.open(cx);
        }
    }
}

impl Focusable for Finder {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Finder {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let palette = ThemeContext::of(window, cx).palette;
        let input = self.ensure_input(window, cx);
        let resolved = self.resolved();
        let typing = !self.query.trim().is_empty();
        let agent_mode = resolved == Mode::Agent;
        // The conversation shows while the text is empty or is itself a question; a search
        // typed under it gets the panel to itself.
        let thread = if self.asked && !agent_mode && (!typing || resolved == Mode::Ask) {
            self.ask.read(cx).exchanges()
        } else {
            Vec::new()
        };
        let threaded = !thread.is_empty();
        // Under a conversation the page list is noise; typing brings results back.
        let hits: Vec<Hit> = if agent_mode || (threaded && !typing) {
            Vec::new()
        } else {
            self.hits().into_iter().cloned().collect()
        };
        let nothing = hits.is_empty();

        let modes = div().flex().gap_1().children(Mode::ALL.map(|mode| {
            let chosen = mode == self.mode;
            div()
                .id(SharedString::from(format!("finder-mode-{}", mode.label())))
                .px_2()
                .py(px(2.))
                .rounded(px(5.))
                .text_xs()
                .cursor_pointer()
                .bg(rgba(if chosen {
                    tint(palette.accent, 0x2a)
                } else {
                    palette.raised
                }))
                .text_color(rgb(if chosen {
                    palette.accent
                } else {
                    palette.muted
                }))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.mode = mode;
                    view.focus_next = true;
                    view.search_server_soon(cx);
                    if mode == Mode::Agent {
                        view.agent.update(cx, |seat, cx| seat.refresh_sessions(cx));
                    }
                    cx.notify();
                }))
                .child(mode.label())
        }));

        // The decision Auto made, where the eye already is.
        let decision = (self.mode == Mode::Auto && !self.query.trim().is_empty()).then(|| {
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("↩ {}", resolved.label().to_lowercase()))
        });

        let list = div()
            .flex()
            .flex_col()
            .py_1()
            .children(hits.into_iter().enumerate().map(|(index, hit)| {
                // While Enter would ask, no row looks chosen; an arrow key chooses one.
                let selected = index == self.cursor && (resolved == Mode::Search || self.picked);
                let open = hit.open.clone();
                div()
                    .id(SharedString::from(format!("finder-hit-{index}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py(px(6.))
                    .mx_1()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(rgba(palette.raised)))
                    .hover(move |row| row.bg(rgba(palette.raised)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.close(cx);
                        cx.emit(FinderEvent::Open(open.clone()));
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_sm().truncate().child(hit.title))
                            .when(!hit.detail.is_empty(), |cell| {
                                cell.child(
                                    div()
                                        .text_xs()
                                        .truncate()
                                        .text_color(rgb(palette.muted))
                                        .child(hit.detail),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(hit.kind),
                    )
            }));

        // Nothing found says so, and says what Enter does instead of leaving an empty box.
        let empty = (nothing && !threaded && typing && !agent_mode).then(|| {
            let next = if self.searching.is_some() {
                "Searching Sjel…"
            } else if self.mode == Mode::Search {
                "Nothing found. ⌘↩ asks Sjel instead."
            } else {
                "Nothing found. ↩ asks Sjel instead."
            };
            div()
                .px_4()
                .py_3()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(next)
        });

        let (can_stop, loading, model) = {
            let ask = self.ask.read(cx);
            (ask.can_stop(), ask.loading, ask.model())
        };
        let link = move |id: &'static str| {
            div()
                .id(id)
                .cursor_pointer()
                .hover(move |link| link.text_color(rgb(palette.text)))
        };
        let thread = threaded.then(|| {
            div()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .id("finder-thread")
                        .flex()
                        .flex_col()
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .track_scroll(&self.thread_scroll)
                        .children(thread.into_iter().map(|exchange| {
                            let status = match (exchange.loading, exchange.tools.is_empty()) {
                                (true, true) => "working…".to_owned(),
                                (true, false) => format!("working · {}", exchange.tools.join(", ")),
                                (false, true) => String::new(),
                                (false, false) => format!("used {}", exchange.tools.join(", ")),
                            };
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .px_4()
                                .py_2()
                                .when(!exchange.prompt.is_empty(), |card| {
                                    card.child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(exchange.prompt),
                                    )
                                })
                                .when(!exchange.answer.is_empty(), |card| {
                                    card.child(div().text_sm().child(exchange.answer))
                                })
                                .when_some(exchange.error, |card, error| {
                                    card.child(
                                        div().text_sm().text_color(rgb(palette.red)).child(error),
                                    )
                                })
                                .when(!status.is_empty() || exchange.meta.is_some(), |card| {
                                    card.child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(status)
                                            .child(div().flex_1())
                                            .children(exchange.meta),
                                    )
                                })
                        })),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_3()
                        .px_4()
                        .py(px(6.))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .when(can_stop, |row| {
                            row.child(
                                link("finder-stop")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.ask.update(cx, |ask, cx| ask.stop(cx));
                                    }))
                                    .child("stop"),
                            )
                        })
                        .when(!loading, |row| {
                            row.child(
                                link("finder-new")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.ask.update(cx, |ask, cx| ask.clear(cx));
                                        view.asked = false;
                                        view.focus_next = true;
                                        cx.notify();
                                    }))
                                    .child("new conversation"),
                            )
                        }),
                )
        });

        // Where the next question goes. Cloud gets the current prompt only, once it is marked
        // public (c0); the session refuses to send it before that.
        let model = match model {
            Model::Local { error } => link("finder-model")
                .on_click(cx.listener(move |view, _, _, cx| {
                    let retry = error.is_some();
                    view.ask.update(cx, |ask, cx| {
                        if retry {
                            ask.retry_providers(cx);
                        } else {
                            ask.next_model(cx);
                        }
                    });
                }))
                .child("local model")
                .into_any_element(),
            Model::Cloud { label, confirmed } => div()
                .flex()
                .gap_2()
                .child(
                    link("finder-model")
                        .text_color(rgb(palette.accent))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.ask.update(cx, |ask, cx| ask.next_model(cx));
                        }))
                        .child(label),
                )
                .child(
                    link("finder-public")
                        .text_color(rgb(if confirmed {
                            palette.green
                        } else {
                            palette.red
                        }))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.ask.update(cx, |ask, cx| ask.toggle_public(cx));
                        }))
                        .child(if confirmed {
                            "public c0 ✓"
                        } else {
                            "mark public c0"
                        }),
                )
                .into_any_element(),
        };

        let hint = if agent_mode {
            "↩ start or steer · run stays in the right panel"
        } else {
            "↩ open or ask"
        };

        let panel = div()
            .key_context("Finder")
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, _: &SelectUp, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &SelectDown, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &NextMode, _, cx| {
                view.mode = view.mode.next();
                view.search_server_soon(cx);
                if view.mode == Mode::Agent {
                    view.agent.update(cx, |seat, cx| seat.refresh_sessions(cx));
                }
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &Close, _, cx| view.close(cx)))
            // A click inside the panel is not a click on the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(WIDTH))
            .max_h(px(600.))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .overflow_hidden()
            .bg(rgba(palette.panel()))
            .border_1()
            .border_color(rgba(palette.border))
            .shadow(vec![gpui_kit::BoxShadow::new(
                px(0.),
                px(12.),
                gpui_kit::hsla(0., 0., 0., 0.22),
            )
            .blur_radius(px(40.))])
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .child(div().flex_1().child(Input::new(&input).appearance(false)))
                    .children(decision)
                    .child(modes),
            )
            .children(thread)
            .children(empty)
            .when(!agent_mode, |panel| {
                panel.child(
                    // Sized to its rows: two results make a short panel, not a tall empty one.
                    div()
                        .id("finder-list")
                        .max_h(px(420.))
                        .overflow_y_scroll()
                        .child(list),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .px_3()
                    .py(px(6.))
                    .border_t_1()
                    .border_color(rgba(palette.border))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .when_some(self.searching.as_ref(), |footer, _| {
                        footer.child(
                            div()
                                .text_color(rgb(palette.accent))
                                .child("searching Sjel…"),
                        )
                    })
                    .child(hint)
                    .child("⌘↩ the other way")
                    .child("⌘/ mode")
                    .child("esc close")
                    .child(div().flex_1())
                    .child(model),
            );

        div()
            .id("finder-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            // The panel is as tall as its rows: a stretched one is an empty box under them.
            .items_start()
            .pt(px(72.))
            .bg(gpui_kit::hsla(0., 0., 0., 0.18))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.close(cx)),
            )
            .child(panel)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::search::{record_hits, server_hit};
    use super::*;
    use crate::shell::{GlobalNote, GlobalTask, ProjectSummary, ServerHit};

    /// Every link the server sends (`sjel-status/src/status/search.rs`) opens a native page.
    #[test]
    fn every_server_link_opens_a_native_page() {
        use crate::route::{parse_deep_link, Target};
        for link in [
            "sjel://calendar",
            "sjel://people",
            "sjel://documents",
            "sjel://inventory",
            "sjel://finance",
            "sjel://inbox",
        ] {
            assert!(
                matches!(parse_deep_link(link), Some(Target::Native(_))),
                "{link}"
            );
        }
        assert!(matches!(
            parse_deep_link("sjel://travel/t1/overview"),
            Some(Target::Travel(_))
        ));
        let hit = server_hit(ServerHit {
            kind: "mystery".into(),
            title: "x".into(),
            detail: String::new(),
            link: "sjel://inbox".into(),
        });
        assert_eq!(hit.kind, "result");
    }

    #[test]
    fn records_become_results_in_kind_order() {
        use sjel_gpui_core::search;

        let projects =
            vec![
                serde_json::from_str::<ProjectSummary>(r#"{"name":"Atlas","open":2,"notes":3}"#)
                    .unwrap(),
            ];
        let tasks = vec![
            serde_json::from_str::<GlobalTask>(
                r#"{"id":"Atlas/Tasks/a.md","title":"Call Anna","project":"Atlas","due":"2026-10-24"}"#,
            )
            .unwrap(),
            serde_json::from_str::<GlobalTask>(
                r#"{"id":"Atlas/Tasks/b.md","title":"Done thing","project":"Atlas","done":true}"#,
            )
            .unwrap(),
        ];
        let notes = vec![serde_json::from_str::<GlobalNote>(
            r#"{"path":"Atlas/Notes/n.md","title":"Anna notes","project":"Atlas"}"#,
        )
        .unwrap()];
        let links = vec![sjel_gpui_core::web_links::WebLink {
            id: "a".repeat(64),
            url: "https://reddit.com/r/rust".into(),
            title: "Rust community".into(),
            host: "reddit.com".into(),
            tenant: Some("r/rust".into()),
            group: "Reddit".into(),
            ..Default::default()
        }];
        let hits = record_hits(projects, tasks, notes, links);
        let kinds: Vec<&str> = hits.iter().map(|hit| hit.kind).collect();
        assert_eq!(
            kinds,
            ["project", "task", "note", "link"],
            "a done task is not a result"
        );
        assert_eq!(hits[1].detail, "Atlas · due 2026-10-24");
        let refs: Vec<&Hit> = hits.iter().collect();
        let ranked = search::rank("anna", &refs, |hit| (hit.title.as_str(), vec![]));
        let found: Vec<&str> = ranked
            .into_iter()
            .map(|index| refs[index].title.as_str())
            .collect();
        assert_eq!(found, ["Anna notes", "Call Anna"]);
    }
}
