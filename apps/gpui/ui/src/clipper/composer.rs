//! Multi-turn dialogue composer, auto-split clipboard importer, and citation audit view.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{IconName, Sizable};
use gpui_kit::{div, prelude::*, rgb, rgba, AnyElement, Context, Entity, FontWeight, Window};
use sjel_gpui_core::clipper::{self, markdown_to_turns, turns_to_markdown, ClipTurn};

use crate::primitives::note;
use crate::theme::{tint, Palette};

use super::reader::render_citation_audit;
use super::types::read_rich_clipboard;
use super::Clips;

pub(super) struct TurnState {
    pub(super) prompt: Entity<TextareaState>,
    pub(super) thoughts: Entity<TextareaState>,
    pub(super) show_thoughts: bool,
    pub(super) response: Entity<TextareaState>,
}

pub(super) struct ComposerState {
    pub(super) title: Entity<InputState>,
    pub(super) url: Entity<InputState>,
    pub(super) turns: Vec<TurnState>,
    pub(super) saving: bool,
    pub(super) status: Option<(bool, String)>,
}

impl Clips {
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

    pub(super) fn add_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn remove_turn(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.composer {
            if composer.turns.len() > 1 && idx < composer.turns.len() {
                composer.turns.remove(idx);
                cx.notify();
            }
        }
    }

    pub(super) fn move_turn(&mut self, idx: usize, up: bool, cx: &mut Context<Self>) {
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

    pub(super) fn paste_into_prompt(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, _)) = read_rich_clipboard() {
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

    pub(super) fn paste_into_thoughts(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, _)) = read_rich_clipboard() {
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

    pub(super) fn toggle_thoughts(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(composer) = &mut self.composer {
            if let Some(turn) = composer.turns.get_mut(idx) {
                turn.show_thoughts = !turn.show_thoughts;
                cx.notify();
            }
        }
    }

    pub(super) fn paste_rich_turn(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, is_html)) = read_rich_clipboard() {
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

    pub(super) fn auto_split_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((content, _)) = read_rich_clipboard() {
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

    pub(super) fn save_composer(&mut self, individual: bool, cx: &mut Context<Self>) {
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

    pub(super) fn render_composer(&self, cx: &mut Context<Self>, palette: Palette) -> AnyElement {
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
                .border_color(rgba(palette.border))
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
                                        .icon(IconName::Copy)
                                        .label("Paste Prompt")
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
                                        .icon(IconName::Copy)
                                        .label("Paste Thoughts")
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
                                        .icon(IconName::Copy)
                                        .label("Paste Response (Rich + Links)")
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
