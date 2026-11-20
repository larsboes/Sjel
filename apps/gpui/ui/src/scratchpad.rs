//! The 24-hour quick scratchpad seat (`Seat::Scratchpad`) and Today quick-capture widget.
//!
//! Write notes, paste links, or jot `- [ ]` tasks without deciding where they belong first.
//! A live routing preview shows how Sjel will sort each line after 24 hours (or when "Sort now"
//! is pressed): tasks to Projects, `#buy` to Wishlist, `#go` to Places, links to Feed, and
//! remaining notes into a dated daily note in Projects.

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::feedback::Toast;
use crate::ely::primitives::Severity;
use crate::ely::theme::ControlSize;
use crate::feedback::WindowToastExt;
use gpui_kit::component::input::{InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, Render, ScrollHandle, Subscription, Window,
};
use sjel_gpui_core::route::{FeedTab, Page, Target};
use sjel_gpui_core::scratchpad::{
    now_unix_secs, sort_into_sjel, ScratchTarget, ScratchpadState, SortOutcome,
};
use sjel_gpui_core::shell;

use crate::data::Repo;
use crate::primitives::tone_chip;
use crate::theme::{tint, Palette, ThemeContext};

pub enum ScratchpadEvent {
    /// Open a destination in the main workspace after sorting or clicking a preview badge.
    Navigate(Target),
    /// A sort completed, so the workspace can refresh Inbox, Projects, Feed, and Wishlist.
    Sorted(SortOutcome),
}

pub struct ScratchpadSeat {
    repo: Repo,
    palette: Option<Palette>,
    pub state: ScratchpadState,
    textarea: Option<(Entity<TextareaState>, Subscription)>,
    quick_input: Option<(Entity<InputState>, Subscription)>,
    pub sorting: bool,
    pub error: Option<String>,
    pub last_outcome: Option<SortOutcome>,
    scroll: ScrollHandle,
    focus: FocusHandle,
}

impl EventEmitter<ScratchpadEvent> for ScratchpadSeat {}

impl Focusable for ScratchpadSeat {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        if let Some((textarea, _)) = &self.textarea {
            return textarea.read(cx).focus_handle(cx);
        }
        self.focus.clone()
    }
}

impl ScratchpadSeat {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        let state = if cfg!(test) {
            ScratchpadState::default()
        } else {
            ScratchpadState::load()
        };
        let last_outcome = state.history.first().cloned();
        Self {
            repo,
            palette: None,
            state,
            textarea: None,
            quick_input: None,
            sorting: false,
            error: None,
            last_outcome,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle(),
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let textarea = self.ensure_textarea(window, cx);
        textarea.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn ensure_textarea(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        if let Some((textarea, _)) = &self.textarea {
            return textarea.clone();
        }
        let initial = self.state.text.clone();
        let textarea = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder(
                    "Jot notes, paste links, or add tasks…\n\
                     • - [ ] Call dentist @Personal → Task\n\
                     • https://... or #look → Feed\n\
                     • #buy Linen shirt 49 € → Wishlist\n\
                     • #go Lisbon cafe → Places\n\
                     • Freeform thoughts → Daily Note",
                )
                .auto_grow(7, 28)
                .submit_on_enter(false);
            state.set_value(initial, window, cx);
            state
        });
        let sub = cx.subscribe_in(&textarea, window, |this, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                this.state.set_text(text, now_unix_secs());
                let _ = this.state.save();
                this.error = None;
                cx.notify();
            }
        });
        self.textarea = Some((textarea.clone(), sub));
        textarea
    }

    pub fn ensure_quick_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.quick_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Quick jot: note, link, - [ ] task, #buy, #go… (↩ to add)")
        });
        let sub = cx.subscribe_in(&input, window, |this, input, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let line = input.read(cx).value().trim().to_owned();
                if line.is_empty() {
                    return;
                }
                input.update(cx, |state, cx| state.set_value(String::new(), window, cx));
                this.append_line(&line, window, cx);
            }
        });
        self.quick_input = Some((input.clone(), sub));
        input
    }

    /// Appends a line from a quick-capture field or template chip and syncs the editor.
    pub fn append_line(&mut self, line: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.state.append_line(line, now_unix_secs());
        if !cfg!(test) {
            let _ = self.state.save();
        }
        if let Some((textarea, _)) = &self.textarea {
            let full = self.state.text.clone();
            textarea.update(cx, |state, cx| {
                state.set_value(full.clone(), window, cx);
                let end = full.len();
                state.set_selected_range(end..end, cx);
            });
        }
        cx.notify();
    }

    /// Inserts a prefix snippet on a new line and focuses the scratchpad editor.
    pub fn insert_template(&mut self, prefix: &str, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.state.text.trim().is_empty() {
            prefix.to_owned()
        } else if self.state.text.ends_with('\n') {
            format!("{}{prefix}", self.state.text)
        } else {
            format!("{}\n{prefix}", self.state.text)
        };
        self.state.set_text(next.clone(), now_unix_secs());
        if !cfg!(test) {
            let _ = self.state.save();
        }
        let textarea = self.ensure_textarea(window, cx);
        textarea.update(cx, |state, cx| {
            state.set_value(next.clone(), window, cx);
            let end = next.len();
            state.set_selected_range(end..end, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.set_text(String::new(), now_unix_secs());
        if !cfg!(test) {
            let _ = self.state.save();
        }
        if let Some((textarea, _)) = &self.textarea {
            textarea.update(cx, |state, cx| state.set_value(String::new(), window, cx));
        }
        self.error = None;
        cx.notify();
    }

    /// Checks whether 24 hours have passed and triggers auto-sorting if not already sorting.
    pub fn check_auto_sort(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.sorting {
            return;
        }
        let now = now_unix_secs();
        if self.state.is_expired(now) {
            self.sort_now(window, cx);
        }
    }

    /// Sorts all current scratchpad items into Sjel and clears the scratchpad on success.
    pub fn sort_now(&mut self, mut window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.sorting {
            return;
        }
        let targets = self.state.parse_targets();
        if targets.is_empty() {
            return;
        }
        self.sorting = true;
        self.error = None;
        cx.notify();

        let repo = self.repo.clone();
        let now = now_unix_secs();
        let date_label = chrono::Local::now().format("%Y-%m-%d").to_string();
        let work =
            shell::run(async move { sort_into_sjel(&repo, targets, &date_label, now).await });

        if let Some(win) = window.as_deref_mut() {
            cx.spawn_in(win, async move |this, cx| {
                let res = work.await.unwrap_or_else(|| {
                    Err(sjel_gpui_core::shell::ShellError::Store(
                        "Scratchpad sort task cancelled".into(),
                    ))
                });
                let _ = this.update_in(cx, |seat, window, cx| {
                    seat.finish_sort(res, Some(window), cx);
                });
            })
            .detach();
        } else {
            cx.spawn(async move |this, cx| {
                let res = work.await.unwrap_or_else(|| {
                    Err(sjel_gpui_core::shell::ShellError::Store(
                        "Scratchpad sort task cancelled".into(),
                    ))
                });
                let _ = this.update(cx, |seat, cx| {
                    seat.finish_sort(res, None, cx);
                });
            })
            .detach();
        }
    }

    fn finish_sort(
        &mut self,
        result: sjel_gpui_core::shell::ShellResult<SortOutcome>,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.sorting = false;
        match result {
            Ok(outcome) => {
                let summary = outcome.summary_line();
                self.state.set_text(String::new(), now_unix_secs());
                self.state.history.insert(0, outcome.clone());
                self.state.history.truncate(5);
                if !cfg!(test) {
                    let _ = self.state.save();
                }
                self.last_outcome = Some(outcome.clone());
                if let Some((textarea, _)) = &self.textarea {
                    if let Some(win) = window {
                        textarea.update(cx, |state, cx| {
                            state.set_value(String::new(), win, cx);
                        });
                        win.push_toast(Toast::new(summary).severity(Severity::Success), cx);
                    }
                } else if let Some(win) = window {
                    win.push_toast(Toast::new(summary).severity(Severity::Success), cx);
                }
                cx.emit(ScratchpadEvent::Sorted(outcome));
            }
            Err(err) => {
                let msg = err.to_string();
                self.error = Some(msg.clone());
                if let Some(win) = window {
                    win.push_toast(Toast::new(msg).severity(Severity::Danger), cx);
                }
            }
        }
        cx.notify();
    }

    fn target_color(target: &ScratchTarget, palette: Palette) -> u32 {
        match target {
            ScratchTarget::Task { done: true, .. } => palette.muted,
            ScratchTarget::Task { done: false, .. } => palette.accent,
            ScratchTarget::Wishlist { .. } => palette.amber,
            ScratchTarget::Place { .. } => palette.green,
            ScratchTarget::Feed { .. } => palette.accent,
            ScratchTarget::Inbox { .. } => palette.amber,
            ScratchTarget::Note { .. } => palette.muted,
        }
    }

    fn destination_for(target: &ScratchTarget) -> Target {
        match target {
            ScratchTarget::Task { .. } | ScratchTarget::Note { .. } => {
                Target::Native(Page::Projects)
            }
            ScratchTarget::Wishlist { .. } => Target::Native(Page::Inventory),
            ScratchTarget::Place { .. } => Target::Native(Page::Travel),
            ScratchTarget::Feed { .. } => Target::Feed(FeedTab::Reading),
            ScratchTarget::Inbox { .. } => Target::Native(Page::Inbox),
        }
    }
}

impl Render for ScratchpadSeat {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
        let textarea = self.ensure_textarea(window, cx);
        let now = now_unix_secs();
        let countdown = self.state.countdown_label(now);
        let targets = self.state.parse_targets();
        let has_content = !targets.is_empty();

        // Count breakdown by destination
        let mut tasks_n = 0usize;
        let mut links_n = 0usize;
        let mut wishes_n = 0usize;
        let mut places_n = 0usize;
        let mut notes_n = 0usize;
        for t in &targets {
            match t {
                ScratchTarget::Task { done: false, .. } => tasks_n += 1,
                ScratchTarget::Task { done: true, .. } => notes_n += 1,
                ScratchTarget::Feed { .. } | ScratchTarget::Inbox { .. } => links_n += 1,
                ScratchTarget::Wishlist { .. } => wishes_n += 1,
                ScratchTarget::Place { .. } => places_n += 1,
                ScratchTarget::Note { .. } => notes_n += 1,
            }
        }

        // Top status strip: 24h countdown badge + Sort now & Clear actions
        let status_strip = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(match countdown {
                        Some(label) => tone_chip(label, palette.amber),
                        None => tone_chip("24h auto-sort", palette.muted),
                    })
                    .when(has_content, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{} items staged", targets.len())),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .when(has_content, |el| {
                        el.child(
                            Button::new("scratchpad-clear", "Clear")
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .disabled(self.sorting)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.clear(window, cx)),
                                ),
                        )
                    })
                    .child(
                        Button::new(
                            "scratchpad-sort-now",
                            if self.sorting {
                                "Sorting…"
                            } else {
                                "Sort into Sjel"
                            },
                        )
                        .size(ControlSize::Sm)
                        .variant(if has_content {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Ghost
                        })
                        .disabled(!has_content || self.sorting)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.sort_now(Some(window), cx);
                        })),
                    ),
            );

        // Quick syntax insert chips so the user doesn't have to memorize tags
        let template_chip = |id: &'static str,
                             label: &'static str,
                             prefix: &'static str,
                             color: u32,
                             cx: &mut Context<Self>| {
            div()
                .id(id)
                .px_2()
                .py(px(2.))
                .rounded(px(5.))
                .bg(rgba(tint(color, 0x16)))
                .border_1()
                .border_color(rgba(tint(color, 0x30)))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(color))
                .cursor_pointer()
                .hover(move |s| s.bg(rgba(tint(color, 0x28))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.insert_template(prefix, window, cx);
                }))
                .child(label)
        };

        let syntax_bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1p5()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .mr_1()
                    .child("Insert:"),
            )
            .child(template_chip(
                "scratch-tpl-task",
                "+ Task",
                "- [ ] ",
                palette.accent,
                cx,
            ))
            .child(template_chip(
                "scratch-tpl-buy",
                "#buy",
                "#buy ",
                palette.amber,
                cx,
            ))
            .child(template_chip(
                "scratch-tpl-look",
                "#look",
                "#look ",
                palette.accent,
                cx,
            ))
            .child(template_chip(
                "scratch-tpl-go",
                "#go",
                "#go ",
                palette.green,
                cx,
            ))
            .child(template_chip(
                "scratch-tpl-learn",
                "#learn",
                "#learn ",
                palette.muted,
                cx,
            ));

        // Live routing preview section
        let mut preview_section = div().flex().flex_col().gap_2();
        if has_content {
            let mut summary_chips = div().flex().flex_wrap().items_center().gap_1p5();
            if tasks_n > 0 {
                summary_chips = summary_chips.child(tone_chip(
                    format!("{tasks_n} {}", if tasks_n == 1 { "task" } else { "tasks" }),
                    palette.accent,
                ));
            }
            if links_n > 0 {
                summary_chips = summary_chips.child(tone_chip(
                    format!("{links_n} {}", if links_n == 1 { "link" } else { "links" }),
                    palette.accent,
                ));
            }
            if wishes_n > 0 {
                summary_chips =
                    summary_chips.child(tone_chip(format!("{wishes_n} wishlist"), palette.amber));
            }
            if places_n > 0 {
                summary_chips = summary_chips.child(tone_chip(
                    format!(
                        "{places_n} {}",
                        if places_n == 1 { "place" } else { "places" }
                    ),
                    palette.green,
                ));
            }
            if notes_n > 0 {
                summary_chips = summary_chips.child(tone_chip(
                    format!(
                        "{notes_n} note {}",
                        if notes_n == 1 { "line" } else { "lines" }
                    ),
                    palette.muted,
                ));
            }

            let rows = targets
                .iter()
                .enumerate()
                .map(|(ix, target)| {
                    let color = Self::target_color(target, palette);
                    let dest = Self::destination_for(target);
                    div()
                        .id(("scratch-preview-row", ix))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_2p5()
                        .py_1p5()
                        .rounded(px(6.))
                        .bg(rgba(palette.raised))
                        .cursor_pointer()
                        .hover(move |s| s.bg(rgba(tint(color, 0x16))))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(ScratchpadEvent::Navigate(dest.clone()));
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.text))
                                .child(target.summary()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px_1p5()
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(rgba(tint(color, 0x1e)))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(color))
                                .child(target.badge_label()),
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>();

            preview_section = preview_section
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.muted))
                                .child("AUTO-SORT PREVIEW"),
                        )
                        .child(summary_chips),
                )
                .child(div().flex().flex_col().gap_1().children(rows));
        } else {
            preview_section = preview_section.child(
                div()
                    .p_3()
                    .rounded(px(8.))
                    .bg(rgba(palette.raised))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("How the 24-hour Scratchpad works"),
                    )
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(
                        "Anything you jot here stays at your fingertips for 24 hours. \
                                 When the day is up (or whenever you press Sort into Sjel), \
                                 links move to Feed, - [ ] lines become Tasks, #buy items go to \
                                 Wishlist, #go goes to Places, and notes save to your Daily Note.",
                    )),
            );
        }

        // Recent sort history receipt
        let history_card = (!self.state.history.is_empty()).then(|| {
            let rows = self
                .state
                .history
                .iter()
                .take(3)
                .enumerate()
                .map(|(ix, item)| {
                    let note_path = item.note_path.clone();
                    div()
                        .id(("scratch-history-row", ix))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_2p5()
                        .py_1p5()
                        .rounded(px(6.))
                        .bg(rgba(palette.raised))
                        .when(note_path.is_some(), |el| {
                            el.cursor_pointer()
                                .hover(move |s| s.bg(rgba(tint(palette.accent, 0x16))))
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(ScratchpadEvent::Navigate(Target::Native(
                                        Page::Projects,
                                    )));
                                }))
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.text))
                                .child(item.summary_line()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(item.date_label.clone()),
                        )
                        .into_any_element()
                })
                .collect::<Vec<_>>();
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .pt_2()
                .border_t_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("RECENTLY SORTED"),
                )
                .children(rows)
        });

        div()
            .id("scratchpad-seat")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .vertical_scrollbar(&self.scroll)
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(status_strip)
            .child(syntax_bar)
            .child(
                div()
                    .rounded(px(8.))
                    .bg(rgba(palette.surface))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .p_2()
                    .child(Textarea::new(&textarea).appearance(false)),
            )
            .when_some(self.error.as_ref(), |el, err| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.red))
                        .child(err.clone()),
                )
            })
            .child(preview_section)
            .children(history_card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use sjel_gpui_core::scratchpad::ScratchTarget;
    use sjel_gpui_core::shell::ShellError;

    #[gpui_kit::test]
    fn scratchpad_seat_appends_lines_and_parses_live_preview(cx: &mut TestAppContext) {
        let (seat, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
            ScratchpadSeat::new(Repo::new(Err(ShellError::NotSetUp)), cx)
        });

        cx.update(|window, cx| {
            seat.update(cx, |seat, cx| {
                seat.append_line("- [ ] Call dentist @Health", window, cx);
                seat.append_line("#buy Espresso beans", window, cx);
                seat.append_line("https://news.ycombinator.com", window, cx);
            });
        });
        cx.run_until_parked();

        seat.read_with(cx, |seat, _| {
            let targets = seat.state.parse_targets();
            assert_eq!(targets.len(), 3);
            assert!(matches!(targets[0], ScratchTarget::Task { .. }));
            assert!(matches!(targets[1], ScratchTarget::Wishlist { .. }));
            assert!(matches!(targets[2], ScratchTarget::Feed { .. }));
        });
    }
}
