//! Today, the phone's landing page: the day the Mac's copy answered, the tasks the replica holds,
//! and the projects those tasks belong to.

pub mod header;
pub mod rows;
pub mod types;

pub use types::*;

use std::sync::Arc;

use chrono::{Local, NaiveDate};
use gpui::{
    div, prelude::*, rgb, AnyElement, Context, EventEmitter, FontWeight, IntoElement, Render,
    SharedString, Window,
};
use sjel_gpui_core::data::{Busy, Repo};
use sjel_gpui_core::mirror::Mirror;
use sjel_gpui_core::shell::{self, CalendarEntry, ProjectRecord};
use sjel_gpui_core::today::{self, Today as View};

use crate::data::{self, Press};
use crate::theme::ThemeContext;
use crate::{Density, Palette};

/// The phone's landing page.
pub struct Today {
    /// The Mac's read-only copy, when one is on disk.
    pub(crate) copy: Option<Arc<Mirror>>,
    /// Why the copy could not be opened, in the words the person needs.
    pub(crate) missing: Option<SharedString>,
    /// A host's word about this copy, e.g. why a newer one could not be fetched.
    pub(crate) note: Option<SharedString>,
    /// The phone's own replica, when it opened.
    pub(crate) store: Option<Repo>,
    /// The records the replica answered: tasks and project names.
    pub(crate) records: Vec<ProjectRecord>,
    /// The day's entries, from the copy.
    pub(crate) entries: Vec<CalendarEntry>,
    /// The reduction, rebuilt whenever either source moves.
    pub(crate) view: Option<View>,
    /// A failed read, kept beside the rows that answered before it.
    pub(crate) error: Option<SharedString>,
    /// How many writes are waiting for the Mac, so the page can say a task will travel.
    pub(crate) pending: usize,
    /// Whether this launch has reached the Mac at all.
    pub(crate) paired: bool,
    /// True while the day is being read, so the header can say so.
    pub(crate) loading: bool,
    /// A day a launch flag fixed (`SJEL_DAY`), for a screenshot of a day that is not today.
    pub(crate) frozen: Option<NaiveDate>,
    pub(crate) busy: Busy,
    /// The last tick written ahead of the store: the record's path and the value sent.
    pub(crate) optimistic: Option<(String, bool)>,
    pub(crate) palette: Palette,
    pub(crate) palette_override: Option<Palette>,
    pub(crate) density: Density,
    pub(crate) generation: u64,
}

impl EventEmitter<TodayEvent> for Today {}

impl Today {
    pub fn new(density: Density) -> Self {
        Self::with_day(None, density)
    }

    /// The page opened on `day` rather than on the clock.
    pub fn at(day: NaiveDate, density: Density) -> Self {
        Self::with_day(Some(day), density)
    }

    fn with_day(frozen: Option<NaiveDate>, density: Density) -> Self {
        Self {
            copy: None,
            missing: None,
            note: None,
            store: None,
            records: Vec::new(),
            entries: Vec::new(),
            view: None,
            error: None,
            pending: 0,
            paired: false,
            loading: false,
            frozen,
            busy: Busy::default(),
            optimistic: None,
            palette: Palette::dark(),
            palette_override: None,
            density,
            generation: 0,
        }
    }

    /// Hands the page the Mac's copy, or the reason it is not there.
    pub fn set_copy(
        &mut self,
        copy: Option<Arc<Mirror>>,
        missing: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.copy = copy;
        self.missing = missing.map(SharedString::from);
        self.error = None;
        self.load(cx);
    }

    /// Hands the page the phone's replica, so a task can be ticked off here.
    pub fn set_store(&mut self, store: Option<Repo>, cx: &mut Context<Self>) {
        self.store = store;
        cx.notify();
    }

    /// The replica's records, pushed by the host after every read and every sync.
    pub fn set_records(&mut self, records: Vec<ProjectRecord>, cx: &mut Context<Self>) {
        self.records = records;
        self.rebuild();
        cx.notify();
    }

    /// How many writes wait for the Mac, and whether this launch has reached it.
    pub fn set_travel(&mut self, pending: usize, paired: bool, cx: &mut Context<Self>) {
        self.pending = pending;
        self.paired = paired;
        cx.notify();
    }

    /// A sentence about the data on screen from the host.
    pub fn set_note(&mut self, note: Option<String>, cx: &mut Context<Self>) {
        self.note = note.map(SharedString::from);
        cx.notify();
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != palette || self.palette_override != Some(palette) {
            self.palette = palette;
            self.palette_override = Some(palette);
            cx.notify();
        }
    }

    /// True once the day has been read, so a host can tell a blank page from a loading one.
    pub fn loaded(&self) -> bool {
        self.view.is_some()
    }

    /// The day on screen: the frozen one, or today.
    pub fn day(&self) -> NaiveDate {
        self.frozen.unwrap_or_else(|| Local::now().date_naive())
    }

    /// Reads the day's entries from the Mac's copy.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.rebuild();
        let Some(copy) = self.copy.clone() else {
            cx.notify();
            return;
        };
        let day = self.day();
        let from = day.format("%Y-%m-%d").to_string();
        let to = today::step(day, 1).format("%Y-%m-%d").to_string();
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        let work = shell::run(async move { copy.calendar(&from, &to).await });
        cx.spawn(async move |this, cx| {
            let answer = work
                .await
                .unwrap_or_else(|| Err("The day was not read.".to_owned()));
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                match answer {
                    Ok(entries) => {
                        view.entries = entries;
                        view.error = None;
                    }
                    Err(error) => view.error = Some(SharedString::from(error)),
                }
                view.rebuild();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Rebuilds the reduction from the two sources.
    pub(crate) fn rebuild(&mut self) {
        let day = self.day();
        let now = match self.frozen {
            Some(_) => None,
            None => Some(Local::now().format("%H:%M").to_string()),
        };
        self.view = Some(today::build(
            day,
            &self.entries,
            &self.records,
            now.as_deref(),
        ));
    }

    /// Ticks a task off in the replica, where the next sync carries it to the Mac.
    pub(crate) fn toggle(
        &mut self,
        path: String,
        done: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.store.clone() else {
            return;
        };
        if let Some(record) = self.records.iter_mut().find(|record| record.path == path) {
            record.done = done;
        }
        self.rebuild();
        self.optimistic = Some((path.clone(), done));
        data::write_repo(
            cx,
            window,
            self,
            &repo,
            Press::once(Self::busy, path.clone(), ""),
            move |repo| async move { repo.set_task_done(&path, done).await },
            Self::settle_task,
        );
    }

    fn settle_task(
        this: &mut Self,
        task: Option<sjel_gpui_core::shell::GlobalTask>,
        cx: &mut Context<Self>,
    ) {
        let landed = task.is_some();
        match task {
            Some(task) => {
                if let Some(record) = this
                    .records
                    .iter_mut()
                    .find(|record| record.path == task.id)
                {
                    record.done = task.done;
                    record.due = task.due.clone();
                    record.priority = task.priority;
                }
            }
            None => {
                if let Some((path, sent)) = this.optimistic.clone() {
                    if let Some(record) = this.records.iter_mut().find(|record| record.path == path)
                    {
                        record.done = !sent;
                    }
                }
            }
        }
        this.optimistic = None;
        this.rebuild();
        if landed {
            cx.emit(TodayEvent::Wrote);
        }
        cx.notify();
    }

    fn busy(&mut self) -> &mut Busy {
        &mut self.busy
    }
}

impl Render for Today {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette_override
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
        self.palette = palette;
        let view = self.view.clone();
        let mut page = div()
            .id("today")
            .flex()
            .flex_col()
            .gap_4()
            .text_color(rgb(palette.text))
            .child(self.header(cx))
            .child(self.quick_capture_bar(cx));

        let Some(view) = view else {
            return page.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Reading today…"),
            );
        };

        let entries: Vec<AnyElement> = view
            .entries
            .iter()
            .map(|entry| self.entry_row(entry, cx))
            .collect();
        let tasks: Vec<AnyElement> = view
            .focus
            .iter()
            .enumerate()
            .map(|(index, task)| self.task_row(task, index, cx))
            .collect();
        let projects: Vec<AnyElement> = view
            .projects
            .iter()
            .enumerate()
            .map(|(index, project)| self.project_row(project, index, cx))
            .collect();

        page = page.child(self.section("THE DAY", "Nothing on the calendar today.", entries));

        let mut up_next = div().flex().flex_col().gap_2();
        up_next = up_next.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("UP NEXT"),
                )
                .child(self.refresh(cx)),
        );
        up_next = up_next.child(self.section_content("Nothing left to do.", tasks, view.rest));
        page = page.child(up_next);
        page = page.child(self.section("PROJECTS", "No projects yet.", projects));
        page
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_gpui_core::shell::CalendarEntry;
    use sjel_gpui_core::today::Spot;

    fn entry(id: &str, title: &str, starts: &str, ends: &str) -> CalendarEntry {
        CalendarEntry {
            title: title.to_owned(),
            starts_at: starts.to_owned(),
            ends_at: ends.to_owned(),
            all_day: false,
            commitment: "planned".to_owned(),
            id: id.to_owned(),
            kind: "event".to_owned(),
            location: None,
            notes: None,
            source: "manual".to_owned(),
        }
    }

    fn record(path: &str, kind: &str, title: &str, done: bool) -> ProjectRecord {
        ProjectRecord {
            project: String::new(),
            path: path.to_owned(),
            kind: kind.to_owned(),
            title: title.to_owned(),
            content: String::new(),
            done,
            due: None,
            priority: 2,
        }
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 24).unwrap()
    }

    fn page(entries: Vec<CalendarEntry>, records: Vec<ProjectRecord>) -> Today {
        let mut page = Today::at(day(), Density::Mobile);
        page.entries = entries;
        page.records = records;
        page.rebuild();
        page
    }

    #[test]
    fn a_frozen_day_reads_no_clock_so_the_rows_cannot_move() {
        let page = page(
            vec![entry(
                "a",
                "Standup",
                "2026-10-24T09:00:00",
                "2026-10-24T09:15:00",
            )],
            Vec::new(),
        );
        assert_eq!(page.day(), day());
        let view = page.view.as_ref().expect("a view");
        assert_eq!(view.entries[0].spot, Spot::Next);
        assert!(!view.entries.iter().any(|row| row.spot == Spot::Now));
    }

    #[test]
    fn the_page_and_the_reduction_agree_about_the_day() {
        let page = page(
            Vec::new(),
            vec![record("Sjel/Task-1.md", "task", "One", false)],
        );
        let view = page.view.as_ref().expect("a view");
        assert_eq!(view.day, day());
        assert_eq!(view.focus.len(), 1);
        assert_eq!(view.projects.len(), 1, "the task names its project");
        assert!(page.loaded(), "a built view counts as loaded");
    }

    #[test]
    fn records_pushed_later_reach_the_view() {
        let mut page = page(Vec::new(), Vec::new());
        assert!(
            page.view.as_ref().expect("a view").focus.is_empty(),
            "nothing yet"
        );
        page.records = vec![record("Sjel/Task-1.md", "task", "One", false)];
        page.rebuild();
        assert_eq!(page.view.as_ref().expect("a view").focus.len(), 1);
    }
}
