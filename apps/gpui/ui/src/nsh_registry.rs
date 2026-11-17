//! Read-only object navigation using the shared registry and phone replica.

use std::sync::{Mutex, OnceLock};

use nsh_core::prelude::*;
use serde_json::Value;
use sjel_gpui_core::{
    data::Repo,
    record::registry::{Binding, View},
};
use tokio::sync::watch;

use crate::nsh_ui::{action, caption, heading, snapshot};

#[derive(Clone, Default)]
struct Snapshot {
    view: Option<View>,
    selected: Option<Value>,
    error: Option<String>,
    loading: bool,
}

struct State {
    repo: Option<Repo>,
    snapshot: Snapshot,
    history: Vec<Snapshot>,
    generation: u64,
}

struct Feed {
    state: Mutex<State>,
    tx: watch::Sender<Snapshot>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let (tx, _) = watch::channel(Snapshot::default());
        Feed {
            state: Mutex::new(State {
                repo: None,
                snapshot: Snapshot::default(),
                history: Vec::new(),
                generation: 0,
            }),
            tx,
        }
    })
}

fn state() -> std::sync::MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn set_repo(repo: Repo) {
    let mut state = state();
    state.repo = Some(repo);
    state.generation += 1;
    state.history.clear();
    state.snapshot = Snapshot::default();
    feed().tx.send_replace(state.snapshot.clone());
}

fn load(binding: Option<Binding>, follow: bool) {
    let (repo, generation) = {
        let mut state = state();
        let Some(repo) = state.repo.clone() else {
            return;
        };
        if state.snapshot.loading {
            return;
        }
        if follow {
            let previous = state.snapshot.clone();
            state.history.push(previous);
        }
        state.generation += 1;
        state.snapshot.loading = true;
        state.snapshot.error = None;
        feed().tx.send_replace(state.snapshot.clone());
        (repo, state.generation)
    };
    sjel_gpui_core::shell::spawn(async move {
        let result = repo.registry_view(binding).await;
        let mut state = state();
        if state.generation != generation {
            return;
        }
        state.snapshot.loading = false;
        match result {
            Ok(view) => {
                state.snapshot.view = Some(view);
                state.snapshot.selected = None;
            }
            Err(error) => {
                state.snapshot.error = Some(error.to_string());
                if follow {
                    state.history.pop();
                }
            }
        }
        feed().tx.send_replace(state.snapshot.clone());
    });
}

fn back() {
    let mut state = state();
    if state.snapshot.loading {
        return;
    }
    if state.snapshot.selected.take().is_none() {
        if let Some(previous) = state.history.pop() {
            state.snapshot = previous;
        }
    }
    feed().tx.send_replace(state.snapshot.clone());
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

#[component]
pub(crate) fn RegistryScreen() -> impl IntoElement {
    let current = snapshot(&feed().tx);
    load(None, false);
    view()
        .gap_2()
        .child(heading("Objects"))
        .child(view().flex_row().child(action("Back", back)).child(action(
            "Reload objects",
            || {
                let binding = state()
                    .snapshot
                    .view
                    .as_ref()
                    .map(|view| view.binding.clone());
                load(binding, false);
            },
        )))
        .child(dynamic(move || {
            let current = current.get();
            let mut out = view().gap_2();
            if current.loading {
                out = out.child(caption("Reading objects…"));
            }
            if let Some(error) = current.error {
                out = out.child(caption(error));
            }
            let Some(view) = current.view else {
                return out.into_element();
            };
            let object = view.object();
            out = out.child(heading(object.label.clone()));
            if let Some(row) = current.selected {
                out = out.child(heading(value_text(&row[&object.title])));
                if let Some(properties) = object.schema["properties"].as_object() {
                    for key in properties.keys() {
                        if let Some(value) = row.get(key) {
                            out = out
                                .child(caption(key.clone()))
                                .child(text(value_text(value)));
                        }
                    }
                }
                for (index, link) in object.links.iter().enumerate() {
                    let binding = view.link(&row, index);
                    out = out.child(action(link.label.clone(), move || match &binding {
                        Ok(binding) => load(Some(binding.clone()), true),
                        Err(error) => {
                            let mut state = state();
                            state.snapshot.error = Some(error.to_string());
                            feed().tx.send_replace(state.snapshot.clone());
                        }
                    }));
                }
            } else {
                if view.rows.is_empty() {
                    out = out.child(caption("No objects"));
                }
                for row in &view.rows {
                    let selected = row.clone();
                    out = out.child(action(value_text(&row[&object.title]), move || {
                        let mut state = state();
                        if state.snapshot.loading {
                            return;
                        }
                        state.snapshot.selected = Some(selected.clone());
                        feed().tx.send_replace(state.snapshot.clone());
                    }));
                }
            }
            out.into_element()
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_engine::EngineConfig;
    use nsh_testing::Headless;
    use sjel_gpui_core::store::TursoStore;
    use std::sync::Arc;

    #[test]
    fn local_object_properties_and_relations_preserve_class_without_writes() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let store = rt.block_on(async {
            let store = Arc::new(TursoStore::in_memory().await.unwrap());
            store.connection().await.unwrap().execute(
                "INSERT INTO projects_records \
                 (path, project, kind, title, content, synced_content, remote_at, registry_class, registry_rationale, registry_archived) \
                 VALUES ('Synthetic/Note.md', 'Synthetic', 'note', 'Evidence note', 'Private body', 'Private body', 'v1', 'c2', 'Owner classification', 0)", (),
            ).await.unwrap();
            store.set_meta("registry:projects", r#"{"projects":[{"name":"Synthetic","archived":false,"open":0,"done":0,"notes":1,"data_class":"c2"}]}"#).await.unwrap();
            store
        });
        set_repo(Repo::replica(store.clone(), || {}));
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(nsh_core::element::Root::of::<RegistryScreen>())
        })
        .unwrap();
        app.wait_for("text:Synthetic").unwrap();
        app.press("text:Synthetic").unwrap();
        app.wait_for("text:c2").unwrap();
        app.press("text:Records").unwrap();
        app.wait_for("text:Evidence note").unwrap();
        app.press("text:Evidence note").unwrap();
        app.wait_for("text:Synthetic/Note.md").unwrap();
        app.wait_for("text:Owner classification").unwrap();
        app.press("text:Back").unwrap();
        app.wait_for("text:Evidence note").unwrap();
        app.press("text:Back").unwrap();
        app.wait_for("text:Synthetic").unwrap();
        rt.block_on(async {
            assert_eq!(store.pending_mutation_count().await.unwrap(), 0);
            assert_eq!(
                store
                    .fetch_record("Synthetic/Note.md")
                    .await
                    .unwrap()
                    .unwrap()
                    .content,
                "Private body"
            );
        });
    }
}
