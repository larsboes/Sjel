//! Today, the phone's landing page: the day the Mac's copy answered, the tasks the replica holds,
//! and the projects those tasks belong to.
//!
//! It is the page a phone opens on, so it answers three questions in the order they are asked on a
//! morning: what is on today, what is next to do, and which project the next thing belongs to.
//! Nothing here is a dashboard: no chart, no total, no greeting. Every row is a thing that exists,
//! and every section says how much of itself is empty rather than drawing nothing.
//!
//! Two sources meet on this page and are named apart in the line under the title. The day comes
//! from the Mac's read-only copy, which the phone fetches; the tasks come from the phone's own
//! replica, which survives being offline. The page knows nothing about the network: its host hands
//! it the copy and the records, and listens for [`TodayEvent`].

use std::sync::Arc;

use chrono::{Local, NaiveDate};
use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, EventEmitter, FontWeight, MouseButton,
    Render, SharedString, Window,
};
use gpui_kit::component::checkbox::Checkbox;
use sjel_gpui_core::calendar;
use sjel_gpui_core::data::{Busy, Repo};
use sjel_gpui_core::mirror::Mirror;
use sjel_gpui_core::shell::{self, CalendarEntry, ProjectRecord};
use sjel_gpui_core::today::{self, Entry, Project, Spot, Task, Today as View};

use crate::data::{self, Press};
use crate::primitives::{priority_pill, progress_bar};
use crate::{Density, Palette};

/// What the page asks its host to do. The host owns the network, the copy on disk and the sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TodayEvent {
    /// The person tapped the day's header or an entry: show that day in the Calendar tab.
    OpenDay(NaiveDate),
    /// The person tapped a project: open it in the Projects page.
    OpenProject(String),
    /// The person tapped the gear, or the banner that says this phone is not paired yet.
    Settings,
    /// The person tapped the note: the soundscape seat of the right edge, which the host slides in.
    Soundscape,
    /// The person tapped the list: the activity seat of the same edge — what this phone is doing.
    Activity,
    /// The person asked for newer data.
    Refresh,
    /// A task was written here and is waiting for the Mac, so the host may sync now.
    Wrote,
}

/// The phone's landing page.
pub struct Today {
    /// The Mac's read-only copy, when one is on disk.
    copy: Option<Arc<Mirror>>,
    /// Why the copy could not be opened, in the words the person needs.
    missing: Option<SharedString>,
    /// A host's word about this copy, e.g. why a newer one could not be fetched.
    note: Option<SharedString>,
    /// The phone's own replica, when it opened. Without it a task cannot be ticked off, and the
    /// page says so instead of offering a checkbox that would forget the tap.
    store: Option<Repo>,
    /// The records the replica answered: tasks and project names.
    records: Vec<ProjectRecord>,
    /// The day's entries, from the copy.
    entries: Vec<CalendarEntry>,
    /// The reduction, rebuilt whenever either source moves.
    view: Option<View>,
    /// A failed read, kept beside the rows that answered before it.
    error: Option<SharedString>,
    /// How many writes are waiting for the Mac, so the page can say a task will travel.
    pending: usize,
    /// Whether this launch has reached the Mac at all.
    paired: bool,
    /// True while the day is being read, so the header can say so.
    loading: bool,
    /// A day a launch flag fixed (`SJEL_DAY`), for a screenshot of a day that is not today. A phone
    /// never sets it, so the live page reads the clock on every rebuild.
    frozen: Option<NaiveDate>,
    busy: Busy,
    /// The last tick written ahead of the store: the record's path and the value sent. A write
    /// that fails puts the row back, because the replica still has the old one.
    optimistic: Option<(String, bool)>,
    palette: Palette,
    density: Density,
    /// Bumped per read, so a slow answer for a day the person has left cannot replace the day on
    /// screen.
    generation: u64,
}

impl EventEmitter<TodayEvent> for Today {}

impl Today {
    pub fn new(density: Density) -> Self {
        Self::with_day(None, density)
    }

    /// The page opened on `day` rather than on the clock. A screenshot or a touch check of a day
    /// that is not today needs this; `ios/run-sim` sets it from `SJEL_DAY`.
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
            density,
            generation: 0,
        }
    }

    /// Hands the page the Mac's copy, or the reason it is not there.
    ///
    /// A newer copy replaces the old one without the page forgetting which day it is on: a refresh
    /// that lands while somebody is reading moves the entries, not the page.
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

    /// The replica's records, pushed by the host after every read and every sync. The page does not
    /// read the store itself: `Board` already holds the rows, and a second reader could only
    /// disagree with the one that drew the queue count.
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

    /// A sentence about the data on screen from the host. A failed fetch does not take away the day
    /// already there: it is still true, only older, and the line above says how old.
    pub fn set_note(&mut self, note: Option<String>, cx: &mut Context<Self>) {
        self.note = note.map(SharedString::from);
        cx.notify();
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != palette {
            self.palette = palette;
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
    ///
    /// Nothing happens without a copy: a phone that has never reached its Mac says so through
    /// [`Self::set_copy`]'s `missing` rather than drawing an empty day that looks like an answer.
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

    /// Rebuilds the reduction from the two sources. Called after every read and every write, so
    /// the rows never depend on which answer came last.
    fn rebuild(&mut self) {
        let day = self.day();
        // A frozen day is a day somebody asked for, not the day it is: no clock, so nothing is
        // "now" and the screenshot cannot disagree with itself a minute later.
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
    ///
    /// The row moves in the frame the tap was made in and moves back if the write fails: a checkbox
    /// that waits for an answer before it shows the tap reads as a dropped tap on a phone.
    fn toggle(&mut self, path: String, done: bool, window: &mut Window, cx: &mut Context<Self>) {
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

    /// What one task write left behind: the record the store answered with, or the tick put back
    /// where it was because the write did not land. The toast has already said why.
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

    /// One round button in the header. `event` is what a tap asks the host for: the gear and the note
    /// are the same control drawn twice, so neither can drift.
    fn header_button(
        &self,
        id: &'static str,
        glyph: &'static str,
        event: TodayEvent,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        div()
            .id(id)
            .flex_none()
            .size(px(36.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(rgb(palette.muted))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(glyph)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| cx.emit(event.clone())),
            )
            .into_any_element()
    }

    /// The line under the title: where the data came from, and whether a task ticked here will
    /// travel. One sentence, in the order the question is asked.
    ///
    /// The way to the pairing screen is a row of its own, and only when the phone has neither a
    /// pairing nor a copy: with a copy on disk the page is readable, and a button that says "Set up"
    /// beside readable data reads as "this is broken".
    fn source_line(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let source = match (&self.error, &self.copy, &self.missing) {
            (Some(error), _, _) => format!("The day could not be read: {error}"),
            (_, Some(copy), _) => format!(
                "Calendar from the Mac's copy, {}",
                copy.manifest().built_label()
            ),
            (_, None, Some(missing)) => missing.to_string(),
            (_, None, None) => "Looking for the Mac's copy…".to_owned(),
        };
        let mut column = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().text_color(rgb(palette.muted)).child(source));
        if let Some(note) = self.note.clone() {
            column = column.child(div().text_xs().text_color(rgb(palette.amber)).child(note));
        }
        // One sentence for the state of the travel, not one line per flag: what is queued and
        // whether anything is going anywhere are the same question.
        let mut travel: Vec<String> = Vec::new();
        if self.pending > 0 {
            travel.push(format!("{} waiting for the Mac", self.pending));
        }
        if !self.paired {
            travel.push("not connected, so changes stay here".to_owned());
        }
        if !travel.is_empty() {
            column = column.child(
                div()
                    .text_xs()
                    .text_color(rgb(if self.pending > 0 {
                        palette.amber
                    } else {
                        palette.muted
                    }))
                    .child(travel.join(" · ")),
            );
        }
        let mut block = div().flex().flex_col().gap_2().child(column);
        if !self.paired && self.copy.is_none() {
            block = block.child(
                div()
                    .id("today-pair")
                    .min_h(px(44.))
                    .rounded(px(10.))
                    .bg(rgba(palette.accent))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xffffff))
                    .child("Pair this phone with your Mac")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.emit(TodayEvent::Settings)),
                    ),
            );
        }
        block.into_any_element()
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let view = self.view.as_ref();
        let headline = view
            .map(View::headline)
            .unwrap_or_else(|| self.day().format("%A, %-d %B").to_string());
        let summary = view.map(View::summary).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("TODAY"),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(headline),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(summary),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.header_button(
                                "today-soundscape",
                                "♪",
                                TodayEvent::Soundscape,
                                cx,
                            ))
                            .child(self.header_button(
                                "today-activity",
                                "≡",
                                TodayEvent::Activity,
                                cx,
                            ))
                            .child(self.header_button(
                                "today-settings",
                                "⚙",
                                TodayEvent::Settings,
                                cx,
                            )),
                    ),
            )
            .child(self.source_line(cx))
            .into_any_element()
    }

    fn entry_row(&self, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let day = self.day();
        let rail = calendar::kind_color(&entry.kind);
        let mark = match entry.spot {
            Spot::Now => Some("NOW"),
            Spot::Next => Some("NEXT"),
            Spot::Past | Spot::Later => None,
        };
        // A commitment is the difference between "I could" and "I must", and colour alone does not
        // say it in daylight; the mark is a word for the same reason.
        let title_color = match entry.spot {
            Spot::Past => palette.muted,
            _ => palette.text,
        };
        div()
            .id(SharedString::from(format!("entry-{}", entry.id)))
            .flex()
            .items_center()
            .gap_3()
            .px(self.density.pad_x())
            .py(self.density.pad_y())
            .min_h(self.density.row_height())
            .cursor_pointer()
            .child(
                div()
                    .flex_none()
                    .w(px(3.))
                    .h(px(28.))
                    .rounded_full()
                    .bg(rgb(rail)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(title_color))
                            .when(entry.spot == Spot::Past, |title| title.line_through())
                            .child(entry.title.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("{} · {}", entry.when, entry.where_)),
                    ),
            )
            .when_some(mark, |row, mark| {
                row.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py(px(2.))
                        .rounded(px(6.))
                        .bg(rgba(palette.accent))
                        .text_color(rgb(0xffffff))
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(mark),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| cx.emit(TodayEvent::OpenDay(day))),
            )
            .into_any_element()
    }

    fn task_row(&self, task: &Task, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let writable = self.store.is_some();
        let path = task.path.clone();
        // The due date is the badge's business. Saying it twice on one row costs the title its
        // width, which is the only thing on the row the person cannot get anywhere else.
        let second = task.project.clone();
        div()
            .id(("today-task", index))
            .flex()
            .items_center()
            .gap_3()
            .px(self.density.pad_x())
            .py(self.density.pad_y())
            .min_h(self.density.row_height())
            .when(!writable, |row| row.opacity(0.6))
            .child(
                div().flex_none().child(
                    Checkbox::new(("today-task-done", index))
                        .checked(false)
                        .on_click(cx.listener(move |view, done: &bool, window, cx| {
                            if view.store.is_none() {
                                return;
                            }
                            view.toggle(path.clone(), *done, window, cx)
                        })),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .truncate()
                            .child(task.title.clone()),
                    )
                    .when(!second.is_empty(), |column| {
                        column.child(div().text_xs().text_color(rgb(palette.muted)).child(second))
                    }),
            )
            .when_some(
                task.due
                    .as_deref()
                    .map(|due| due_mark(due, task.overdue, palette)),
                |row, mark| row.child(mark),
            )
            .when(task.priority < 2, |row| {
                row.child(priority_pill(task.priority, palette))
            })
            .into_any_element()
    }

    fn project_row(&self, project: &Project, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let name = project.name.clone();
        div()
            .id(("today-project", index))
            .flex()
            .items_center()
            .gap_3()
            .px(self.density.pad_x())
            .py(self.density.pad_y())
            .min_h(self.density.row_height())
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .truncate()
                            .child(project.title.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(project.line()),
                    ),
            )
            .child(div().flex_none().child(progress_bar(
                project.progress(),
                palette.accent,
                palette,
            )))
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("→"),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| cx.emit(TodayEvent::OpenProject(name.clone()))),
            )
            .into_any_element()
    }

    /// One section with its rows, or the sentence that says why it is empty. An empty list that
    /// draws nothing reads as a page that failed to load.
    fn section(
        &self,
        title: &'static str,
        empty: &'static str,
        rows: Vec<AnyElement>,
    ) -> AnyElement {
        let palette = self.palette;
        let body = if rows.is_empty() {
            div()
                .rounded(px(12.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .px_4()
                .py_3()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(empty)
                .into_any_element()
        } else {
            crate::list(rows, palette)
        };
        crate::section(title, body, palette).into_any_element()
    }

    fn refresh(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        div()
            .id("today-refresh")
            .text_xs()
            .text_color(rgb(palette.accent))
            .cursor_pointer()
            .child(if self.loading {
                "Reading…"
            } else {
                "Refresh"
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(TodayEvent::Refresh)),
            )
            .into_any_element()
    }
}

/// The due date in the width a phone row can spare: `Overdue` when it has passed, else `30 Sep`.
///
/// `due_badge` says `Overdue · 2026-09-30`, which is the right thing on a table with a DUE column
/// and the wrong one on a row whose title needs that width. Colour alone does not say "late" in
/// daylight, so the overdue case is a word.
fn due_mark(due: &str, overdue: bool, palette: Palette) -> AnyElement {
    if overdue {
        return div()
            .flex_none()
            .px_2()
            .py(px(2.))
            .rounded(px(6.))
            .bg(rgba(palette.red & 0xffffff | 0x22000000))
            .text_color(rgb(palette.red))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child("Overdue")
            .into_any_element();
    }
    let label = sjel_gpui_core::calendar::parse_day(due)
        .map(|day| day.format("%-d %b").to_string())
        .unwrap_or_else(|| due.to_owned());
    div()
        .flex_none()
        .text_xs()
        .text_color(rgb(palette.muted))
        .child(label)
        .into_any_element()
}

impl Render for Today {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let view = self.view.clone();
        let mut page = div()
            .id("today")
            .flex()
            .flex_col()
            .gap_4()
            .text_color(rgb(palette.text))
            .child(self.header(cx));

        let Some(view) = view else {
            // Nothing has been read yet. One sentence, not three empty sections that look like
            // answers.
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

impl Today {
    /// A section body whose empty case is the same card as [`Self::section`], plus the count of
    /// tasks that did not fit.
    fn section_content(
        &self,
        empty: &'static str,
        rows: Vec<AnyElement>,
        rest: usize,
    ) -> AnyElement {
        let palette = self.palette;
        let body = if rows.is_empty() {
            div()
                .rounded(px(12.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .px_4()
                .py_3()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(empty)
                .into_any_element()
        } else {
            crate::list(rows, palette)
        };
        match rest {
            0 => body,
            more => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(body)
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("and {more} more in Projects")),
                )
                .into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_gpui_core::shell::CalendarEntry;

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

    /// A page with a frozen day and whatever the two sources answered. The `Context` a page method
    /// needs is not available in a unit test, so the reduction is driven directly: what this checks
    /// is that the page and the reduction agree about the day, not that a row was drawn.
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
        // Without a clock the first timed entry takes the pointer and nothing claims to be running.
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
