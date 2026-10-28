//! The Sjel Clipper in the app: Settings › Clipper edits the templates and shows the Interpreter
//! model comms owns; Feed › Clips lists and reads saved clips.
//!
//! Both read and write over comms' owner-only clip socket through `sjel_gpui_core::clipper`,
//! off the UI thread. The browser extension pulls the templates from comms, so a save here
//! reaches every browser at its next sync.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, Entity, FontWeight, Window};
use serde_json::Value;
use sjel_gpui_core::blocks::Doc;
use sjel_gpui_core::clipper::{self, ClipDocument, Settings, BEHAVIORS};
use sjel_gpui_ui::doc_view::render_doc;
use sjel_gpui_ui::theme::Density;

use crate::theme::Palette;
use crate::ui::{list, note, section};

/// The editor's inputs for the selected template.
struct Editor {
    name: Entity<InputState>,
    path: Entity<InputState>,
    note_name: Entity<InputState>,
    content: Entity<TextareaState>,
    triggers: Entity<TextareaState>,
}

pub struct ClipperSettings {
    loaded: Option<Result<Settings, String>>,
    loading: bool,
    /// The working copy; saved as a whole with the revision it was loaded at.
    templates: Vec<Value>,
    selected: usize,
    behavior: String,
    editor: Option<Editor>,
    /// The template index the editor inputs were filled from.
    editor_for: Option<usize>,
    dirty: bool,
    status: Option<(bool, String)>,
}

impl ClipperSettings {
    pub fn new() -> Self {
        Self {
            loaded: None,
            loading: false,
            templates: Vec::new(),
            selected: 0,
            behavior: "create".into(),
            editor: None,
            editor_for: None,
            dirty: false,
            status: None,
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::load_settings() })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                if let Ok(settings) = &result {
                    view.templates = settings.templates.clone();
                    view.selected = view.selected.min(view.templates.len().saturating_sub(1));
                    view.editor_for = None;
                    view.dirty = false;
                }
                view.loaded = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> &Editor {
        if self.editor.is_none() {
            self.editor = Some(Editor {
                name: cx.new(|cx| InputState::new(window, cx).placeholder("Template name")),
                path: cx.new(|cx| InputState::new(window, cx).placeholder("Clippings")),
                note_name: cx.new(|cx| InputState::new(window, cx).placeholder("{{title}}")),
                content: cx.new(|cx| TextareaState::new(window, cx).auto_grow(6, 18)),
                triggers: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 6)),
            });
        }
        if self.editor_for != Some(self.selected) {
            self.fill_editor(window, cx);
        }
        self.editor.as_ref().expect("created above")
    }

    fn fill_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let template = self
            .templates
            .get(self.selected)
            .cloned()
            .unwrap_or(Value::Null);
        self.behavior = clipper::text_field(&template, "behavior");
        if let Some(editor) = &self.editor {
            let set = |input: &Entity<InputState>,
                       value: String,
                       window: &mut Window,
                       cx: &mut Context<Self>| {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            };
            set(
                &editor.name,
                clipper::text_field(&template, "name"),
                window,
                cx,
            );
            set(
                &editor.path,
                clipper::text_field(&template, "path"),
                window,
                cx,
            );
            set(
                &editor.note_name,
                clipper::text_field(&template, "noteNameFormat"),
                window,
                cx,
            );
            let content = clipper::text_field(&template, "noteContentFormat");
            editor
                .content
                .update(cx, |input, cx| input.set_value(content, window, cx));
            let triggers = clipper::triggers(&template);
            editor
                .triggers
                .update(cx, |input, cx| input.set_value(triggers, window, cx));
        }
        self.editor_for = Some(self.selected);
    }

    /// Copy the inputs into the working copy of the selected template.
    fn commit_editor(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some(index)) = (&self.editor, self.editor_for) else {
            return;
        };
        let Some(template) = self.templates.get_mut(index) else {
            return;
        };
        let fields = [
            ("name", editor.name.read(cx).value().to_string()),
            ("path", editor.path.read(cx).value().to_string()),
            (
                "noteNameFormat",
                editor.note_name.read(cx).value().to_string(),
            ),
            (
                "noteContentFormat",
                editor.content.read(cx).value().to_string(),
            ),
        ];
        let triggers = editor.triggers.read(cx).value().to_string();
        let before = template.clone();
        clipper::apply_edit(template, &fields, &self.behavior, &triggers);
        if *template != before {
            self.dirty = true;
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.commit_editor(cx);
        self.selected = index;
        self.status = None;
        cx.notify();
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        self.commit_editor(cx);
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        self.templates
            .push(clipper::new_template(&self.templates, millis));
        self.selected = self.templates.len() - 1;
        self.dirty = true;
        cx.notify();
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        if self.templates.len() <= 1 {
            self.status = Some((
                false,
                "Keep at least one template; the extension needs one.".into(),
            ));
        } else {
            self.templates.remove(self.selected);
            self.selected = self.selected.min(self.templates.len() - 1);
            self.editor_for = None;
            self.dirty = true;
        }
        cx.notify();
    }

    fn reorder(&mut self, up: bool, cx: &mut Context<Self>) {
        self.commit_editor(cx);
        let moved = clipper::move_template(&mut self.templates, self.selected, up);
        if moved != self.selected {
            self.selected = moved;
            self.editor_for = Some(moved);
            self.dirty = true;
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.commit_editor(cx);
        let Some(Ok(settings)) = &self.loaded else {
            return;
        };
        let (templates, base) = (self.templates.clone(), settings.revision);
        self.status = Some((true, "Saving…".into()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::save_templates(&templates, base) })
                .await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(revision) => {
                        if let Some(Ok(settings)) = &mut view.loaded {
                            settings.revision = revision;
                            settings.templates = view.templates.clone();
                        }
                        view.dirty = false;
                        view.status = Some((
                            true,
                            format!("Saved as revision {revision}. Browsers apply it at their next sync, within five minutes."),
                        ));
                    }
                    Err(error) => view.status = Some((false, error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn interpreter_line(settings: &Settings) -> String {
        match (&settings.interpreter.model, settings.interpreter.available) {
            (Some(model), true) => format!(
                "Runs on {model}{}, on this Mac. The browser holds no provider key. Change the model in the inference config's light summarization role.",
                settings.interpreter.backend.as_ref().map(|b| format!(" ({b})")).unwrap_or_default()
            ),
            _ => "No local model is configured for the light summarization role, so the Interpreter refuses to run.".into(),
        }
    }
}

impl Render for ClipperSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        if self.loaded.is_none() && !self.loading {
            self.refresh(cx);
        }
        let settings = match &self.loaded {
            None => {
                return list(
                    vec![note(
                        "Reading the clipper settings from comms…",
                        palette.muted,
                    )],
                    palette,
                )
            }
            Some(Err(error)) => return list(vec![note(error.clone(), palette.red)], palette),
            Some(Ok(settings)) => settings.clone(),
        };
        if settings.revision == 0 {
            return div()
                .flex()
                .flex_col()
                .gap_6()
                .child(section("INTERPRETER", note(Self::interpreter_line(&settings), palette.muted), palette))
                .child(note(
                    "Sjel holds no clipper templates yet. Open the Sjel Clipper's settings in Chrome once; it hands its templates to Sjel, and they are edited here from then on.",
                    palette.muted,
                ))
                .into_any_element();
        }
        let editor = self.editor(window, cx);
        let (name, path, note_name, content, triggers) = (
            editor.name.clone(),
            editor.path.clone(),
            editor.note_name.clone(),
            editor.content.clone(),
            editor.triggers.clone(),
        );
        let rows: Vec<AnyElement> = self
            .templates
            .iter()
            .enumerate()
            .map(|(index, template)| {
                let selected = index == self.selected;
                Button::new(("clipper-template", index))
                    .small()
                    .when(selected, |b| b.primary())
                    .when(!selected, |b| b.ghost())
                    .label(clipper::text_field(template, "name"))
                    .on_click(cx.listener(move |view, _, _, cx| view.select(index, cx)))
                    .into_any_element()
            })
            .collect();
        let behaviors = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .children(BEHAVIORS.iter().map(|(value, label)| {
                let selected = self.behavior == *value;
                let value = (*value).to_owned();
                Button::new(format!("clipper-behavior-{value}"))
                    .small()
                    .when(selected, |b| b.primary())
                    .when(!selected, |b| b.ghost())
                    .label(*label)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.behavior = value.clone();
                        view.commit_editor(cx);
                        cx.notify();
                    }))
            }));
        let field = |label: &'static str, input: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().text_color(rgb(palette.muted)).child(label))
                .child(input)
        };
        let actions = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("clipper-up")
                    .small()
                    .ghost()
                    .label("Move up")
                    .on_click(cx.listener(|v, _, _, cx| v.reorder(true, cx))),
            )
            .child(
                Button::new("clipper-down")
                    .small()
                    .ghost()
                    .label("Move down")
                    .on_click(cx.listener(|v, _, _, cx| v.reorder(false, cx))),
            )
            .child(
                Button::new("clipper-new")
                    .small()
                    .ghost()
                    .label("New template")
                    .on_click(cx.listener(|v, _, _, cx| v.add(cx))),
            )
            .child(
                Button::new("clipper-delete")
                    .small()
                    .ghost()
                    .label("Delete")
                    .on_click(cx.listener(|v, _, _, cx| v.remove(cx))),
            )
            .child(
                Button::new("clipper-reload")
                    .small()
                    .ghost()
                    .label("Reload")
                    .on_click(cx.listener(|v, _, _, cx| v.refresh(cx))),
            )
            .child(
                Button::new("clipper-save")
                    .small()
                    .primary()
                    .label("Save")
                    .on_click(cx.listener(|v, _, _, cx| v.save(cx))),
            );
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(section("INTERPRETER", note(Self::interpreter_line(&settings), palette.muted), palette))
            .child(section(
                "TEMPLATES",
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(note(
                        format!("Revision {}. Sjel owns these; the browser shows them read-only.", settings.revision),
                        palette.muted,
                    ))
                    .child(div().flex().flex_wrap().gap_2().children(rows))
                    .child(field("Name", Input::new(&name).into_any_element()))
                    .child(field("Save behavior", behaviors.into_any_element()))
                    .child(field("Path", Input::new(&path).into_any_element()))
                    .child(field("Note name", Input::new(&note_name).into_any_element()))
                    .child(field("Note content", Textarea::new(&content).into_any_element()))
                    .child(field("Triggers, one URL prefix or schema:Type per line", Textarea::new(&triggers).into_any_element()))
                    .child(note(
                        "Properties, context and the other template fields are kept as the browser set them.",
                        palette.muted,
                    ))
                    .child(actions)
                    .children(self.dirty.then(|| note("Unsaved changes.", palette.amber)))
                    .children(self.status.as_ref().map(|(ok, line)| note(line.clone(), if *ok { palette.muted } else { palette.red })))
                    .into_any_element(),
                palette,
            ))
            .into_any_element()
    }
}

pub struct Clips {
    documents: Option<Result<Vec<ClipDocument>, String>>,
    loading: bool,
    open: Option<Result<ClipDocument, String>>,
    palette: Option<Palette>,
}

impl Clips {
    pub fn new() -> Self {
        Self {
            documents: None,
            loading: false,
            open: None,
            palette: None,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::documents(200) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                view.documents = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn open(&mut self, key: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::document(&key) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.open = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn reader(
        &self,
        document: &ClipDocument,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let doc = Doc::parse(&document.markdown);
        let url = document.url.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("clips-back")
                            .small()
                            .ghost()
                            .label("← Clips")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.open = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("clips-original")
                            .small()
                            .ghost()
                            .label("Open original")
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    ),
            )
            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(
                if document.title.is_empty() {
                    document.key.clone()
                } else {
                    document.title.clone()
                },
            ))
            .child(note(
                format!(
                    "{} · {} version{} · {}",
                    document.key,
                    document.versions,
                    if document.versions == 1 { "" } else { "s" },
                    document.updated_at
                ),
                palette.muted,
            ))
            .child(render_doc(
                &doc,
                palette,
                Density::Desktop,
                None::<fn(usize, &gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App)>,
            ))
            .into_any_element()
    }
}

impl Render for Clips {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| Palette::for_appearance(window.appearance()));
        if self.documents.is_none() && !self.loading {
            self.load(cx);
        }
        let body = match (&self.open, &self.documents) {
            (Some(Ok(document)), _) => self.reader(&document.clone(), palette, cx),
            (Some(Err(error)), _) => note(error.clone(), palette.red),
            (None, None) => note("Reading clips from comms…", palette.muted),
            (None, Some(Err(error))) => note(error.clone(), palette.red),
            (None, Some(Ok(documents))) if documents.is_empty() => note(
                "No clips yet. Save a page with the Sjel Clipper in Chrome.",
                palette.muted,
            ),
            (None, Some(Ok(documents))) => {
                let rows = documents
                    .iter()
                    .enumerate()
                    .map(|(index, document)| {
                        let key = document.key.clone();
                        div()
                            .id(("clip-row", index))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| view.open(key.clone(), cx)))
                            .child(div().font_weight(FontWeight::MEDIUM).child(
                                if document.title.is_empty() {
                                    document.key.clone()
                                } else {
                                    document.title.clone()
                                },
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!(
                                        "{} · {} · v{}",
                                        document.updated_at, document.url, document.versions
                                    )),
                            )
                            .into_any_element()
                    })
                    .collect();
                list(rows, palette)
            }
        };
        div()
            .id("clips-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(div().max_w(px(860.)).child(body))
    }
}
