//! Phone projects, tasks and note editing over the existing local replica.
use crate::nsh_ui::{action, caption, field, heading, snapshot};
use nsh_core::prelude::*;
use sjel_gpui_core::{
    data::{Blocked, Repo},
    shell::{self, GlobalNote, GlobalTask, ProjectRecord, ProjectSummary, ShellError},
    tasks::{
        sort_notes, sort_tasks, status_label, NoteSort, StatusFilter, TaskSort, TASK_STATUSES,
    },
};
use std::sync::{Mutex, OnceLock};
use tokio::sync::watch;

#[derive(Clone, Default)]
struct Snapshot {
    projects: Vec<ProjectSummary>,
    tasks: Vec<GlobalTask>,
    notes: Vec<GlobalNote>,
    records: Vec<ProjectRecord>,
    project: Option<String>,
    record: Option<ProjectRecord>,
    blocked: Vec<Blocked>,
    error: Option<String>,
    loading: bool,
    saving: bool,
    saved: u64,
    sync: String,
}
struct State {
    snap: Snapshot,
    repo: Option<Repo>,
    generation: u64,
    record_generation: u64,
}
struct Feed {
    state: Mutex<State>,
    tx: watch::Sender<Snapshot>,
    retry: Mutex<Option<Box<dyn Fn() + Send>>>,
}
fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let snap = Snapshot::default();
        let (tx, _) = watch::channel(snap.clone());
        Feed {
            state: Mutex::new(State {
                snap,
                repo: None,
                generation: 0,
                record_generation: 0,
            }),
            tx,
            retry: Mutex::new(None),
        }
    })
}
fn state() -> std::sync::MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}
fn publish(s: &State) {
    feed().tx.send_replace(s.snap.clone());
}

/// Supplies the phone's replica. Writes retain the replica's existing outbox callback.
pub fn set_repo(repo: Repo) {
    {
        let mut s = state();
        s.repo = Some(repo);
    }
    load();
}
/// Supplies the sync refusal list without discarding locally edited records.
pub fn set_blocked(blocked: Vec<Blocked>) {
    let mut s = state();
    s.snap.blocked = blocked;
    publish(&s);
}
/// Displays the replica queue and the most recent sync result.
pub fn set_sync(pending: usize, report: Option<&str>) {
    let next = format!(
        "{pending} queued · {}",
        report.unwrap_or("Saved on this phone")
    );
    let mut s = state();
    if s.snap.sync != next {
        s.snap.sync = next;
        publish(&s);
    }
}
/// Registers the host's retry of blocked synchronization writes.
pub fn set_retry(retry: impl Fn() + Send + 'static) {
    *feed().retry.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(retry));
}
/// Opens one project from Today or a cross-capability link.
pub fn open_project(project: Option<String>) {
    {
        let mut s = state();
        s.snap.project = project;
        s.snap.record = None;
        s.record_generation += 1;
        publish(&s);
    }
    load();
}
/// Reads local records and their task and note projections.
pub fn load() {
    let (repo, project, generation) = {
        let mut s = state();
        let Some(repo) = s.repo.clone() else {
            return;
        };
        s.generation += 1;
        s.snap.loading = true;
        s.snap.error = None;
        publish(&s);
        (repo, s.snap.project.clone(), s.generation)
    };
    shell::spawn(async move {
        let (projects, tasks, notes) = futures::join!(
            repo.fetch_projects(),
            repo.fetch_tasks(None),
            repo.fetch_all_notes()
        );
        let records = if let Some(project) = project {
            repo.fetch_project_records(&project).await
        } else {
            Ok(Vec::new())
        };
        let mut s = state();
        if s.generation != generation {
            return;
        }
        s.snap.loading = false;
        match (projects, tasks, notes, records) {
            (Ok(projects), Ok(tasks), Ok(notes), Ok(records)) => {
                s.snap.projects = projects;
                s.snap.tasks = tasks;
                s.snap.notes = notes;
                s.snap.records = records;
            }
            (projects, tasks, notes, records) => {
                s.snap.error = projects
                    .err()
                    .or_else(|| tasks.err())
                    .or_else(|| notes.err())
                    .or_else(|| records.err())
                    .map(|e| e.to_string())
            }
        }
        publish(&s);
    });
}
fn open_record(path: String) {
    let (repo, generation) = {
        let mut s = state();
        let Some(repo) = s.repo.clone() else {
            return;
        };
        s.record_generation += 1;
        (repo, s.record_generation)
    };
    shell::spawn(async move {
        let result = repo.fetch_record(&path).await;
        let mut s = state();
        if s.record_generation != generation {
            return;
        }
        match result {
            Ok(record) => s.snap.record = Some(record),
            Err(e) => s.snap.error = Some(e.to_string()),
        }
        publish(&s);
    });
}

enum Write {
    Save {
        path: String,
        content: String,
        base: String,
    },
    Done {
        path: String,
        done: bool,
    },
    Property {
        path: String,
        key: &'static str,
        value: String,
    },
    Delete(String),
    Add {
        kind: &'static str,
        project: String,
        title: String,
    },
}
fn write(action: Write) {
    let (repo, editing, editing_generation) = {
        let mut s = state();
        if s.snap.saving {
            return;
        }
        let Some(repo) = s.repo.clone() else {
            return;
        };
        s.snap.saving = true;
        s.snap.error = None;
        publish(&s);
        (
            repo,
            s.snap.record.as_ref().map(|r| r.path.clone()),
            s.record_generation,
        )
    };
    shell::spawn(async move {
        let deleting = matches!(action, Write::Delete(_));
        let result = match action {
            Write::Save {
                path,
                content,
                base,
            } => repo.save_record(&path, &content, &base).await,
            Write::Done { path, done } => repo.set_task_done(&path, done).await.map(|_| ()),
            Write::Property { path, key, value } => {
                repo.set_property(&path, key, &value).await.map(|_| ())
            }
            Write::Delete(path) => repo.remove_record(&path).await,
            Write::Add {
                kind,
                project,
                title,
            } if title.trim().is_empty() || (kind != "project" && project.trim().is_empty()) => {
                Err(ShellError::Store(
                    "Choose a project and enter a title.".into(),
                ))
            }
            Write::Add {
                kind: "project",
                title,
                ..
            } => repo.add_project(title.trim(), "").await,
            Write::Add {
                kind: "task",
                project,
                title,
            } => repo.add_task(&project, title.trim()).await,
            Write::Add { project, title, .. } => repo.add_note(&project, title.trim()).await,
        };
        let landed = result.is_ok();
        {
            let mut s = state();
            s.snap.saving = false;
            match result {
                Ok(()) => {
                    s.snap.saved += 1;
                    if deleting && s.record_generation == editing_generation {
                        s.snap.record = None;
                    }
                }
                Err(e) => s.snap.error = Some(e.to_string()),
            }
            publish(&s);
        }
        if landed {
            load();
            if !deleting && state().record_generation == editing_generation {
                if let Some(path) = editing {
                    open_record(path);
                }
            }
        }
    });
}

#[component]
fn RecordEditor() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let Some(record) = state().snap.record.clone() else {
        return view();
    };
    let draft = signal(record.content.clone());
    let edit = signal(false);
    let confirm = signal(false);
    let due = signal(record.due.clone().unwrap_or_default());
    let path = record.path.clone();
    let save_path = path.clone();
    let base = signal(record.content.clone());
    let editor_path = path.clone();
    effect(move || {
        if let Some(current) = snap
            .with(|s| s.record.clone())
            .filter(|r| r.path == editor_path)
        {
            let previous = base.get();
            if current.content != previous {
                if draft.get() == previous {
                    draft.set(current.content.clone());
                }
                base.set(current.content);
            }
        }
    });
    let mut out = view()
        .gap_2()
        .child(action("Back to records", || {
            let mut s = state();
            s.snap.record = None;
            s.record_generation += 1;
            publish(&s);
        }))
        .child(heading(record.title.clone()))
        .child(caption(record.path.clone()));
    if record.kind == "task" {
        let done_path = path.clone();
        out = out
            .child(action(
                move || {
                    if snap.with(|s| s.record.as_ref().is_some_and(|r| r.done)) {
                        "Reopen task"
                    } else {
                        "Complete task"
                    }
                },
                move || {
                    write(Write::Done {
                        path: done_path.clone(),
                        done: !snap.with(|s| s.record.as_ref().is_some_and(|r| r.done)),
                    })
                },
            ))
            .child(field("Due · YYYY-MM-DD, empty clears", due));
        let due_path = path.clone();
        out = out.child(action("Save due date", move || {
            let value = due.get();
            if !value.is_empty() && chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").is_err() {
                let mut s = state();
                s.snap.error = Some("Use YYYY-MM-DD for the due date.".into());
                publish(&s);
                return;
            }
            write(Write::Property {
                path: due_path.clone(),
                key: "due",
                value,
            });
        }));
        let mut priorities = view().flex_row();
        for priority in 0..=3 {
            let path = path.clone();
            priorities = priorities.child(action(format!("Priority {priority}"), move || {
                write(Write::Property {
                    path: path.clone(),
                    key: "priority",
                    value: priority.to_string(),
                })
            }));
        }
        out = out.child(priorities);
    }
    out = out
        .child(action("Edit content", move || edit.set(true)))
        .child(dynamic(move || {
            if edit.get() {
                nsh_core::element::input()
                    .test_id("project-content")
                    .multiline(true)
                    .h(px(260.))
                    .value(move || draft.get())
                    .on_change(move |v: &str| draft.set(v.to_owned()))
                    .into_element()
            } else {
                nsh_base::text_view().markdown(base.get()).into_element()
            }
        }))
        .child(action("Save content", move || {
            write(Write::Save {
                path: save_path.clone(),
                content: draft.get(),
                base: base.get(),
            })
        }))
        .child(dynamic(move || {
            let path = path.clone();
            action(
                if confirm.get() {
                    "Confirm delete"
                } else {
                    "Delete record"
                },
                move || {
                    if confirm.get() {
                        write(Write::Delete(path.clone()));
                    } else {
                        confirm.set(true);
                    }
                },
            )
        }))
        .child(
            text(move || {
                snap.with(|s| {
                    if s.saving {
                        "Saving…".into()
                    } else {
                        s.error.clone().unwrap_or_default()
                    }
                })
            })
            .text_sm(),
        );
    out
}

#[component]
fn NewRecordForm() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let created = stored(state().snap.saved);
    let title = signal(String::new());
    let project = signal(state().snap.project.clone().unwrap_or_default());
    let kind = signal("task");
    effect(move || {
        let saved = snap.with(|s| s.saved);
        if saved != created.get() {
            title.set(String::new());
            created.replace(saved);
        }
    });
    view()
        .gap_2()
        .child(field("Project", project))
        .child(field("Title", title))
        .child(
            view()
                .flex_row()
                .child(action("Task", move || kind.set("task")))
                .child(action("Note", move || kind.set("note")))
                .child(action("Project", move || kind.set("project"))),
        )
        .child(action("Create", move || {
            write(Write::Add {
                kind: kind.get(),
                project: project.get(),
                title: title.get(),
            })
        }))
}

/// The phone's project hubs, cross-project tasks, notes, and local note editor.
#[component]
pub fn ProjectsScreen() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let tab = signal(0u8);
    let search = signal(String::new());
    let status = signal(StatusFilter::Active);
    let task_sort = signal(TaskSort::Default);
    let note_sort = signal(NoteSort::Default);
    let ascending = signal(true);
    let board = signal(false);
    let project_filter = signal(None::<String>);
    let form = signal(false);
    let selected = computed(move || snap.with(|s| s.record.as_ref().map(|r| r.path.clone())));
    view()
        .flex_1()
        .px_4()
        .gap_2()
        .child(heading("Projects"))
        .child(text(move || snap.with(|s| s.sync.clone())).text_xs())
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .child(action("Hubs", move || tab.set(0)))
                .child(action("All tasks", move || tab.set(1)))
                .child(action("All notes", move || tab.set(2)))
                .child(action("Refresh", load))
                .child(action("New", move || form.update(|v| *v = !*v))),
        )
        .child(field("Search", search))
        .child(scroll().flex_1().child(dynamic(move || {
            if selected.get().is_some() {
                return record_editor().into_element();
            }
            view()
                .child(dynamic(move || form.get().then(new_record_form)))
                .child(dynamic(move || {
                    let current = snap.get();
                    let query = search.get().to_lowercase();
                    let matches = |title: &str| title.to_lowercase().contains(&query);
                    let mut out = view().gap_1();
                    if let Some(error) = current.error {
                        out = out.child(text(error).text_sm());
                    }
                    if current.loading {
                        out = out.child(caption("Reading records…"));
                    }
                    for blocked in current.blocked {
                        let path = blocked.path;
                        out = out
                            .child(action(format!("Blocked · {}", blocked.title), move || {
                                open_record(path.clone())
                            }))
                            .child(caption(blocked.reason));
                    }
                    if !state().snap.blocked.is_empty() {
                        out = out.child(action("Retry synchronization", || {
                            if let Some(retry) = feed()
                                .retry
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .as_ref()
                            {
                                retry();
                            }
                        }));
                    }
                    if let Some(project) = current.project {
                        out = out
                            .child(heading(project))
                            .child(action("All projects", || open_project(None)));
                        for record in current.records.iter().filter(|r| matches(&r.title)) {
                            let path = record.path.clone();
                            out = out.child(action(
                                format!("{} · {}", record.kind, record.title),
                                move || open_record(path.clone()),
                            ));
                        }
                    } else if tab.get() == 0 {
                        for project in current.projects.iter().filter(|p| matches(&p.name)) {
                            let name = project.name.clone();
                            out = out.child(action(
                                format!(
                                    "{} · {} open · {} notes",
                                    project.name, project.open, project.notes
                                ),
                                move || open_project(Some(name.clone())),
                            ));
                        }
                    } else if tab.get() == 1 {
                        let mut filters = view()
                            .flex_row()
                            .flex_wrap()
                            .child(action("Active", move || status.set(StatusFilter::Active)))
                            .child(action("All statuses", move || {
                                status.set(StatusFilter::All)
                            }));
                        for value in TASK_STATUSES {
                            filters = filters.child(action(status_label(value), move || {
                                status.set(StatusFilter::Is(value))
                            }));
                        }
                        out = out.child(filters).child(
                            view()
                                .flex_row()
                                .flex_wrap()
                                .child(action("List / board", move || board.update(|v| *v = !*v)))
                                .child(action("Reverse sort", move || {
                                    ascending.update(|v| *v = !*v)
                                }))
                                .child(action("All projects", move || project_filter.set(None))),
                        );
                        let mut projects = view().flex_row().flex_wrap();
                        for p in &current.projects {
                            let name = p.name.clone();
                            projects = projects.child(action(name.clone(), move || {
                                project_filter.set(Some(name.clone()))
                            }));
                        }
                        out = out.child(projects);
                        let mut sorts = view().flex_row().flex_wrap();
                        for (value, label) in [
                            (TaskSort::Default, "Default"),
                            (TaskSort::Title, "Title"),
                            (TaskSort::Project, "Project"),
                            (TaskSort::Status, "Status"),
                            (TaskSort::Due, "Due"),
                            (TaskSort::Priority, "Priority"),
                        ] {
                            sorts = sorts.child(action(label, move || task_sort.set(value)));
                        }
                        out = out.child(sorts);
                        let mut tasks: Vec<_> = current
                            .tasks
                            .iter()
                            .filter(|t| {
                                matches(&t.title)
                                    && status.get().matches(t.status())
                                    && project_filter
                                        .with(|p| p.as_ref().is_none_or(|p| p == &t.project))
                            })
                            .collect();
                        sort_tasks(&mut tasks, task_sort.get(), ascending.get());
                        if board.get() {
                            tasks.sort_by_key(|t| {
                                TASK_STATUSES
                                    .iter()
                                    .position(|v| *v == t.status())
                                    .unwrap_or(0)
                            });
                        }
                        let mut previous = String::new();
                        for task in tasks {
                            if board.get() && previous != task.status() {
                                previous = task.status().into();
                                out = out.child(heading(status_label(task.status())));
                            }
                            let path = task.id.clone();
                            out = out
                                .child(action(
                                    format!("{} · {}", task.title, task.project),
                                    move || open_record(path.clone()),
                                ))
                                .child(caption(format!(
                                    "{} · priority {} · {}",
                                    task.status(),
                                    task.priority,
                                    task.due.as_deref().unwrap_or("No due date")
                                )));
                        }
                    } else {
                        let mut sorts = view().flex_row().flex_wrap();
                        for (value, label) in [
                            (NoteSort::Default, "Default"),
                            (NoteSort::Title, "Title"),
                            (NoteSort::Project, "Project"),
                            (NoteSort::Kind, "Kind"),
                        ] {
                            sorts = sorts.child(action(label, move || note_sort.set(value)));
                        }
                        out = out.child(sorts);
                        let mut notes: Vec<_> =
                            current.notes.iter().filter(|n| matches(&n.title)).collect();
                        sort_notes(&mut notes, note_sort.get(), ascending.get());
                        for note in notes {
                            let path = note.path.clone();
                            out = out.child(action(
                                format!("{} · {}", note.title, note.project),
                                move || open_record(path.clone()),
                            ));
                        }
                    }
                    out
                }))
                .into_element()
        })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replica_edit_save_and_two_press_delete_keep_the_editor_until_commit() {
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;
        use sjel_gpui_core::store::TursoStore;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (store, repo, path) = runtime.block_on(async {
            let store = std::sync::Arc::new(TursoStore::in_memory().await.unwrap());
            let repo = Repo::replica(store.clone(), || {});
            repo.add_project("Synthetic", "").await.unwrap();
            repo.add_note("Synthetic", "Migration note").await.unwrap();
            let record = repo
                .fetch_all_notes()
                .await
                .unwrap()
                .into_iter()
                .find(|n| n.title == "Migration note")
                .unwrap();
            (store, repo, record.path)
        });
        let record = runtime.block_on(repo.fetch_record(&path)).unwrap();
        {
            let mut s = state();
            s.repo = Some(repo.clone());
            s.snap.record = Some(record);
            publish(&s);
        }
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(nsh_core::element::Root::of::<ProjectsScreen>())
        })
        .unwrap();
        app.wait_for("text:Migration note").unwrap();
        app.press("text:Edit content").unwrap();
        app.type_text("id:project-content", "\nSynthetic body")
            .unwrap();
        app.press("text:Save content").unwrap();
        app.wait_for("text:Save content").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state().snap.saved == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(state().snap.saved, 1);
        let saved = runtime.block_on(repo.fetch_record(&path)).unwrap();
        assert!(saved.content.contains("Synthetic body"));
        assert!(runtime.block_on(store.pending_mutation_count()).unwrap() >= 3);
        app.press("text:Delete record").unwrap();
        app.wait_for("text:Confirm delete").unwrap();
        assert!(runtime.block_on(repo.fetch_record(&path)).is_ok());
        app.press("text:Confirm delete").unwrap();
        app.wait_for("text:Hubs").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state().snap.record.is_some() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(runtime.block_on(repo.fetch_record(&path)).is_err());
    }
}
