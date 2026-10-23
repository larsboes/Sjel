//! The finder: one input for a search and a prompt, over whatever page is open (⌘K).
//!
//! It replaces the command palette and keeps everything the palette offered -- pages, actions,
//! dashboard destinations -- as search results, next to the projects, tasks and notes the Mac's
//! database already holds. The same input also asks: in *Auto* the words decide
//! (`sjel_gpui_core::search::intent`), the decision is printed beside the input, and ⌘↩ goes the
//! other way. A question is answered inline through the app's one `Ask` session, so ⌘2 shows the
//! same conversation with its history.
//!
//! Decided 2026-10-23 (`PORTING_PLAN.md`, section 3): an overlay first; the docked right panel
//! comes with the adaptive UI. Server search (`/api/search`) is the next step and appends below
//! the local results.
//!
//! The server half (`GET /api/sjel-status/search`) runs [`SERVER_DEBOUNCE`] after the last
//! keystroke and appends below the local results, which never wait for it.

use std::time::{Duration, Instant};

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, Action, App, Context, Entity, EventEmitter,
    FocusHandle, Focusable, IntoElement, MouseButton, Render, SharedString, Subscription, Window,
};
use sjel_gpui_core::search::{self, Mode};

use crate::ask::Ask;
use crate::data::Repo;
use crate::route::Page;
use crate::shell::{self, GlobalNote, GlobalTask, ProjectSummary, ServerHit};
use crate::theme::Palette;

actions!(finder, [SelectUp, SelectDown, NextMode, Close]);

/// How many results the list shows. More than a screenful is a list to scroll, not an answer.
const SHOWN: usize = 12;
/// Records older than this are read again when the finder opens.
const RECORDS_FRESH: Duration = Duration::from_secs(60);
const WIDTH: f32 = 640.;
/// How long typing has to pause before the server is asked: one request per word, not per key.
const SERVER_DEBOUNCE: Duration = Duration::from_millis(180);
/// Server results shown at most, below the local ones.
const SERVER_SHOWN: usize = 15;

/// Where a result goes. The workspace carries it out, so the finder never touches a page.
#[derive(Clone)]
pub enum Open {
    Page(Page),
    /// An app action, built fresh each time: an action is not `Clone`.
    Action(fn() -> Box<dyn Action>),
    Dashboard(&'static str),
    Project(String),
    Record {
        project: String,
        path: String,
    },
    /// A server result's `sjel://` link, opened the way a deep link is.
    Link(String),
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
    /// Show the whole conversation on the Ask page.
    ContinueInAsk,
    Closed,
}

pub struct Finder {
    repo: Repo,
    ask: Entity<Ask>,
    /// Pages, actions and dashboard destinations: what the palette offered.
    commands: Vec<Hit>,
    /// Projects, open tasks and notes, read when the finder opens.
    records: Vec<Hit>,
    records_at: Option<Instant>,
    /// The server's answer, and the text it answered: shown only while that is still the text.
    server: Vec<Hit>,
    server_query: String,
    /// The text a server request is running for, if one is.
    searching: Option<String>,
    input: Option<(Entity<InputState>, Subscription)>,
    query: String,
    mode: Mode,
    cursor: usize,
    /// Set when the arrow keys moved the cursor: a result picked by hand is what Enter opens,
    /// whatever the words look like.
    picked: bool,
    /// True once a question was sent from here: the answer shows until the next text is typed.
    asked: bool,
    open: bool,
    focus_next: bool,
    focus: FocusHandle,
    _ask: Subscription,
}

impl EventEmitter<FinderEvent> for Finder {}

impl Finder {
    pub fn new(repo: Repo, ask: Entity<Ask>, commands: Vec<Hit>, cx: &mut Context<Self>) -> Self {
        // The answer streams into the Ask session; the finder redraws with it.
        let observe = cx.observe(&ask, |_, _, cx| cx.notify());
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
            open: false,
            focus_next: false,
            focus: cx.focus_handle(),
            _ask: observe,
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

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.close(cx);
        } else {
            self.open(cx);
        }
    }

    /// Projects, open tasks and notes. Each read lands on its own, so a slow one does not hold
    /// back the others, and a failed one leaves the finder with what it has.
    fn load_records(&mut self, cx: &mut Context<Self>) {
        self.records_at = Some(Instant::now());
        let repo = self.repo.clone();
        let reads = shell::run(async move {
            let projects = repo.fetch_projects().await.unwrap_or_default();
            let tasks = repo.fetch_tasks(None).await.unwrap_or_default();
            let notes = repo.fetch_all_notes().await.unwrap_or_default();
            (projects, tasks, notes)
        });
        cx.spawn(async move |this, cx| {
            let Some((projects, tasks, notes)) = reads.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                view.records = record_hits(projects, tasks, notes);
                cx.notify();
            });
        })
        .detach();
    }

    /// What the list shows for the current text: the pages when nothing is typed, the best
    /// matches otherwise.
    pub fn hits(&self) -> Vec<&Hit> {
        if self.query.trim().is_empty() {
            return self
                .commands
                .iter()
                .filter(|hit| matches!(hit.open, Open::Page(_)))
                .take(SHOWN)
                .collect();
        }
        let all: Vec<&Hit> = self.commands.iter().chain(self.records.iter()).collect();
        let local = search::rank(&self.query, &all, |hit| {
            (hit.title.as_str(), vec![hit.detail.as_str(), hit.kind])
        })
        .into_iter()
        .take(SHOWN)
        .map(|index| all[index]);
        // The server's rows come after, in its own source order, and only for the text they
        // answer: a late answer to an older text is dropped rather than shown under a new one.
        let server = self
            .server
            .iter()
            .filter(|_| self.server_query == self.query.trim())
            .take(SERVER_SHOWN);
        local.chain(server).collect()
    }

    /// Ask the server for the current text once typing pauses. Ask mode asks; it does not search.
    fn search_server_soon(&mut self, cx: &mut Context<Self>) {
        let text = self.query.trim().to_owned();
        if text.is_empty() || self.mode == Mode::Ask || text == self.server_query {
            return;
        }
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SERVER_DEBOUNCE).await;
            // Still the text on screen? A later keystroke has its own request coming.
            let current = this
                .update(cx, |view, cx| {
                    let current = view.query.trim() == text;
                    if current {
                        view.searching = Some(text.clone());
                        cx.notify();
                    }
                    current
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let asked = text.clone();
            let answer = shell::run(async move { shell.search(&asked).await }).await;
            let _ = this.update(cx, |view, cx| {
                if view.searching.as_deref() == Some(text.as_str()) {
                    view.searching = None;
                }
                // A failed search leaves the local results alone.
                if let Some(Ok(hits)) = answer {
                    view.server = hits.into_iter().map(server_hit).collect();
                    view.server_query = text;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Which way Enter goes for the current text.
    pub fn resolved(&self) -> Mode {
        self.mode.resolve(&self.query)
    }

    fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let input = match &self.input {
            Some((input, _)) => input.clone(),
            None => {
                let input = cx.new(|cx| {
                    InputState::new(window, cx).placeholder("Search Sjel, or ask it something…")
                });
                let subscription = cx.subscribe_in(
                    &input,
                    window,
                    |view, input, event, window, cx| match event {
                        InputEvent::Change => {
                            view.query = input.read(cx).value().to_string();
                            view.cursor = 0;
                            view.picked = false;
                            if !view.query.trim().is_empty() {
                                view.asked = false;
                            }
                            view.search_server_soon(cx);
                            cx.notify();
                        }
                        InputEvent::PressEnter { secondary, .. } => {
                            view.confirm(*secondary, window, cx);
                        }
                        _ => {}
                    },
                );
                self.input = Some((input.clone(), subscription));
                input
            }
        };
        if std::mem::take(&mut self.focus_next) {
            let empty = self.query.is_empty();
            input.update(cx, |input, cx| {
                if empty && !input.value().is_empty() {
                    input.set_value("", window, cx);
                }
                input.focus(window, cx);
            });
        }
        input
    }

    /// Enter: open the selected result, or ask. `other_way` is ⌘↩, which flips the decision.
    ///
    /// In *Auto* a search that finds nothing asks instead: an empty list is never the answer to a
    /// text someone pressed Enter on.
    fn confirm(&mut self, other_way: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.query.trim().to_owned();
        // Under an answer an empty Enter is not a request to open the first page.
        if text.is_empty() && self.asked {
            return;
        }
        let mut ask = self.resolved() == Mode::Ask && !self.picked;
        if other_way {
            ask = !ask;
        }
        let hit = self.hits().get(self.cursor).map(|hit| hit.open.clone());
        if !ask && hit.is_none() && self.mode == Mode::Auto && !text.is_empty() {
            ask = true;
        }
        if ask {
            if text.is_empty() {
                return;
            }
            self.ask.update(cx, |ask, cx| ask.submit(text, cx));
            self.asked = true;
            self.query.clear();
            if let Some((input, _)) = &self.input {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            cx.notify();
            return;
        }
        if let Some(open) = hit {
            self.close(cx);
            cx.emit(FinderEvent::Open(open));
        }
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.hits().len();
        if count == 0 {
            return;
        }
        // The first arrow press picks the row the cursor is on; later ones move it.
        if self.picked {
            self.cursor = (self.cursor as isize + by).rem_euclid(count as isize) as usize;
        }
        self.picked = true;
        cx.notify();
    }
}

/// A server row as a result. Its link is opened through the deep-link parser, so the server can
/// add a source without the app learning a new kind of place.
fn server_hit(hit: ServerHit) -> Hit {
    Hit {
        kind: match hit.kind.as_str() {
            "event" => "event",
            "person" => "person",
            "document" => "document",
            "item" => "item",
            "subscription" => "subscription",
            "trip" => "trip",
            "mail" => "mail",
            _ => "result",
        },
        title: hit.title,
        detail: hit.detail,
        open: Open::Link(hit.link),
    }
}

/// The records as results: projects first, then open tasks, then notes, which is also the order
/// ties keep.
fn record_hits(
    projects: Vec<ProjectSummary>,
    tasks: Vec<GlobalTask>,
    notes: Vec<GlobalNote>,
) -> Vec<Hit> {
    let projects = projects
        .into_iter()
        .filter(|project| !project.archived)
        .map(|project| Hit {
            kind: "project",
            detail: format!("{} open · {} notes", project.open, project.notes),
            open: Open::Project(project.name.clone()),
            title: project.name,
        });
    let tasks = tasks.into_iter().filter(|task| !task.done).map(|task| Hit {
        kind: "task",
        detail: match &task.due {
            Some(due) => format!("{} · due {due}", task.project),
            None => task.project.clone(),
        },
        open: Open::Record {
            project: task.project.clone(),
            path: task.id.clone(),
        },
        title: task.title,
    });
    let notes = notes.into_iter().map(|note| Hit {
        kind: "note",
        detail: note.project.clone(),
        open: Open::Record {
            project: note.project.clone(),
            path: note.path.clone(),
        },
        title: note.title,
    });
    projects.chain(tasks).chain(notes).collect()
}

impl Focusable for Finder {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Finder {
    /// Bindings the finder carries. Up and down reach it because a one-line input lets them
    /// through, the same way the kit's command palette receives them. Escape is the
    /// workspace's window-wide binding, which closes an open finder first.
    pub fn keys() -> [gpui_kit::KeyBinding; 5] {
        [
            gpui_kit::KeyBinding::new("up", SelectUp, Some("Finder")),
            gpui_kit::KeyBinding::new("down", SelectDown, Some("Finder")),
            gpui_kit::KeyBinding::new("ctrl-p", SelectUp, Some("Finder")),
            gpui_kit::KeyBinding::new("ctrl-n", SelectDown, Some("Finder")),
            gpui_kit::KeyBinding::new("cmd-w", Close, Some("Finder")),
        ]
    }

    /// Tab is the input's own key (it indents), so the mode cycles on ⌘/ instead.
    pub fn mode_key() -> gpui_kit::KeyBinding {
        gpui_kit::KeyBinding::new("cmd-/", NextMode, Some("Finder"))
    }
}

impl Render for Finder {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let palette = Palette::for_appearance(window.appearance());
        let input = self.ensure_input(window, cx);
        let resolved = self.resolved();
        let exchange = self
            .asked
            .then(|| self.ask.read(cx).latest())
            .flatten()
            .filter(|_| self.query.trim().is_empty());
        // Under an answer the page list is noise; typing again brings results back.
        let hits: Vec<Hit> = if exchange.is_some() {
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
                    palette.accent << 8 | 0x2a
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
        let empty = (nothing && exchange.is_none() && !self.query.trim().is_empty()).then(|| {
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

        let answer = exchange.map(|exchange| {
            let status = if exchange.loading {
                if exchange.tools.is_empty() {
                    "working…".to_owned()
                } else {
                    format!("working · {}", exchange.tools.join(", "))
                }
            } else if exchange.tools.is_empty() {
                String::new()
            } else {
                format!("used {}", exchange.tools.join(", "))
            };
            div()
                .id("finder-answer")
                .flex()
                .flex_col()
                .gap_2()
                .px_4()
                .py_3()
                .max_h(px(360.))
                .overflow_y_scroll()
                .border_b_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(exchange.prompt),
                )
                .when(!exchange.answer.is_empty(), |card| {
                    card.child(div().text_sm().child(exchange.answer))
                })
                .when_some(exchange.error, |card, error| {
                    card.child(div().text_sm().text_color(rgb(palette.red)).child(error))
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(status)
                        .child(div().flex_1())
                        .child(
                            div()
                                .id("finder-continue")
                                .cursor_pointer()
                                .hover(move |link| link.text_color(rgb(palette.text)))
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.close(cx);
                                    cx.emit(FinderEvent::ContinueInAsk);
                                }))
                                .child("whole conversation · ⌘2"),
                        ),
                )
        });

        let panel = div()
            .key_context("Finder")
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, _: &SelectUp, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &SelectDown, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &NextMode, _, cx| {
                view.mode = view.mode.next();
                view.search_server_soon(cx);
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
            .bg(rgb(palette.panel()))
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
            .children(answer)
            .children(empty)
            .child(
                // Sized to its rows: two results make a short panel, not a tall empty one.
                div()
                    .id("finder-list")
                    .max_h(px(420.))
                    .overflow_y_scroll()
                    .child(list),
            )
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
                    .child("↩ open or ask")
                    .child("⌘↩ the other way")
                    .child("⌘/ mode")
                    .child("esc close"),
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
    use super::*;

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
        let hits = record_hits(projects, tasks, notes);
        let kinds: Vec<&str> = hits.iter().map(|hit| hit.kind).collect();
        assert_eq!(
            kinds,
            ["project", "task", "note"],
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
