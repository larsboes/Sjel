//! The day itself: calendar entries, due tasks, and daily expenses as one timeline.
//!
//! It is a block of the Today page (`home`), not a page of its own, so it holds the day on screen
//! and nothing about where that block sits. `/` and `/context` both reach it (`core::area` folds
//! the two addresses into one).

pub mod cards;
pub mod timeline;
pub mod types;

pub use types::{bind_keys, ContextEvent, NextDay, PreviousDay, Refresh, Today};

use chrono::{Duration, Local, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, rgb, AnyElement, Context, EventEmitter, FocusHandle, FontWeight, Window,
};
use sjel_gpui_core::context::{ContextDay, TimelineItem, TimelineKind};
use sjel_gpui_core::shell::{
    self, CalendarEntry, FinanceDashboardProjection, GlobalTask, ShellError,
};

use crate::data::{Remote, Repo};
use crate::primitives::key_bar;
use crate::theme::{Density, Palette, ThemeContext};

use self::cards::{render_day_header, render_filter_action, render_metric_cards};
use self::timeline::render_timeline_content;

pub struct ContextPage {
    repo: Repo,
    palette: Option<Palette>,
    density: Density,
    pub day: NaiveDate,
    pub filter_kind: Option<TimelineKind>,
    pub entries: Remote<Vec<CalendarEntry>>,
    pub tasks: Remote<Vec<GlobalTask>>,
    pub finance: Remote<FinanceDashboardProjection>,
    focus: FocusHandle,
    focus_next: bool,
}

impl EventEmitter<ContextEvent> for ContextPage {}

impl ContextPage {
    pub fn new(repo: Repo, cx: &mut Context<Self>) -> Self {
        Self {
            repo,
            palette: None,
            density: Density::Desktop,
            day: Local::now().date_naive(),
            filter_kind: None,
            entries: Remote::default(),
            tasks: Remote::default(),
            finance: Remote::default(),
            focus: cx.focus_handle(),
            focus_next: true,
        }
    }

    pub fn toggle_filter(&mut self, kind: TimelineKind, cx: &mut Context<Self>) {
        if self.filter_kind == Some(kind) {
            self.filter_kind = None;
        } else {
            self.filter_kind = Some(kind);
        }
        cx.notify();
    }

    pub fn with_density(mut self, density: Density) -> Self {
        self.density = density;
        self
    }

    pub fn set_density(&mut self, density: Density, cx: &mut Context<Self>) {
        if self.density != density {
            self.density = density;
            cx.notify();
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

    pub(crate) fn toggle_task(&mut self, task_id: String, done: bool, cx: &mut Context<Self>) {
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
        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
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

        let header = render_day_header(self.day, is_today, palette, cx);
        let cards = render_metric_cards(&context, tasks, is_today, self.filter_kind, palette, cx);

        let filtered_timeline: Vec<TimelineItem> = context
            .timeline
            .into_iter()
            .filter(|item| self.filter_kind.is_none_or(|k| item.kind == k))
            .collect();

        let timeline_content =
            render_timeline_content(filtered_timeline, self.filter_kind, self.day, palette, cx);

        let filter_action = render_filter_action(self.filter_kind, self.day, cx);

        div()
            .key_context("ContextTimeline")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_previous_day))
            .on_action(cx.listener(Self::on_next_day))
            .on_action(cx.listener(Self::on_today))
            .on_action(cx.listener(Self::on_refresh))
            .flex()
            .flex_col()
            .gap_3()
            .child(header)
            .child(cards)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child(match self.filter_kind {
                                        Some(TimelineKind::Event) => "DAILY TIMELINE · EVENTS",
                                        Some(TimelineKind::Task) => "DAILY TIMELINE · TASKS",
                                        Some(TimelineKind::Spend) => "DAILY TIMELINE · SPENDING",
                                        None => "DAILY TIMELINE",
                                    }),
                            )
                            .when(self.filter_kind.is_some(), |el| {
                                el.child(
                                    Button::new("context-clear-filter")
                                        .xsmall()
                                        .ghost()
                                        .label("Show all")
                                        .on_click(cx.listener(|page, _, _, cx| {
                                            page.filter_kind = None;
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .children(filter_action),
            )
            .child(timeline_content)
            .when(!self.density.is_mobile(), |el| {
                el.child(div().pt_2().child(key_bar(
                    &[
                        ("← / → or [ ]", "step day"),
                        ("t", "today"),
                        ("⌘r", "refresh"),
                    ],
                    palette,
                )))
            })
            .into_any_element()
    }
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
