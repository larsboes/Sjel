//! Search ranking, server querying, and record ingestion.
use std::time::Instant;

use gpui_kit::{div, prelude::*, px, rgb, rgba, Context, SharedString, Window};
use sjel_gpui_core::search::{self, Mode};

use crate::shell::{self, GlobalNote, GlobalTask, ProjectSummary, ServerHit};
use sjel_gpui_ui::theme::{tint, Palette};

use super::types::{Hit, Open, SERVER_DEBOUNCE, SERVER_SHOWN, SHOWN};
use super::Finder;

impl Finder {
    pub(super) fn load_records(&mut self, cx: &mut Context<Self>) {
        self.records_at = Some(Instant::now());
        let repo = self.repo.clone();
        let reads = shell::run(async move {
            let projects = repo.fetch_projects().await.unwrap_or_default();
            let tasks = repo.fetch_tasks(None).await.unwrap_or_default();
            let notes = repo.fetch_all_notes().await.unwrap_or_default();
            let links = match repo.shell() {
                Ok(shell) => shell
                    .web_links()
                    .await
                    .map(|reply| reply.links)
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            };
            (projects, tasks, notes, links)
        });
        cx.spawn(async move |this, cx| {
            let Some((projects, tasks, notes, links)) = reads.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                view.records = record_hits(projects, tasks, notes, links);
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
    pub(super) fn search_server_soon(&mut self, cx: &mut Context<Self>) {
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
}

pub(super) fn tier_chip(
    label: &'static str,
    active: bool,
    palette: Palette,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    let (bg, text_color, border_color) = if active {
        (
            rgba(tint(palette.accent, 0x24)),
            rgb(palette.accent),
            rgba(tint(palette.accent, 0x50)),
        )
    } else {
        (
            rgba(palette.raised),
            rgb(palette.muted),
            rgba(palette.border),
        )
    };
    div()
        .id(SharedString::from(format!("tier-{label}")))
        .flex()
        .items_center()
        .px_2()
        .py(px(1.5))
        .rounded(px(4.))
        .cursor_pointer()
        .bg(bg)
        .border_1()
        .border_color(border_color)
        .text_xs()
        .font_weight(if active {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .text_color(text_color)
        .on_click(on_click)
        .child(label)
        .into_any_element()
}

/// The first `count` lines of a tool result, with a count when there is more.
pub(super) fn first_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut shown: String = lines
        .iter()
        .take(count)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if lines.len() > count {
        shown.push_str(&format!("\n… {} more line(s)", lines.len() - count));
    }
    shown
}

/// A server row as a result. Its link is opened through the deep-link parser, so the server can
/// add a source without the app learning a new kind of place.
pub(super) fn server_hit(hit: ServerHit) -> Hit {
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
pub(super) fn record_hits(
    projects: Vec<ProjectSummary>,
    tasks: Vec<GlobalTask>,
    notes: Vec<GlobalNote>,
    links: Vec<sjel_gpui_core::web_links::WebLink>,
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
    let links = links.into_iter().map(|link| Hit {
        kind: "link",
        detail: format!(
            "{}{} · {}",
            link.group,
            link.tenant
                .as_ref()
                .map(|tenant| format!(" · {tenant}"))
                .unwrap_or_default(),
            link.url
        ),
        open: Open::WebDestination {
            id: link.id,
            url: link.url,
        },
        title: link.title,
    });
    projects.chain(tasks).chain(notes).chain(links).collect()
}
