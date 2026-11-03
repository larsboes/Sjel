//! The Sjel Clipper in the app: Settings › Clipper edits the templates and shows the Interpreter
//! model comms owns; Feed › Clips lists and reads saved clips.
//!
//! Both read and write over comms' owner-only clip socket through `sjel_gpui_core::clipper`,
//! off the UI thread. The browser extension pulls the templates from comms, so a save here
//! reaches every browser at its next sync.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, Window};
use serde_json::Value;
use sjel_gpui_core::blocks::Doc;
use sjel_gpui_core::clipper::{
    self, audit_citations, markdown_to_turns, turns_to_markdown, CitationStatus, ClipDocument,
    ClipTurn, Settings, BEHAVIORS,
};
use sjel_gpui_ui::doc_view::render_doc;
use sjel_gpui_ui::theme::Density;

use crate::theme::{tint, Palette};
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

struct TurnState {
    prompt: Entity<TextareaState>,
    thoughts: Entity<TextareaState>,
    show_thoughts: bool,
    response: Entity<TextareaState>,
}

struct ComposerState {
    title: Entity<InputState>,
    url: Entity<InputState>,
    turns: Vec<TurnState>,
    saving: bool,
    status: Option<(bool, String)>,
}

const CLIP_PAGE_BLOCKS: usize = 80;

pub struct Clips {
    documents: Option<Result<Vec<ClipDocument>, String>>,
    loading: bool,
    open: Option<Result<ClipDocument, String>>,
    parsed_doc: Option<Doc>,
    reader_page: usize,
    confirm_delete: Option<String>,
    confirm_delete_all: bool,
    delete_status: Option<(bool, String)>,
    palette: Option<Palette>,
    composer: Option<ComposerState>,
}

impl Clips {
    pub fn new() -> Self {
        Self {
            documents: None,
            loading: false,
            open: None,
            parsed_doc: None,
            reader_page: 0,
            confirm_delete: None,
            confirm_delete_all: false,
            delete_status: None,
            palette: None,
            composer: None,
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
                view.parsed_doc = result
                    .as_ref()
                    .ok()
                    .map(|document| Doc::parse(&document.markdown));
                view.reader_page = 0;
                view.confirm_delete = None;
                view.open = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_document(&mut self, key: String, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.delete_status = Some((true, "Removing clip…".into()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::delete_document(&key) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                view.confirm_delete = None;
                match result {
                    Ok(count) => {
                        view.open = None;
                        view.parsed_doc = None;
                        view.delete_status =
                            Some((true, format!("Removed {count} clip version(s).")));
                        view.load(cx);
                    }
                    Err(error) => view.delete_status = Some((false, error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn delete_all_documents(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.delete_status = Some((true, "Removing all clips…".into()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { clipper::delete_all_documents() })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                view.confirm_delete_all = false;
                match result {
                    Ok(count) => {
                        view.open = None;
                        view.parsed_doc = None;
                        view.documents = Some(Ok(Vec::new()));
                        view.delete_status =
                            Some((true, format!("Removed {count} clip version(s).")));
                    }
                    Err(error) => view.delete_status = Some((false, error)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        if self.composer.is_none() {
            self.start_composer(String::new(), String::new(), Vec::new(), window, cx);
        } else {
            cx.notify();
        }
    }

    pub fn start_composer(
        &mut self,
        title: String,
        url: String,
        turns: Vec<ClipTurn>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx)
                .placeholder("Clip Title (e.g. LLM Privacy and Routing Strategy)");
            s.set_value(title, window, cx);
            s
        });
        let url_input = cx.new(|cx| {
            let mut s = InputState::new(window, cx)
                .placeholder("Source URL (e.g. https://aistudio.google.com/...)");
            s.set_value(url, window, cx);
            s
        });
        let mut turn_states = Vec::new();
        for turn in turns {
            let prompt = cx.new(|cx| {
                let mut s = TextareaState::new(window, cx).auto_grow(2, 8);
                s.set_value(turn.prompt, window, cx);
                s
            });
            let thoughts = cx.new(|cx| {
                let mut s = TextareaState::new(window, cx).auto_grow(2, 6);
                if let Some(t) = turn.thoughts.as_ref() {
                    s.set_value(t.clone(), window, cx);
                }
                s
            });
            let show_thoughts = turn.thoughts.is_some();
            let response = cx.new(|cx| {
                let mut s = TextareaState::new(window, cx).auto_grow(4, 18);
                s.set_value(turn.response, window, cx);
                s
            });
            turn_states.push(TurnState {
                prompt,
                thoughts,
                show_thoughts,
                response,
            });
        }
        if turn_states.is_empty() {
            turn_states.push(TurnState {
                prompt: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 8)),
                thoughts: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 6)),
                show_thoughts: false,
                response: cx.new(|cx| TextareaState::new(window, cx).auto_grow(4, 18)),
            });
        }
        self.composer = Some(ComposerState {
            title: title_input,
            url: url_input,
            turns: turn_states,
            saving: false,
            status: None,
        });
        cx.notify();
    }

    fn add_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.composer {
            composer.turns.push(TurnState {
                prompt: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 8)),
                thoughts: cx.new(|cx| TextareaState::new(window, cx).auto_grow(2, 6)),
                show_thoughts: false,
                response: cx.new(|cx| TextareaState::new(window, cx).auto_grow(4, 18)),
            });
            cx.notify();
        }
    }

    fn remove_turn(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.composer {
            if composer.turns.len() > 1 && idx < composer.turns.len() {
                composer.turns.remove(idx);
                cx.notify();
            }
        }
    }

    fn move_turn(&mut self, idx: usize, up: bool, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.composer {
            let target = if up {
                idx.checked_sub(1)
            } else {
                idx.checked_add(1).filter(|t| *t < composer.turns.len())
            };
            if let Some(t) = target {
                composer.turns.swap(idx, t);
                cx.notify();
            }
        }
    }

    fn paste_into_prompt(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, _)) = crate::clipboard::read_rich_clipboard() {
            if let Some(composer) = &mut self.composer {
                if let Some(turn) = composer.turns.get(idx) {
                    turn.prompt
                        .update(cx, |t, cx| t.set_value(content, window, cx));
                    composer.status = Some((true, format!("Pasted prompt into Turn {}.", idx + 1)));
                    cx.notify();
                }
            }
        } else if let Some(composer) = &mut self.composer {
            composer.status = Some((false, "Clipboard is empty.".into()));
            cx.notify();
        }
    }

    fn paste_into_thoughts(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, _)) = crate::clipboard::read_rich_clipboard() {
            if let Some(composer) = &mut self.composer {
                if let Some(turn) = composer.turns.get_mut(idx) {
                    turn.show_thoughts = true;
                    turn.thoughts
                        .update(cx, |t, cx| t.set_value(content, window, cx));
                    composer.status =
                        Some((true, format!("Pasted thoughts into Turn {}.", idx + 1)));
                    cx.notify();
                }
            }
        } else if let Some(composer) = &mut self.composer {
            composer.status = Some((false, "Clipboard is empty.".into()));
            cx.notify();
        }
    }

    fn toggle_thoughts(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.composer {
            if let Some(turn) = composer.turns.get_mut(idx) {
                turn.show_thoughts = !turn.show_thoughts;
                cx.notify();
            }
        }
    }

    fn paste_rich_turn(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, is_html)) = crate::clipboard::read_rich_clipboard() {
            if let Some(composer) = &mut self.composer {
                if let Some(turn) = composer.turns.get(idx) {
                    turn.response
                        .update(cx, |t, cx| t.set_value(content, window, cx));
                    composer.status = Some((
                        true,
                        if is_html {
                            format!(
                                "Pasted rich response into Turn {} (links converted).",
                                idx + 1
                            )
                        } else {
                            format!("Pasted response into Turn {}.", idx + 1)
                        },
                    ));
                    cx.notify();
                }
            }
        } else if let Some(composer) = &mut self.composer {
            composer.status = Some((false, "Clipboard is empty.".into()));
            cx.notify();
        }
    }

    fn auto_split_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, _)) = crate::clipboard::read_rich_clipboard() {
            let turns = markdown_to_turns(&content);
            if !turns.is_empty() {
                let url = self
                    .composer
                    .as_ref()
                    .map(|c| c.url.read(cx).value().to_string())
                    .unwrap_or_default();
                let title = self
                    .composer
                    .as_ref()
                    .map(|c| c.title.read(cx).value().to_string())
                    .unwrap_or_default();
                self.start_composer(title, url, turns, window, cx);
                if let Some(composer) = &mut self.composer {
                    composer.status = Some((
                        true,
                        format!(
                            "Auto-split clipboard into {} dialogue turn(s).",
                            composer.turns.len()
                        ),
                    ));
                    cx.notify();
                }
            } else if let Some(composer) = &mut self.composer {
                composer.status = Some((
                    false,
                    "Could not detect dialogue turns in clipboard. Use individual paste buttons."
                        .into(),
                ));
                cx.notify();
            }
        }
    }

    fn save_composer(&mut self, individual: bool, cx: &mut Context<Self>) {
        let Some(composer) = &mut self.composer else {
            return;
        };
        let title = composer.title.read(cx).value().trim().to_string();
        let url = composer.url.read(cx).value().trim().to_string();
        let title = if title.is_empty() {
            "Untitled Thread Clip".to_string()
        } else {
            title
        };
        let url = if url.is_empty() {
            "https://aistudio.google.com/".to_string()
        } else {
            url
        };

        let mut turns = Vec::new();
        for (i, turn) in composer.turns.iter().enumerate() {
            let t_val = turn.thoughts.read(cx).value().trim().to_string();
            turns.push(ClipTurn {
                index: i + 1,
                prompt: turn.prompt.read(cx).value().to_string(),
                thoughts: if t_val.is_empty() { None } else { Some(t_val) },
                response: turn.response.read(cx).value().to_string(),
            });
        }
        composer.saving = true;
        composer.status = Some((true, "Saving clip to Sjel…".into()));
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if individual {
                        let mut last_key = String::new();
                        for turn in &turns {
                            let note_title = format!("{} - Turn {}", title, turn.index);
                            let single = std::slice::from_ref(turn);
                            let single_md = turns_to_markdown(single);
                            last_key = clipper::save_clip(
                                &url,
                                &note_title,
                                "create",
                                &single_md,
                                single,
                                None,
                            )?;
                        }
                        Ok(last_key)
                    } else {
                        let markdown = turns_to_markdown(&turns);
                        clipper::save_clip(&url, &title, "create", &markdown, &turns, None)
                    }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                if let Some(composer) = &mut view.composer {
                    composer.saving = false;
                    match result {
                        Ok(key) => {
                            view.composer = None;
                            view.load(cx);
                            view.open(key, cx);
                        }
                        Err(err) => {
                            composer.status = Some((false, err));
                            cx.notify();
                        }
                    }
                }
            });
        })
        .detach();
    }

    fn reader(
        &self,
        document: &ClipDocument,
        doc: &Doc,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page_count = doc.blocks.len().max(1).div_ceil(CLIP_PAGE_BLOCKS);
        let page = self.reader_page.min(page_count.saturating_sub(1));
        let start = page * CLIP_PAGE_BLOCKS;
        let end = (start + CLIP_PAGE_BLOCKS).min(doc.blocks.len());
        let page_doc = Doc {
            front: String::new(),
            blocks: doc.blocks[start..end].to_vec(),
            trailing: 0,
        };
        let page_markdown = page_doc.markdown();
        let url = document.url.clone();
        let title = if document.title.is_empty() {
            document.key.clone()
        } else {
            document.title.clone()
        };
        let doc_for_edit = document.clone();
        let key = document.key.clone();
        let key_for_delete = key.clone();
        let confirm_delete = self.confirm_delete.as_deref() == Some(key.as_str());
        let status = self.delete_status.as_ref().map(|(ok, message)| {
            note(
                message.clone(),
                if *ok { palette.muted } else { palette.red },
            )
        });

        let mut navigation = div().flex().items_center().gap_2();
        if page > 0 {
            navigation = navigation.child(
                Button::new("clip-page-previous")
                    .small()
                    .ghost()
                    .label("← Previous")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.reader_page = view.reader_page.saturating_sub(1);
                        cx.notify();
                    })),
            );
        }
        navigation = navigation.child(note(
            format!(
                "Page {} of {} · blocks {}–{} of {}",
                page + 1,
                page_count,
                start + 1,
                end,
                doc.blocks.len()
            ),
            palette.muted,
        ));
        if page + 1 < page_count {
            navigation = navigation.child(
                Button::new("clip-page-next")
                    .small()
                    .ghost()
                    .label("Next →")
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.reader_page = (view.reader_page + 1).min(page_count - 1);
                        cx.notify();
                    })),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_4()
            .children(status)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("clips-back")
                            .small()
                            .ghost()
                            .label("← Clips")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.open = None;
                                view.parsed_doc = None;
                                view.confirm_delete = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("clips-edit-composer")
                            .small()
                            .ghost()
                            .label("Edit in Composer")
                            .on_click(cx.listener(move |view, _, window, cx| {
                                let turns = markdown_to_turns(&doc_for_edit.markdown);
                                view.start_composer(
                                    doc_for_edit.title.clone(),
                                    doc_for_edit.url.clone(),
                                    turns,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("clips-original")
                            .small()
                            .ghost()
                            .label("Open original")
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .child(
                        Button::new("clips-delete-document")
                            .small()
                            .ghost()
                            .label(if confirm_delete {
                                "Confirm delete"
                            } else {
                                "Delete clip"
                            })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if view.confirm_delete.as_deref() == Some(key_for_delete.as_str()) {
                                    view.delete_document(key_for_delete.clone(), cx);
                                } else {
                                    view.confirm_delete = Some(key_for_delete.clone());
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
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
            .child(navigation)
            .child(render_citation_audit(&page_markdown, palette))
            .child(render_doc(
                &page_doc,
                palette,
                Density::Desktop,
                None::<fn(usize, &gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App)>,
            ))
            .into_any_element()
    }

    fn clips_header(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let clip_count = self
            .documents
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map_or(0, Vec::len);
        let delete_all_label = if self.confirm_delete_all {
            format!("Confirm remove all ({clip_count})")
        } else {
            "Remove all".to_string()
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .py_1()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_sm()
                            .child("Saved Clips"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Web pages and multi-turn LLM thread clips"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children((clip_count > 0).then(|| {
                        Button::new("clips-delete-all")
                            .small()
                            .ghost()
                            .label(delete_all_label)
                            .on_click(cx.listener(|view, _, _, cx| {
                                if view.confirm_delete_all {
                                    view.delete_all_documents(cx);
                                } else {
                                    view.confirm_delete_all = true;
                                    cx.notify();
                                }
                            }))
                    }))
                    .child(
                        Button::new("clips-refresh")
                            .small()
                            .ghost()
                            .label("Refresh")
                            .on_click(cx.listener(|view, _, _, cx| view.load(cx))),
                    )
                    .child(
                        Button::new("clips-new-thread")
                            .small()
                            .primary()
                            .label("+ New Thread Clip")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.start_composer("".into(), "".into(), vec![], window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_composer(&self, cx: &mut Context<Self>, palette: Palette) -> AnyElement {
        let Some(composer) = &self.composer else {
            return div().into_any_element();
        };

        let saving = composer.saving;
        let title_input = composer.title.clone();
        let url_input = composer.url.clone();
        let total_turns = composer.turns.len();

        let mut turn_cards: Vec<AnyElement> = Vec::new();
        for (i, turn) in composer.turns.iter().enumerate() {
            let prompt_entity = turn.prompt.clone();
            let thoughts_entity = turn.thoughts.clone();
            let show_thoughts = turn.show_thoughts;
            let response_entity = turn.response.clone();
            let response_text = response_entity.read(cx).value().to_string();

            let mut turn_nav = div().flex().items_center().gap_1();
            if i > 0 {
                turn_nav = turn_nav.child(
                    Button::new(("turn-up", i))
                        .small()
                        .ghost()
                        .label("↑")
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.move_turn(i, true, cx);
                        })),
                );
            }
            if i + 1 < total_turns {
                turn_nav = turn_nav.child(
                    Button::new(("turn-down", i))
                        .small()
                        .ghost()
                        .label("↓")
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.move_turn(i, false, cx);
                        })),
                );
            }
            if total_turns > 1 {
                turn_nav = turn_nav.child(
                    Button::new(("turn-del", i))
                        .small()
                        .ghost()
                        .label("✕ Delete")
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.remove_turn(i, cx);
                        })),
                );
            }

            let card = div()
                .id(("composer-turn-card", i))
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(rgb(palette.border))
                .bg(rgba(tint(palette.surface, 0x20)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_sm()
                                .child(format!("Turn {}", i + 1)),
                        )
                        .child(turn_nav),
                )
                // Prompt section
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(palette.muted))
                                        .child(format!("Prompt {}", i + 1)),
                                )
                                .child(
                                    Button::new(("turn-paste-prompt", i))
                                        .small()
                                        .ghost()
                                        .label("📋 Paste Prompt")
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.paste_into_prompt(i, window, cx);
                                        })),
                                ),
                        )
                        .child(Textarea::new(&prompt_entity)),
                )
                // Thoughts section
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    Button::new(("turn-toggle-thoughts", i))
                                        .small()
                                        .ghost()
                                        .label(if show_thoughts {
                                            "▼ Thoughts"
                                        } else {
                                            "▶ ✦ Thoughts (optional)"
                                        })
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.toggle_thoughts(i, cx);
                                        })),
                                )
                                .children(show_thoughts.then(|| {
                                    Button::new(("turn-paste-thoughts", i))
                                        .small()
                                        .ghost()
                                        .label("📋 Paste Thoughts")
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.paste_into_thoughts(i, window, cx);
                                        }))
                                })),
                        )
                        .children(show_thoughts.then(|| Textarea::new(&thoughts_entity))),
                )
                // Response section
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(palette.muted))
                                        .child(format!("Response {}", i + 1)),
                                )
                                .child(
                                    Button::new(("turn-paste-rich", i))
                                        .small()
                                        .primary()
                                        .label("📋 Paste Response (Rich + Links)")
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            view.paste_rich_turn(i, window, cx);
                                        })),
                                ),
                        )
                        .child(Textarea::new(&response_entity)),
                )
                .child(render_citation_audit(&response_text, palette));

            turn_cards.push(card.into_any_element());
        }

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("composer-cancel")
                            .small()
                            .ghost()
                            .label("← Cancel")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.composer = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Thread Composer"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("composer-auto-split")
                            .small()
                            .ghost()
                            .label("⚡ Auto-Split Clipboard")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.auto_split_clipboard(window, cx);
                            })),
                    )
                    .child(
                        Button::new("composer-save-indiv")
                            .small()
                            .ghost()
                            .label("Save Individual Notes")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.save_composer(true, cx);
                            })),
                    )
                    .child(
                        Button::new("composer-save-thread")
                            .small()
                            .primary()
                            .label(if saving {
                                "Saving…"
                            } else {
                                "Save 1 Thread Note"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.save_composer(false, cx);
                            })),
                    ),
            );

        let fields = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Title"),
                    )
                    .child(Input::new(&title_input)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Source URL"),
                    )
                    .child(Input::new(&url_input)),
            );

        let footer_actions = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                Button::new("composer-add-turn")
                    .small()
                    .ghost()
                    .label("+ Add Turn")
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.add_turn(window, cx);
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("composer-save-indiv-bottom")
                            .small()
                            .ghost()
                            .label("Save Individual Notes")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.save_composer(true, cx);
                            })),
                    )
                    .child(
                        Button::new("composer-save-thread-bottom")
                            .small()
                            .primary()
                            .label(if saving {
                                "Saving…"
                            } else {
                                "Save 1 Thread Note"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.save_composer(false, cx);
                            })),
                    ),
            );

        let status_view = composer
            .status
            .as_ref()
            .map(|(ok, msg)| note(msg.clone(), if *ok { palette.muted } else { palette.red }));

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .children(status_view)
            .child(fields)
            .children(turn_cards)
            .child(footer_actions)
            .into_any_element()
    }
}

fn render_citation_audit(text: &str, palette: Palette) -> AnyElement {
    let citations = audit_citations(text);
    if citations.is_empty() {
        return div().into_any_element();
    }
    let resolved_count = citations
        .iter()
        .filter(|c| c.status == CitationStatus::Resolved)
        .count();
    let unlinked_count = citations
        .iter()
        .filter(|c| c.status == CitationStatus::UnlinkedRef)
        .count();
    let malformed_count = citations
        .iter()
        .filter(|c| c.status == CitationStatus::MalformedUrl)
        .count();

    let chips: Vec<AnyElement> = citations
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let (bg, border, text_color, desc) = match c.status {
                CitationStatus::Resolved => (
                    rgba(tint(palette.accent, 0x18)),
                    rgb(palette.accent),
                    rgb(palette.accent),
                    c.url.clone().unwrap_or_default(),
                ),
                CitationStatus::UnlinkedRef => (
                    rgba(tint(palette.amber, 0x24)),
                    rgb(palette.amber),
                    rgb(palette.amber),
                    "Missing target URL (bare [N] from plain paste)".to_string(),
                ),
                CitationStatus::MalformedUrl => (
                    rgba(tint(palette.red, 0x24)),
                    rgb(palette.red),
                    rgb(palette.red),
                    format!("Invalid URL: {}", c.url.clone().unwrap_or_default()),
                ),
            };

            let target_url = c.url.clone();
            let mut chip = div()
                .id(("citation-chip", i))
                .flex()
                .items_center()
                .gap_1p5()
                .px_2()
                .py_0p5()
                .rounded_md()
                .border_1()
                .border_color(border)
                .bg(bg)
                .text_xs();

            if let Some(url) = target_url {
                chip = chip.cursor_pointer().on_click(move |_, _, cx| {
                    let clean = sjel_gpui_core::clipper::unwrap_google_redirect(&url);
                    if clean.contains("vertexaisearch.cloud.google.com/grounding-api-redirect/") {
                        let clean_clone = clean.clone();
                        cx.spawn(async move |cx| {
                            let resolved =
                                sjel_gpui_core::clipper::resolve_redirect_http(&clean_clone).await;
                            let _ = cx.update(|cx| cx.open_url(&resolved));
                        })
                        .detach();
                    } else {
                        cx.open_url(&clean);
                    }
                });
            }

            chip.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text_color)
                    .child(c.label),
            )
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .max_w(px(260.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(desc),
            )
            .into_any_element()
        })
        .collect();

    let summary_text = format!(
        "Citation & Link Audit: {} resolved{}, {} unlinked{}",
        resolved_count,
        if malformed_count > 0 {
            format!(", {} malformed", malformed_count)
        } else {
            String::new()
        },
        unlinked_count,
        if unlinked_count > 0 {
            " (paste rich HTML from AI Studio to preserve citation links)"
        } else {
            ""
        }
    );

    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded_lg()
        .bg(rgba(tint(palette.surface, 0x30)))
        .border_1()
        .border_color(if unlinked_count > 0 {
            rgb(palette.amber)
        } else {
            rgb(palette.border)
        })
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(if unlinked_count > 0 {
                    rgb(palette.amber)
                } else {
                    rgb(palette.muted)
                })
                .child(summary_text),
        )
        .child(div().flex().flex_wrap().gap_2().children(chips))
        .into_any_element()
}

impl Render for Clips {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| Palette::for_appearance(window.appearance()));
        if self.documents.is_none() && !self.loading {
            self.load(cx);
        }

        if self.composer.is_some() {
            let body = self.render_composer(cx, palette);
            return div().w_full().child(body);
        }

        let body = match (&self.open, &self.documents) {
            (Some(Ok(document)), _) => match self.parsed_doc.as_ref() {
                Some(doc) => self.reader(document, doc, palette, cx),
                None => note("Preparing clip…", palette.muted),
            },
            (Some(Err(error)), _) => note(error.clone(), palette.red),
            (None, None) => note("Reading clips from comms…", palette.muted),
            (None, Some(Err(error))) => note(error.clone(), palette.red),
            (None, Some(Ok(documents))) if documents.is_empty() => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.clips_header(palette, cx))
                .child(note(
                    "No clips yet. Save a page with the Sjel Clipper in Chrome, or create a thread clip.",
                    palette.muted,
                ))
                .into_any_element(),
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
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.clips_header(palette, cx))
                    .child(list(rows, palette))
                    .into_any_element()
            }
        };
        let status = self.delete_status.as_ref().map(|(ok, message)| {
            note(
                message.clone(),
                if *ok { palette.muted } else { palette.red },
            )
        });
        div().w_full().child(
            div()
                .id("clips-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(
                    div()
                        .max_w(px(860.))
                        .flex()
                        .flex_col()
                        .gap_3()
                        .children(status)
                        .child(body),
                ),
        )
    }
}
