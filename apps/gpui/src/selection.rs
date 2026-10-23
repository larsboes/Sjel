//! Keyboard selection: J/K move a cursor over the rows a page draws, Enter runs the row's
//! primary action, ⌘⌫ its costly one. The selected row reveals the actions a mouse gets on hover.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::ElementExt;
use gpui_kit::{point, prelude::*, px, rgba, Div, ScrollHandle};

use crate::calendar;
use crate::route::Page;
use crate::shell::{CalendarEntry, GlobalTask};
use crate::theme::Palette;
use crate::workspace::Workspace;

/// One row the keyboard can reach, in the order the page draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// Enter allows, ⌘⌫ denies.
    Approval(String),
    /// A kept-running capability that is down. Enter starts it.
    Down(String),
    /// The calendar entry at this place in `Workspace::ahead`. Enter opens the calendar in the
    /// dashboard.
    Entry(usize),
    /// Urgent or due-today task shown on Home. Enter toggles completion.
    FocusTask(String),
    /// Enter starts a stopped capability, ⌘⌫ stops a running one.
    Capability { name: String, up: bool },
}

/// Cursor state, shared with the selected row's prepaint so a keyboard move scrolls it into view.
pub struct Selection {
    pub cursor: Option<usize>,
    pub scroll: ScrollHandle,
    reveal: Rc<Cell<bool>>,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            cursor: None,
            scroll: ScrollHandle::new(),
            reveal: Rc::default(),
        }
    }
}

impl Selection {
    /// Moves by `step` within `len` rows; the first move selects the first row.
    pub fn step(&mut self, step: isize, len: usize) {
        if len == 0 {
            self.cursor = None;
            return;
        }
        let last = len as isize - 1;
        self.cursor = Some(match self.cursor {
            None => 0,
            Some(at) => (at as isize + step).clamp(0, last) as usize,
        });
        self.reveal.set(true);
    }

    pub fn clear(&mut self) {
        self.cursor = None;
    }

    /// A new page starts at the top with nothing selected.
    pub fn reset(&mut self) {
        self.cursor = None;
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    /// Highlights `row` when it is selected, and scrolls it into view after a keyboard move.
    pub fn mark(&self, row: Div, selected: bool, palette: Palette) -> Div {
        if !selected {
            return row;
        }
        let (reveal, scroll) = (self.reveal.clone(), self.scroll.clone());
        row.bg(rgba(palette.accent << 8 | 0x24))
            .on_prepaint(move |bounds, window, _| {
                if !reveal.replace(false) {
                    return;
                }
                let view = scroll.bounds();
                let margin = px(12.);
                let shift = if bounds.top() < view.top() + margin {
                    view.top() + margin - bounds.top()
                } else if bounds.bottom() > view.bottom() - margin {
                    view.bottom() - margin - bounds.bottom()
                } else {
                    return;
                };
                let offset = scroll.offset();
                scroll.set_offset(point(offset.x, offset.y + shift));
                window.refresh();
            })
    }
}

impl Workspace {
    /// The rows of the current page, in drawing order. A page highlights a row by comparing its
    /// `Item` with `selected`, never by position, so Enter acts on the highlighted row even when
    /// this order and the drawing order differ.
    pub fn items(&self) -> Vec<Item> {
        match self.page {
            Page::Home => {
                let mut items: Vec<Item> = self
                    .agent
                    .value
                    .iter()
                    .flat_map(|agent| agent.pending.iter().map(|a| Item::Approval(a.id.clone())))
                    .collect();
                if let Some(health) = &self.health.value {
                    items.extend(health.down().into_iter().map(Item::Down));
                }
                items.extend((0..self.ahead().len()).map(Item::Entry));
                items.extend(
                    self.urgent_tasks()
                        .into_iter()
                        .map(|t| Item::FocusTask(t.id)),
                );
                items
            }
            Page::Capabilities => self
                .health
                .value
                .iter()
                .flat_map(|health| health.capabilities.iter())
                .map(|(name, state)| Item::Capability {
                    name: name.clone(),
                    up: state.up,
                })
                .collect(),
            Page::Projects
            | Page::Context
            | Page::Ask
            | Page::Travel
            | Page::Feed
            | Page::Inbox
            | Page::Calendar
            | Page::Documents
            | Page::Backup
            | Page::Finance
            | Page::People
            | Page::Inventory
            | Page::Software
            | Page::Import
            | Page::Systems
            | Page::Packs
            | Page::SelfModel
            | Page::Settings => Vec::new(),
        }
    }

    /// The row under the cursor.
    pub fn selected(&self) -> Option<Item> {
        self.items().get(self.selection.cursor?).cloned()
    }

    /// Calendar entries still ahead, as Home draws them.
    pub fn ahead(&self) -> Vec<CalendarEntry> {
        let now = chrono::Local::now().naive_local();
        self.calendar
            .value
            .clone()
            .map(|e| calendar::ahead(e, now))
            .unwrap_or_default()
    }

    /// Urgent or due-today tasks, as Home draws them.
    pub fn urgent_tasks(&self) -> Vec<GlobalTask> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.focus_tasks
            .value
            .as_ref()
            .map(|tasks| {
                tasks
                    .iter()
                    .filter(|t| !t.done)
                    .filter(|t| {
                        t.priority == 1
                            || t.due
                                .as_deref()
                                .is_some_and(|d| !d.is_empty() && d <= today.as_str())
                    })
                    .take(6)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_clamp_and_first_move_selects_the_top() {
        let mut selection = Selection::default();
        selection.step(-1, 3);
        assert_eq!(selection.cursor, Some(0));
        selection.step(1, 3);
        selection.step(1, 3);
        selection.step(1, 3);
        assert_eq!(selection.cursor, Some(2));
        selection.step(1, 0);
        assert_eq!(selection.cursor, None);
    }
}
