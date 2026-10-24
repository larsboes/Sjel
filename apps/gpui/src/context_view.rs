//! The day itself: calendar entries, due tasks, and daily expenses as one timeline.
//!
//! It is a block of the Today page (`home`), not a page of its own, so it holds the day on screen
//! and nothing about where that block sits. `/` and `/context` both reach it (`core::area` folds
//! the two addresses into one).

use chrono::{Duration, Local, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, AnyElement, App, Context, FocusHandle, FontWeight,
    KeyBinding, SharedString, Window,
};
use sjel_gpui_core::context::{
    format_day_header, format_money, needs_attention, ContextDay, TimelineItem, TimelineKind,
};
use sjel_gpui_core::shell::{
    self, CalendarEntry, FinanceDashboardProjection, GlobalTask, ShellError,
};

use crate::data::{Remote, Repo};
use crate::theme::Palette;

actions!(
    context_page,
    [
        /// Select the previous day.
        PreviousDay,
        /// Select the next day.
        NextDay,
        /// Select today.
        Today,
        /// Reload data for the day on screen.
        Refresh,
    ]
);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("left", PreviousDay, Some("ContextTimeline")),
        KeyBinding::new("h", PreviousDay, Some("ContextTimeline")),
        KeyBinding::new("[", PreviousDay, Some("ContextTimeline")),
        KeyBinding::new("right", NextDay, Some("ContextTimeline")),
        KeyBinding::new("l", NextDay, Some("ContextTimeline")),
        KeyBinding::new("]", NextDay, Some("ContextTimeline")),
        KeyBinding::new("t", Today, Some("ContextTimeline")),
    ]);
}

pub struct ContextPage {
    repo: Repo,
    palette: Option<Palette>,
    pub day: NaiveDate,
    pub entries: Remote<Vec<CalendarEntry>>,
    pub tasks: Remote<Vec<GlobalTask>>,
    pub finance: Remote<FinanceDashboardProjection>,
    focus: FocusHandle,
    focus_next: bool,
}

impl ContextPage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            day: Local::now().date_naive(),
            entries: Remote::default(),
            tasks: Remote::default(),
            finance: Remote::default(),
            focus: cx.focus_handle(),
            focus_next: true,
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.focus_next = true;
        self.load(cx);
    }

    pub fn select(&mut self, day: NaiveDate, cx: &mut Context<Self>) {
        if self.day != day {
            self.day = day;
            self.load(cx);
            cx.notify();
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else {
            return;
        };

        let day_str = self.day.format("%Y-%m-%d").to_string();
        let next_day = self.day + Duration::days(1);
        let next_day_str = next_day.format("%Y-%m-%d").to_string();

        self.entries.loading = true;
        self.tasks.loading = true;
        self.finance.loading = true;
        cx.notify();

        // 1. Fetch Calendar
        let shell_cal = shell.clone();
        let from = day_str.clone();
        let to = next_day_str;
        let cal_work = shell::run(async move { shell_cal.calendar(&from, &to).await });
        cx.spawn(async move |this, cx| {
            let res = cal_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.entries.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 2. Fetch Tasks
        // Same source as Home's rows (`Repo::fetch_tasks`), so the count matches the list.
        let repo_tasks = self.repo.clone();
        let tasks_work = shell::run(async move { repo_tasks.fetch_tasks(None).await });
        cx.spawn(async move |this, cx| {
            let res = tasks_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.tasks.settle(res);
                cx.notify();
            });
        })
        .detach();

        // 3. Fetch Finance
        let shell_fin = shell;
        let fin_day = day_str;
        let fin_work = shell::run(async move {
            shell_fin
                .finance_dashboard(Some(&fin_day), Some(&fin_day))
                .await
        });
        cx.spawn(async move |this, cx| {
            let res = fin_work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.finance.settle(res);
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_task(&mut self, task_id: String, done: bool, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        let work = shell::run(async move { shell.set_task_done(&task_id, !done).await });
        cx.spawn(async move |this, cx| {
            let res = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if let Ok(updated) = res {
                    if let Some(tasks) = page.tasks.value.as_mut() {
                        if let Some(t) = tasks.iter_mut().find(|t| t.id == updated.id) {
                            *t = updated;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_previous_day(&mut self, _: &PreviousDay, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.day - Duration::days(1), cx);
    }

    fn on_next_day(&mut self, _: &NextDay, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.day + Duration::days(1), cx);
    }

    fn on_today(&mut self, _: &Today, _: &mut Window, cx: &mut Context<Self>) {
        self.select(Local::now().date_naive(), cx);
    }

    fn on_refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.refresh(cx);
    }
}

impl Render for ContextPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette.unwrap_or_else(Palette::dark);
        self.render_day(palette, window, cx)
    }
}

impl ContextPage {
    /// The day on screen, sized to its content: the Today page owns the scroll around it.
    fn render_day(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if std::mem::take(&mut self.focus_next) {
            window.focus(&self.focus, cx);
        }
        let today = Local::now().date_naive();
        let is_today = self.day == today;

        let entries = self.entries.value.as_deref().unwrap_or(&[]);
        let tasks = self.tasks.value.as_deref().unwrap_or(&[]);
        let finance_txs = self
            .finance
            .value
            .as_ref()
            .map(|f| f.transactions.as_slice())
            .unwrap_or(&[]);

        let context = ContextDay::build(self.day, entries, tasks, finance_txs);

        // The date, at the weight of a block heading: the page's own title is the area's.
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(format_day_header(self.day)),
                    )
                    .when(is_today, |el| {
                        el.child(
                            div()
                                .px_2()
                                .py(px(2.))
                                .rounded_full()
                                .bg(rgb(palette.accent))
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xffffff))
                                .child("TODAY"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("context-prev")
                            .small()
                            .ghost()
                            .label("‹")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.select(page.day - Duration::days(1), cx)
                            })),
                    )
                    .child(
                        Button::new("context-today")
                            .small()
                            .ghost()
                            .label("Today")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.select(Local::now().date_naive(), cx)
                            })),
                    )
                    .child(
                        Button::new("context-next")
                            .small()
                            .ghost()
                            .label("›")
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.select(page.day + Duration::days(1), cx)
                            })),
                    )
                    .child(
                        Button::new("context-refresh")
                            .small()
                            .ghost()
                            .label("↻")
                            .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                    ),
            );

        // Stat cards strip
        let event_count = context.events.len();
        // One predicate for the card and the rows: `core::context::needs_attention`.
        let today_str = Local::now().format("%Y-%m-%d").to_string();
        let attention = needs_attention(tasks, &today_str);
        let mut open_ids: Vec<&str> = context
            .tasks
            .iter()
            .filter(|task| !task.done)
            .map(|task| task.id.as_str())
            .collect();
        if is_today {
            for task in &attention {
                if !open_ids.contains(&task.id.as_str()) {
                    open_ids.push(task.id.as_str());
                }
            }
        }
        let open_task_count = open_ids.len();
        let done_task_count = context.tasks.iter().filter(|t| t.done).count();
        let total_spend_str = format_money(context.total_spent_cents, &context.currency);

        let cards = div()
            .flex()
            .gap_4()
            .child(stat_card(
                "EVENTS",
                format!("{event_count} scheduled"),
                if event_count == 0 {
                    "Calendar clear"
                } else {
                    "Meetings & blocks"
                },
                palette,
            ))
            .child(stat_card(
                "TASKS",
                format!("{open_task_count} open"),
                if is_today && !attention.is_empty() {
                    "Urgent or due today".to_string()
                } else if done_task_count > 0 {
                    format!("{done_task_count} completed")
                } else {
                    "Due today".to_string()
                },
                palette,
            ))
            .child(stat_card(
                "SPENDING",
                total_spend_str,
                format!("{} transactions", context.spend.len()),
                palette,
            ));

        // Timeline list
        let timeline_content = if context.timeline.is_empty() {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .py_12()
                .gap_2()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.muted))
                        .child("Quiet day"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("No calendar events, due tasks, or transactions on this day."),
                )
                .into_any_element()
        } else {
            let mut list = div().flex().flex_col().gap_2();
            for item in context.timeline {
                list = list.child(render_timeline_row(item, palette, cx));
            }
            list.into_any_element()
        };

        div()
            .key_context("ContextTimeline")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_previous_day))
            .on_action(cx.listener(Self::on_next_day))
            .on_action(cx.listener(Self::on_today))
            .on_action(cx.listener(Self::on_refresh))
            .flex()
            .flex_col()
            .gap_4()
            .child(header)
            .child(cards)
            .child(
                div().flex().items_center().justify_between().pt_2().child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("DAILY TIMELINE"),
                ),
            )
            .child(timeline_content)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(shortcut_hint("← / → or [ ]", "step day", palette))
                    .child(shortcut_hint("t", "today", palette))
                    .child(shortcut_hint("⌘r", "refresh", palette)),
            )
            .into_any_element()
    }
}

fn stat_card(
    label: &'static str,
    main: impl Into<SharedString>,
    sub: impl Into<SharedString>,
    palette: Palette,
) -> AnyElement {
    div()
        .flex_1()
        .p_4()
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(label),
        )
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(main.into()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(sub.into()),
        )
        .into_any_element()
}

fn render_timeline_row(
    item: TimelineItem,
    palette: Palette,
    cx: &mut Context<ContextPage>,
) -> AnyElement {
    let (kind_color, kind_name) = match item.kind {
        TimelineKind::Event => (palette.accent, "EVENT"),
        TimelineKind::Task => (palette.amber, "TASK"),
        TimelineKind::Spend => (palette.green, "SPEND"),
    };

    let time_pill = match &item.time {
        Some(t) => div()
            .w(px(64.))
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.text))
            .child(t.clone()),
        None => div()
            .w(px(64.))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.muted))
            .child("ALL DAY"),
    };

    let kind_pill = div()
        .px_1()
        .py(px(1.))
        .rounded(px(3.))
        .bg(rgba((kind_color & 0xffffff) | 0x22000000))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(kind_color))
        .child(kind_name);

    let right_element: AnyElement = match item.kind {
        TimelineKind::Task => {
            let task_id = item
                .id
                .strip_prefix("task:")
                .unwrap_or(&item.id)
                .to_string();
            let is_done = item.completed;
            Button::new(SharedString::from(format!("toggle-task-{}", item.id)))
                .xsmall()
                .ghost()
                .label(if is_done { "✓ Done" } else { "○ Mark Done" })
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.toggle_task(task_id.clone(), is_done, cx);
                }))
                .into_any_element()
        }
        TimelineKind::Spend => {
            if let Some(cents) = item.amount_cents {
                let curr = item.currency.as_deref().unwrap_or("EUR");
                let is_expense = cents < 0;
                let text = format_money(cents, curr);
                div()
                    .px_2()
                    .py(px(2.))
                    .rounded(px(4.))
                    .bg(rgba(if is_expense { 0xef444422 } else { 0x22c55e22 }))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(if is_expense {
                        palette.red
                    } else {
                        palette.green
                    }))
                    .child(text)
                    .into_any_element()
            } else {
                div().into_any_element()
            }
        }
        TimelineKind::Event => {
            if let Some(badge) = item.badge {
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(badge)
                    .into_any_element()
            } else {
                div().into_any_element()
            }
        }
    };

    div()
        .flex()
        .items_center()
        .justify_between()
        .p_3()
        .rounded(px(6.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .child(
            div()
                .flex()
                .items_center()
                .gap_4()
                .min_w_0()
                .child(time_pill)
                .child(kind_pill)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.text))
                                .child(item.title),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(item.subtitle),
                        ),
                ),
        )
        .child(right_element)
        .into_any_element()
}

fn shortcut_hint(key: &'static str, desc: &'static str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .px_1()
                .py(px(1.))
                .rounded(px(3.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.text))
                .child(key),
        )
        .child(div().child(desc))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn context_day_navigation_steps_by_one_day(cx: &mut TestAppContext) {
        let page = cx.new(|cx| ContextPage::new(Repo::new(Err(ShellError::NotSetUp)), cx));
        page.update(cx, |page, cx| {
            let start = NaiveDate::from_ymd_opt(2026, 10, 22).unwrap();
            page.day = start;
            page.select(page.day + Duration::days(1), cx);
            assert_eq!(page.day, NaiveDate::from_ymd_opt(2026, 10, 23).unwrap());
            page.select(page.day - Duration::days(2), cx);
            assert_eq!(page.day, NaiveDate::from_ymd_opt(2026, 10, 21).unwrap());
        });
    }
}
