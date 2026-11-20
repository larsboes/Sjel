//! The shared Projects and Objects views over the phone replica.

use std::sync::OnceLock;

use gpui_kit::{
    div, prelude::*, rgb, rgba, Context, Entity, FontWeight, Subscription, Task, Window,
};
use sjel_gpui_core::data::{Blocked, Repo};
use sjel_gpui_ui::{
    ely::buttons::{Button, ButtonVariant},
    projects::{Projects, ProjectsEvent},
    record::RegistryPage,
    theme::{Density, ThemeContext},
};
use tokio::sync::watch;

#[derive(Clone, Default)]
struct Snapshot {
    repo: Option<Repo>,
    blocked: Vec<Blocked>,
    board_generation: u64,
    project: Option<(u64, String)>,
    sync: String,
}

fn feed() -> &'static watch::Sender<Snapshot> {
    static FEED: OnceLock<watch::Sender<Snapshot>> = OnceLock::new();
    FEED.get_or_init(|| watch::channel(Snapshot::default()).0)
}

pub(crate) fn set_board(repo: Option<Repo>, blocked: Vec<Blocked>) {
    feed().send_modify(|state| {
        state.repo = repo;
        state.blocked = blocked;
        state.board_generation = state.board_generation.wrapping_add(1);
    });
}

pub(crate) fn open_project(name: String) {
    feed().send_modify(|state| {
        let revision = state
            .project
            .as_ref()
            .map_or(0, |(revision, _)| revision.wrapping_add(1));
        state.project = Some((revision, name));
    });
}

pub(crate) fn set_sync(pending: usize, report: Option<&str>) {
    let sync = format!(
        "{pending} queued · {}",
        report.unwrap_or("Saved on this phone")
    );
    feed().send_if_modified(|snapshot| {
        if snapshot.sync == sync {
            return false;
        }
        snapshot.sync = sync;
        true
    });
}

pub(crate) struct PhoneProjects {
    snapshot: Snapshot,
    seen_project: Option<u64>,
    projects: Option<Entity<Projects>>,
    projects_events: Option<Subscription>,
    registry: Option<Entity<RegistryPage>>,
    show_objects: bool,
    _updates: Task<()>,
}

impl PhoneProjects {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let mut source = feed().subscribe();
        let snapshot = source.borrow_and_update().clone();
        let updates = cx.spawn(async move |this, cx| {
            while source.changed().await.is_ok() {
                let snapshot = source.borrow_and_update().clone();
                if this
                    .update(cx, |view, cx| view.update(snapshot, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut view = Self {
            snapshot: Snapshot::default(),
            seen_project: None,
            projects: None,
            projects_events: None,
            registry: None,
            show_objects: false,
            _updates: updates,
        };
        view.update(snapshot, cx);
        view
    }

    fn update(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        let board_changed = snapshot.board_generation != self.snapshot.board_generation;
        if board_changed {
            self.registry = None;
        }
        if let Some(repo) = &snapshot.repo {
            if self.projects.is_none() {
                let projects =
                    cx.new(|cx| Projects::new(repo.clone(), cx).with_density(Density::Mobile));
                self.projects_events =
                    Some(cx.subscribe(&projects, |_, _, event, _| match event {
                        ProjectsEvent::Editing(open) => crate::gpui_host::keyboard(*open),
                        ProjectsEvent::Retry => crate::host::announce(crate::Event::Retry),
                    }));
                projects.update(cx, |view, cx| view.load_projects(cx));
                self.projects = Some(projects);
            }
            if let Some(projects) = &self.projects {
                projects.update(cx, |view, cx| {
                    if board_changed {
                        view.set_blocked(snapshot.blocked.clone(), cx);
                        view.reload(cx);
                    }
                    if let Some((revision, name)) = &snapshot.project {
                        if Some(*revision) != self.seen_project {
                            self.seen_project = Some(*revision);
                            self.show_objects = false;
                            view.open_project(name.clone(), cx);
                        }
                    }
                });
            }
        }
        self.snapshot = snapshot;
        cx.notify();
    }
}

impl Render for PhoneProjects {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let show_objects = self.show_objects;
        let mut page = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgba(palette.surface))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Projects"),
                    )
                    .child(
                        Button::new(
                            "open-project-objects",
                            if show_objects { "Projects" } else { "Objects" },
                        )
                        .variant(ButtonVariant::Ghost)
                        .disabled(self.snapshot.repo.is_none())
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.show_objects = !view.show_objects;
                            if !view.show_objects {
                                view.registry = None;
                            }
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(self.snapshot.sync.clone()),
            );

        if show_objects {
            if let Some(repo) = &self.snapshot.repo {
                let registry = self
                    .registry
                    .get_or_insert_with(|| {
                        cx.new(|cx| RegistryPage::new(repo.clone(), "projects".into(), cx))
                    })
                    .clone();
                page = page.child(div().flex_1().min_h_0().child(registry));
            }
        } else if let Some(projects) = &self.projects {
            page = page.child(div().flex_1().min_h_0().child(projects.clone()));
        } else {
            page = page.child(
                div()
                    .text_sm()
                    .child("Opening this phone’s project replica…"),
            );
        }
        page
    }
}
