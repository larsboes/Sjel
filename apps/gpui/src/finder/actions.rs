//! Keyboard actions, input bindings, navigation, and confirmation.
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{actions, AppContext, Context, Entity, Window};
use sjel_gpui_core::search::Mode;

use super::types::FinderEvent;
use super::Finder;

actions!(finder, [SelectUp, SelectDown, NextMode, Close]);

impl Finder {
    pub(super) fn ensure_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let input = match &self.input {
            Some((input, _)) => input.clone(),
            None => {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("Search Sjel, ask it something, or start a task…")
                });
                let subscription = cx.subscribe_in(
                    &input,
                    window,
                    |view, input, event, window, cx| match event {
                        InputEvent::Change => {
                            view.query = input.read(cx).value().to_string();
                            view.cursor = 0;
                            view.picked = false;
                            view.search_server_soon(cx);
                            cx.notify();
                        }
                        InputEvent::PressEnter { secondary, .. } => {
                            view.confirm(*secondary, window, cx);
                        }
                        _ => {}
                    },
                );
                self.input = Some((input.clone(), subscription));
                input
            }
        };
        if std::mem::take(&mut self.focus_next) {
            let empty = self.query.is_empty();
            input.update(cx, |input, cx| {
                if empty && !input.value().is_empty() {
                    input.set_value("", window, cx);
                }
                input.focus(window, cx);
            });
        }
        input
    }

    /// Enter: open the selected result, or ask. `other_way` is ⌘↩, which flips the decision.
    ///
    /// In *Auto* a search that finds nothing asks instead: an empty list is never the answer to a
    /// text someone pressed Enter on.
    pub(super) fn confirm(&mut self, other_way: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.query.trim().to_owned();
        // Under an answer an empty Enter is not a request to open the first page.
        if text.is_empty() && self.asked {
            return;
        }
        // Agent is never what Enter does in Auto: it starts a run that edits files.
        if self.resolved() == Mode::Agent && !self.picked {
            if text.is_empty() {
                return;
            }
            if self.agent.running {
                self.agent_steer(text, window, cx);
            } else {
                self.agent_send(text, window, cx);
            }
            return;
        }
        let mut ask = self.resolved() == Mode::Ask && !self.picked;
        if other_way {
            ask = !ask;
        }
        let hit = self.hits().get(self.cursor).map(|hit| hit.open.clone());
        if !ask && hit.is_none() && self.mode == Mode::Auto && !text.is_empty() {
            ask = true;
        }
        if ask {
            if text.is_empty() {
                return;
            }
            self.ask.update(cx, |ask, cx| ask.submit(text, cx));
            self.asked = true;
            self.thread_scroll.scroll_to_bottom();
            self.query.clear();
            if let Some((input, _)) = &self.input {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            cx.notify();
            return;
        }
        if let Some(open) = hit {
            self.close(cx);
            cx.emit(FinderEvent::Open(open));
        }
    }

    pub(super) fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.hits().len();
        if count == 0 {
            return;
        }
        // The first arrow press picks the row the cursor is on; later ones move it.
        if self.picked {
            self.cursor = (self.cursor as isize + by).rem_euclid(count as isize) as usize;
        }
        self.picked = true;
        cx.notify();
    }

    /// Bindings the finder carries. Up and down reach it because a one-line input lets them
    /// through, the same way the kit's command palette receives them. Escape is the
    /// workspace's window-wide binding, which closes an open finder first.
    pub fn keys() -> [gpui_kit::KeyBinding; 5] {
        [
            gpui_kit::KeyBinding::new("up", SelectUp, Some("Finder")),
            gpui_kit::KeyBinding::new("down", SelectDown, Some("Finder")),
            gpui_kit::KeyBinding::new("ctrl-p", SelectUp, Some("Finder")),
            gpui_kit::KeyBinding::new("ctrl-n", SelectDown, Some("Finder")),
            gpui_kit::KeyBinding::new("cmd-w", Close, Some("Finder")),
        ]
    }

    /// Tab is the input's own key (it indents), so the mode cycles on ⌘/ instead.
    pub fn mode_key() -> gpui_kit::KeyBinding {
        gpui_kit::KeyBinding::new("cmd-/", NextMode, Some("Finder"))
    }
}
