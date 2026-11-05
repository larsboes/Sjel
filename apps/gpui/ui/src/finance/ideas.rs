//! Investment ideas: a list, one idea's thesis, price levels, notes and history, and a form for
//! a new one. Plan: `docs/plans/finance.md`, section 6.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString, Window,
};
use sjel_gpui_core::finance::format_exact_money;
use sjel_gpui_core::shell::{self, FinanceIdea, FinanceIdeaDraft, FinanceIdeaTrigger, ShellError};

use crate::primitives::tone_chip;
use crate::theme::Palette;

use super::FinancePage;

const STANCES: [&str; 4] = ["hold", "watch", "enter", "exit"];

pub(crate) struct IdeaInputs {
    new_label: Entity<InputState>,
    new_ticker: Entity<InputState>,
    thesis: Entity<TextareaState>,
    level_price: Entity<InputState>,
    level_currency: Entity<InputState>,
    note: Entity<InputState>,
}

/// A typed level as the server accepts it: digits with an optional point. A comma is read as
/// the decimal separator when there is no point, so `1200,50` works as typed in German.
fn normalize_level(text: &str) -> Option<String> {
    let text = text.trim().replace(' ', "");
    let text = if text.contains('.') {
        text
    } else {
        text.replace(',', ".")
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let digits = |part: &str| part.chars().all(|c| c.is_ascii_digit());
    (!whole.is_empty() && digits(whole) && digits(fraction) && !text.ends_with('.')).then_some(text)
}

fn decimal(mantissa: &str, scale: u32) -> Option<f64> {
    Some(mantissa.parse::<f64>().ok()? / 10f64.powi(scale as i32))
}

/// How far the price must move, in percent, to reach a level in the same currency.
fn level_gap(idea: &FinanceIdea, trigger: &FinanceIdeaTrigger) -> Option<f64> {
    let price = idea.price.as_ref()?;
    if !price.currency.eq_ignore_ascii_case(&trigger.currency) {
        return None;
    }
    let now = decimal(&price.price.mantissa, price.price.scale)?;
    let level: f64 = trigger.price.parse().ok()?;
    (now > 0.).then(|| (level - now) / now * 100.)
}

fn nearest_gap(idea: &FinanceIdea) -> Option<(f64, &FinanceIdeaTrigger)> {
    idea.current
        .triggers
        .iter()
        .filter_map(|trigger| Some((level_gap(idea, trigger)?, trigger)))
        .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
}

fn price_text(idea: &FinanceIdea) -> Option<String> {
    let price = idea.price.as_ref()?;
    let text = format_exact_money(&price.price.mantissa, price.price.scale, &price.currency)?;
    Some(format!("{text} on {}", price.observed_on))
}

fn stance_tone(stance: &str, palette: Palette) -> u32 {
    match stance {
        "enter" => palette.green,
        "exit" => palette.red,
        "watch" => palette.accent,
        _ => palette.muted,
    }
}

fn title(text: &'static str, palette: Palette) -> gpui_kit::Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted))
        .child(text)
}

impl FinancePage {
    pub(super) fn load_ideas(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        self.ideas.loading = true;
        let work = shell::run(async move { shell.finance_ideas().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.ideas.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn selected(&self) -> Option<&FinanceIdea> {
        let id = self.selected_idea.as_deref()?;
        self.ideas.value.as_ref()?.iter().find(|idea| idea.id == id)
    }

    /// Creates (`id` is `None`) or replaces an idea, then reloads and selects it.
    fn save_idea(&mut self, id: Option<String>, draft: FinanceIdeaDraft, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let work = shell::run(async move { shell.finance_save_idea(id.as_deref(), &draft).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                match result {
                    Ok(body) => {
                        page.notice = None;
                        if let Some(id) = body["id"].as_str() {
                            page.selected_idea = Some(id.to_string());
                        }
                    }
                    Err(error) => page.notice = Some(error.to_string()),
                }
                page.load_ideas(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Replaces the selected idea with `edit` applied to a copy of its current draft.
    fn edit_selected(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut FinanceIdeaDraft)) {
        let Some(idea) = self.selected() else {
            return;
        };
        let id = idea.id.clone();
        let mut draft = idea.current.clone();
        edit(&mut draft);
        self.save_idea(Some(id), draft, cx);
    }

    fn create_idea(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inputs) = self.idea_inputs.as_ref() else {
            return;
        };
        let label = inputs.new_label.read(cx).value().trim().to_string();
        if label.is_empty() {
            self.notice = Some("A new idea needs a name.".into());
            cx.notify();
            return;
        }
        let ticker = inputs.new_ticker.read(cx).value().trim().to_string();
        let (new_label, new_ticker) = (inputs.new_label.clone(), inputs.new_ticker.clone());
        new_label.update(cx, |input, cx| input.set_value("", window, cx));
        new_ticker.update(cx, |input, cx| input.set_value("", window, cx));
        let draft = FinanceIdeaDraft {
            label,
            ticker,
            stance: self.new_idea_stance.into(),
            status: "open".into(),
            ..FinanceIdeaDraft::default()
        };
        self.save_idea(None, draft, cx);
    }

    fn save_thesis(&mut self, cx: &mut Context<Self>) {
        let Some(inputs) = self.idea_inputs.as_ref() else {
            return;
        };
        let thesis = inputs.thesis.read(cx).value().trim().to_string();
        self.edit_selected(cx, |draft| draft.thesis = thesis);
    }

    fn add_level(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(inputs) = self.idea_inputs.as_ref() else {
            return;
        };
        let typed = inputs.level_price.read(cx).value().to_string();
        let Some(price) = normalize_level(&typed) else {
            self.notice = Some(format!("{typed:?} is not a price such as 1200.50."));
            cx.notify();
            return;
        };
        let currency = inputs
            .level_currency
            .read(cx)
            .value()
            .trim()
            .to_ascii_uppercase();
        let currency = if currency.is_empty() {
            self.selected()
                .and_then(|idea| idea.price.as_ref())
                .map_or_else(|| "EUR".to_string(), |price| price.currency.clone())
        } else {
            currency
        };
        let level_price = inputs.level_price.clone();
        level_price.update(cx, |input, cx| input.set_value("", window, cx));
        let direction = self.level_direction.to_string();
        self.edit_selected(cx, |draft| {
            draft.triggers.push(FinanceIdeaTrigger {
                direction,
                price,
                currency,
            })
        });
    }

    fn add_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(inputs), Some(id)) = (self.idea_inputs.as_ref(), self.selected_idea.clone())
        else {
            return;
        };
        let text = inputs.note.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let note = inputs.note.clone();
        note.update(cx, |input, cx| input.set_value("", window, cx));
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        let work = shell::run(async move { shell.finance_add_idea_note(&id, &text).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.notice = Some(error.to_string());
                }
                page.load_ideas(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn review_now(&mut self, cx: &mut Context<Self>) {
        let (Some(id), Ok(shell)) = (self.selected_idea.clone(), self.repo.shell()) else {
            return;
        };
        self.reviewing = true;
        self.notice = None;
        cx.notify();
        let work = shell::run(async move { shell.finance_review_idea(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.reviewing = false;
                if let Err(error) = result {
                    page.notice = Some(format!("Review failed: {error}"));
                }
                page.load_ideas(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn judge_review(&mut self, review_id: i64, verdict: &'static str, cx: &mut Context<Self>) {
        let (Some(id), Ok(shell)) = (self.selected_idea.clone(), self.repo.shell()) else {
            return;
        };
        let work =
            shell::run(async move { shell.finance_review_verdict(&id, review_id, verdict).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Err(error) = result {
                    page.notice = Some(error.to_string());
                }
                page.load_ideas(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn ensure_idea_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.idea_inputs.is_none() {
            let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
                cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
            };
            self.idea_inputs = Some(IdeaInputs {
                new_label: input("Name, e.g. Singapore Exchange", window, cx),
                new_ticker: input("Ticker, e.g. S68.SI", window, cx),
                thesis: cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 12)),
                level_price: input("Price", window, cx),
                level_currency: input("EUR", window, cx),
                note: input("Add a note", window, cx),
            });
        }
        // The thesis field holds the selected idea's thesis until the person edits it.
        if self.thesis_for != self.selected_idea {
            let thesis = self
                .selected()
                .map(|idea| idea.current.thesis.clone())
                .unwrap_or_default();
            if let Some(inputs) = self.idea_inputs.as_ref() {
                inputs
                    .thesis
                    .update(cx, |input, cx| input.set_value(thesis, window, cx));
            }
            self.thesis_for = self.selected_idea.clone();
        }
    }

    pub(super) fn render_ideas(
        &mut self,
        window: &mut Window,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.ideas.value.is_none() && !self.ideas.loading {
            self.load_ideas(cx);
        }
        self.ensure_idea_inputs(window, cx);
        let Some(inputs) = self.idea_inputs.as_ref() else {
            return div().into_any_element();
        };

        let stance_pills =
            |id_prefix: &'static str,
             current: &str,
             cx: &mut Context<Self>,
             on_pick: fn(&mut FinancePage, &'static str, &mut Context<FinancePage>)| {
                div().flex().gap_1().children(STANCES.iter().map(|stance| {
                    Button::new(SharedString::from(format!("{id_prefix}-{stance}")))
                        .xsmall()
                        .label(*stance)
                        .when(current == *stance, |button| button.primary())
                        .when(current != *stance, |button| button.ghost())
                        .on_click(cx.listener(move |page, _, _, cx| on_pick(page, stance, cx)))
                }))
            };

        let new_form = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(title("NEW IDEA", palette))
            .child(Input::new(&inputs.new_label).small())
            .child(Input::new(&inputs.new_ticker).small())
            .child(stance_pills(
                "idea-new-stance",
                self.new_idea_stance,
                cx,
                |page, stance, cx| {
                    page.new_idea_stance = stance;
                    cx.notify();
                },
            ))
            .child(
                Button::new("idea-create")
                    .small()
                    .primary()
                    .label("Create")
                    .on_click(cx.listener(|page, _, window, cx| page.create_idea(window, cx))),
            );

        let mut ideas: Vec<&FinanceIdea> = self.ideas.value.iter().flatten().collect();
        ideas.sort_by_key(|idea| idea.current.status != "open");
        let rows = ideas.into_iter().map(|idea| {
            let selected = self.selected_idea.as_deref() == Some(idea.id.as_str());
            let closed = idea.current.status != "open";
            let gap = nearest_gap(idea).map(|(gap, trigger)| {
                format!("{gap:+.1} % to {} {}", trigger.direction, trigger.price)
            });
            let detail = [
                (!idea.current.ticker.is_empty()).then(|| idea.current.ticker.clone()),
                price_text(idea),
                gap,
                (!idea.research.is_empty()).then(|| format!("{} research", idea.research.len())),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let id = idea.id.clone();
            div()
                .id(SharedString::from(format!("idea-row-{}", idea.id)))
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded(px(6.))
                .cursor_pointer()
                .when(selected, |row| row.bg(rgba(palette.raised)))
                .when(closed, |row| row.opacity(0.55))
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.selected_idea = Some(id.clone());
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(div().text_sm().child(idea.current.label.clone()))
                        .child(tone_chip(
                            if closed {
                                "closed".to_string()
                            } else {
                                idea.current.stance.clone()
                            },
                            stance_tone(&idea.current.stance, palette),
                        )),
                )
                .child(div().text_xs().text_color(rgb(palette.muted)).child(detail))
        });

        let list = div()
            .w(px(320.))
            .flex_none()
            .flex()
            .flex_col()
            .gap_3()
            .child(new_form)
            .child(div().flex().flex_col().gap_1().children(rows))
            .when(
                self.ideas.value.as_ref().is_some_and(Vec::is_empty),
                |list| {
                    list.child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child("No ideas yet."),
                    )
                },
            );

        let detail = match self.selected() {
            Some(idea) => self.render_idea_detail(idea.clone(), palette, cx),
            None => div()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Select an idea, or create one.")
                .into_any_element(),
        };

        div()
            .id("finance-ideas-scroll")
            .size_full()
            .flex()
            .gap_4()
            .overflow_y_scroll()
            .track_scroll(&self.ideas_scroll)
            .vertical_scrollbar(&self.ideas_scroll)
            .pb_6()
            .child(list)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .when_some(self.notice.clone(), |column, notice| {
                        column.child(div().text_sm().text_color(rgb(palette.amber)).child(notice))
                    })
                    .child(detail),
            )
            .into_any_element()
    }

    fn render_idea_detail(
        &self,
        idea: FinanceIdea,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(inputs) = self.idea_inputs.as_ref() else {
            return div().into_any_element();
        };
        let open = idea.current.status == "open";
        let panel = || {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
        };

        let header =
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(idea.current.label.clone()),
                        )
                        .child(div().text_xs().text_color(rgb(palette.muted)).child(
                            match price_text(&idea) {
                                Some(price) => format!("{} · last {price}", idea.current.ticker),
                                None if idea.current.ticker.is_empty() => {
                                    "No ticker: no price and no level checks.".into()
                                }
                                None => format!(
                                    "{} · no price yet; the nightly fetch adds one",
                                    idea.current.ticker
                                ),
                            },
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex().gap_1().children(STANCES.iter().map(|stance| {
                            let current = idea.current.stance == *stance;
                            Button::new(SharedString::from(format!("idea-stance-{stance}")))
                                .xsmall()
                                .label(*stance)
                                .when(current, |button| button.primary())
                                .when(!current, |button| button.ghost())
                                .on_click(cx.listener(move |page, _, _, cx| {
                                    page.edit_selected(cx, |draft| draft.stance = (*stance).into())
                                }))
                        })))
                        .child(
                            Button::new("idea-close")
                                .xsmall()
                                .ghost()
                                .label(if open { "Close" } else { "Reopen" })
                                .on_click(cx.listener(move |page, _, _, cx| {
                                    page.edit_selected(cx, |draft| {
                                        draft.status = if open { "closed" } else { "open" }.into()
                                    })
                                })),
                        ),
                );

        let thesis = panel()
            .child(title("THESIS", palette))
            .child(Textarea::new(&inputs.thesis))
            .child(
                div().flex().justify_end().child(
                    Button::new("idea-save-thesis")
                        .small()
                        .label("Save thesis")
                        .on_click(cx.listener(|page, _, _, cx| page.save_thesis(cx))),
                ),
            );

        let levels = idea
            .current
            .triggers
            .iter()
            .enumerate()
            .map(|(index, trigger)| {
                let gap = level_gap(&idea, trigger)
                    .map(|gap| format!("{gap:+.1} % away"))
                    .unwrap_or_default();
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .child(format!(
                        "{} {} {}",
                        trigger.direction, trigger.price, trigger.currency
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_xs().text_color(rgb(palette.muted)).child(gap))
                            .child(
                                Button::new(SharedString::from(format!("idea-level-x-{index}")))
                                    .xsmall()
                                    .ghost()
                                    .label("Remove")
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.edit_selected(cx, |draft| {
                                            if index < draft.triggers.len() {
                                                draft.triggers.remove(index);
                                            }
                                        })
                                    })),
                            ),
                    )
            });
        let direction_pill = |direction: &'static str, cx: &mut Context<Self>| {
            Button::new(SharedString::from(format!("idea-level-{direction}")))
                .xsmall()
                .label(direction)
                .when(self.level_direction == direction, |button| button.primary())
                .when(self.level_direction != direction, |button| button.ghost())
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.level_direction = direction;
                    cx.notify();
                }))
        };
        let levels = panel()
            .child(title("PRICE LEVELS", palette))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("A crossing creates a task due that day; it fires again only after the price crosses back."),
            )
            .children(levels)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(direction_pill("above", cx))
                    .child(direction_pill("below", cx))
                    .child(div().w(px(110.)).child(Input::new(&inputs.level_price).small()))
                    .child(div().w(px(64.)).child(Input::new(&inputs.level_currency).small()))
                    .child(
                        Button::new("idea-level-add")
                            .small()
                            .label("Add level")
                            .on_click(cx.listener(|page, _, window, cx| page.add_level(window, cx))),
                    ),
            );

        let notes = panel()
            .child(title("NOTES", palette))
            .children(idea.notes.iter().rev().map(|note| {
                div()
                    .flex()
                    .gap_3()
                    .text_sm()
                    .child(
                        div()
                            .w(px(80.))
                            .flex_none()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(note.recorded_at.get(..10).unwrap_or_default().to_string()),
                    )
                    .child(div().flex_1().child(note.text.clone()))
            }))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&inputs.note).small()))
                    .child(
                        Button::new("idea-note-add")
                            .small()
                            .label("Add note")
                            .on_click(cx.listener(|page, _, window, cx| page.add_note(window, cx))),
                    ),
            );

        let research = panel()
            .child(title("RESEARCH", palette))
            .child(div().text_xs().text_color(rgb(palette.muted)).child(
                "Feed items of the last 90 days that name this idea's name, ticker or keywords.",
            ))
            .children(idea.research.iter().map(|item| {
                let url = item.url.clone();
                div()
                    .id(SharedString::from(format!("idea-research-{}", item.id)))
                    .flex()
                    .gap_3()
                    .text_sm()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .child(
                        div()
                            .w(px(80.))
                            .flex_none()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(item.day.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(rgb(palette.accent))
                            .child(item.title.clone()),
                    )
            }))
            .when(idea.research.is_empty(), |panel| {
                panel.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Nothing yet. Keywords widen the match."),
                )
            });

        let review = self.render_review_panel(&idea, palette, cx);

        // Earlier theses, each shown once, newest first.
        let mut earlier: Vec<(String, String)> = Vec::new();
        for snapshot in idea.history.iter().rev().skip(1) {
            let thesis = snapshot.draft.thesis.trim();
            if !thesis.is_empty()
                && thesis != idea.current.thesis.trim()
                && earlier.iter().all(|(_, seen)| seen != thesis)
            {
                earlier.push((snapshot.recorded_at.clone(), thesis.to_string()));
            }
        }
        let history = panel()
            .child(title("HISTORY", palette))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "Created {} · {} edits",
                        idea.created_at.get(..10).unwrap_or_default(),
                        idea.history.len().saturating_sub(1)
                    )),
            )
            .children(earlier.into_iter().map(|(at, thesis)| {
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("{}: {thesis}", at.get(..10).unwrap_or_default()))
            }));

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(header)
            .child(thesis)
            .child(levels)
            .child(review)
            .child(research)
            .child(notes)
            .child(history)
            .into_any_element()
    }
}

fn action_tone(action: &str, palette: Palette) -> u32 {
    match action {
        "enter" => palette.green,
        "exit" => palette.red,
        _ => palette.muted,
    }
}

fn move_text(label: &str, bp: Option<i64>) -> Option<String> {
    bp.map(|bp| format!("{label} {:+.1} %", bp as f64 / 100.))
}

impl FinancePage {
    fn render_review_panel(
        &self,
        idea: &FinanceIdea,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let record = idea.track_record;
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(title("MODEL REVIEW", palette))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(
                        if record.scored == 0 {
                            "No proposal scored yet".to_string()
                        } else {
                            format!(
                                "{} of {} proposals right after 30 days",
                                record.right, record.scored
                            )
                        },
                    ))
                    .child(
                        Button::new("idea-review-now")
                            .xsmall()
                            .label(if self.reviewing {
                                "Reviewing…"
                            } else {
                                "Review now"
                            })
                            .disabled(self.reviewing || idea.current.status != "open")
                            .on_click(cx.listener(|page, _, _, cx| page.review_now(cx))),
                    ),
            );
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(header)
            .child(div().text_xs().text_color(rgb(palette.muted)).child(
                "Advice from a model, weekly: it reads the research below and proposes enter, exit or hold. Nothing is traded.",
            ));
        let Some(latest) = idea.reviews.first() else {
            return panel
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("No review yet."),
                )
                .into_any_element();
        };
        let at_price = match (&latest.price, &latest.currency) {
            (Some(price), Some(currency)) => format!(" at {price} {currency}"),
            _ => String::new(),
        };
        let moves = [
            move_text("30 d", latest.move_30_bp),
            move_text("90 d", latest.move_90_bp),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let review_id = latest.id;
        panel = panel
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(tone_chip(
                        latest.action.clone(),
                        action_tone(&latest.action, palette),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{}{at_price} · {}{}",
                                latest.created_at.get(..10).unwrap_or_default(),
                                latest.model,
                                if moves.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {moves}")
                                }
                            )),
                    ),
            )
            .child(div().text_sm().child(latest.digest.clone()))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(latest.rationale.clone()),
            )
            .children(latest.cited.iter().enumerate().map(|(index, item)| {
                let url = item.url.clone();
                div()
                    .id(SharedString::from(format!("idea-review-cite-{index}")))
                    .text_xs()
                    .cursor_pointer()
                    .text_color(rgb(palette.accent))
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .child(format!("{} · {}", item.day, item.title))
            }))
            .child(match latest.verdict.as_deref() {
                Some(verdict) => div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("You {verdict} this proposal."))
                    .into_any_element(),
                None => div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("idea-review-accept")
                            .xsmall()
                            .label("Accept")
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.judge_review(review_id, "accepted", cx)
                            })),
                    )
                    .child(
                        Button::new("idea-review-reject")
                            .xsmall()
                            .ghost()
                            .label("Reject")
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.judge_review(review_id, "rejected", cx)
                            })),
                    )
                    .into_any_element(),
            });
        panel
            .children(idea.reviews.iter().skip(1).map(|review| {
                let parts = [
                    Some(review.created_at.get(..10).unwrap_or_default().to_string()),
                    Some(review.action.clone()),
                    review.verdict.clone(),
                    move_text("30 d", review.move_30_bp),
                    move_text("90 d", review.move_90_bp),
                ];
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(parts.into_iter().flatten().collect::<Vec<_>>().join(" · "))
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_level_is_normalized_or_refused() {
        assert_eq!(normalize_level("1200,50").as_deref(), Some("1200.50"));
        assert_eq!(normalize_level(" 1200.5 ").as_deref(), Some("1200.5"));
        assert_eq!(normalize_level("35").as_deref(), Some("35"));
        assert_eq!(normalize_level("1.200,50"), None);
        assert_eq!(normalize_level("12a"), None);
        assert_eq!(normalize_level("12."), None);
        assert_eq!(normalize_level(""), None);
    }
}
