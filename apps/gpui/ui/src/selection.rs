//! Keyboard selection: J/K move a cursor over the rows a page draws, Enter runs the row's
//! primary action, ⌘⌫ its costly one. The selected row reveals the actions a mouse gets on hover.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::ElementExt;
use gpui_kit::{point, prelude::*, px, rgba, Div, ScrollHandle};

use crate::theme::{tint, Palette};

/// One row the keyboard can reach, in the order the page draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// Enter allows, ⌘⌫ denies.
    Approval(String),
    /// A kept-running capability that is down. Enter starts it.
    Down(String),
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
        row.bg(rgba(tint(palette.accent, 0x24)))
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
