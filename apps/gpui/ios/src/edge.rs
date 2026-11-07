//! The right edge: soundscape, activity, agent and terminal seats over the page.

use super::*;

impl Capabilities {
    /// Opens the right edge on `seat` (`ui/src/sidebar.rs`).
    ///
    /// The audio device is taken here and not at launch: claiming the audio route is a visible act,
    /// and belongs to opening the panel that plays. Switching seats keeps the sound running — the
    /// edge is beside the page, not away from it — and only closing it stops the sound.
    pub(super) fn open_edge(&mut self, seat: Seat, cx: &mut Context<Self>) {
        self.sidebar.open_on(seat);
        if seat == Seat::Soundscape && !self.audio_started {
            self.audio_started = true;
            match audio::start() {
                Ok(device) => self
                    .soundscape
                    .update(cx, |drawer, cx| drawer.set_device(Some(device), None, cx)),
                Err(why) => self
                    .soundscape
                    .update(cx, |drawer, cx| drawer.set_device(None, Some(why), cx)),
            }
        }
        if seat == Seat::Activity {
            // The activity seat's numbers are the system's and the last sync's, so they are asked for
            // again as the seat comes into view rather than drawn from an hour ago.
            notify::report();
        }
        if seat == Seat::Agent {
            self.agent.update(cx, |seat, cx| seat.refresh_sessions(cx));
        }
        if seat == Seat::Terminal {
            if self.terminal_unlocked {
                self.open_terminal(cx);
            } else if self.client.is_some() {
                self.terminal_note = Some("Confirm with Face ID to open the terminal.".into());
                unlock();
            }
        }
        cx.notify();
    }

    /// The newest live terminal on the Mac, or a new login shell, in the edge.
    /// The Mac's terminals in the edge, as tabs: every live one, or a new login shell.
    pub(super) fn open_terminal(&mut self, cx: &mut Context<Self>) {
        if self.terminal.is_some() {
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        let deck = cx.new(|cx| {
            TerminalDeck::new(Host::Device(client), true, TERMINAL_COLS, TERMINAL_ROWS, cx)
        });
        let subscription = cx.subscribe(&deck, |view, _, event: &TerminalEvent, cx| match event {
            TerminalEvent::Editing(open) => keyboard(*open),
            TerminalEvent::Zoom(steps) => view.zoom_terminal(*steps, cx),
        });
        deck.update(cx, |deck, cx| deck.load(cx));
        self.terminal = Some((deck, subscription));
        self.terminal_note = None;
    }

    /// Steps the terminal's font size and keeps it, the way the Settings row does.
    pub(super) fn zoom_terminal(&mut self, steps: i32, cx: &mut Context<Self>) {
        let size = step_terminal_font_size(self.appearance.terminal_font_size, steps);
        if size != self.appearance.terminal_font_size {
            self.appearance.terminal_font_size = size;
            let _ = self.appearance.save();
            cx.notify();
        }
    }

    pub(super) fn agent_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Entity<InputState> {
        if let Some((input, _)) = &self.agent_input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Ask the agent to work…"));
        let subscription = cx.subscribe_in(&input, window, |view, input, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let prompt = input.read(cx).value().trim().to_owned();
                if prompt.is_empty() {
                    return;
                }
                input.update(cx, |input, cx| input.set_value("", window, cx));
                view.agent.update(cx, |seat, cx| seat.submit(prompt, cx));
            }
        });
        self.agent_input = Some((input.clone(), subscription));
        input
    }

    /// Puts the edge away, and stops the sound with it: a panel that is not on screen is not a thing
    /// that should still be playing in a pocket.
    pub(super) fn close_edge(&mut self, cx: &mut Context<Self>) {
        if self.sidebar.seat() == Seat::Terminal {
            keyboard(false);
        }
        self.sidebar.close();
        self.soundscape.update(cx, |drawer, cx| drawer.pause(cx));
        cx.notify();
    }

    /// The activity seat: what this phone is doing, in its own words.
    ///
    pub(super) fn activity_panel(&self, palette: Palette) -> AnyElement {
        let pending = self.board.as_ref().map_or(0, Board::pending);
        let paired = self.client.is_some();
        let with_mac: Vec<String> = vec![
            self.status.to_string(),
            match (&self.synced, paired) {
                (Some(synced), _) => synced.to_string(),
                (None, true) => "Paired. Nothing has moved this launch.".to_owned(),
                (None, false) => "Not paired: this phone keeps its own records.".to_owned(),
            },
            if pending > 0 {
                format!("{pending} queued for the Mac")
            } else {
                "Nothing queued".to_owned()
            },
        ];
        // One coherent answer, not two lines that contradict each other: with the switch off, the
        // permission's state is the Settings card's business, not a "Allowed" above "Reminders are
        // off".
        let mut reminders: Vec<String> = Vec::new();
        match notify::disabled_reason() {
            Some(reason) => reminders.push(reason),
            None if !self.notify_prefs.enabled => reminders.push("Reminders are off".to_owned()),
            None => {
                reminders.push(self.notify_state.authorization.phrase().to_owned());
                for reminder in self.notify_plan.reminders.iter().take(3) {
                    reminders.push(reminders::when(
                        reminder.at,
                        &reminder.title,
                        chrono::Local::now(),
                    ));
                }
                if reminders.len() == 1 {
                    reminders.push("Nothing scheduled".to_owned());
                }
            }
        }
        let section = |title: &'static str, lines: Vec<String>| {
            let mut column = div().flex().flex_col().gap_1().child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(title),
            );
            for line in lines {
                column = column.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child(SharedString::from(line)),
                );
            }
            column
        };
        div()
            .id("activity")
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .pt(px(12.))
            .overflow_y_scroll()
            .child(section("WITH THE MAC", with_mac))
            .child(section("REMINDERS", reminders))
            .into_any_element()
    }

    /// The paired phone can start, follow, and approve a run held by the Mac. The seat draws the
    /// run; the phone adds its text field and a send button above it.
    pub(super) fn agent_panel(
        &mut self,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let seat = self.agent.clone();
        if !seat.read(cx).connected() {
            return seat.into_any_element();
        }
        let running = seat.read(cx).run.running;
        let input = self.agent_input(window, cx);
        div()
            .id("agent-activity")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .child(div().flex_1().child(Input::new(&input)))
                    .child(
                        div()
                            .id("phone-agent-send")
                            .min_h(px(44.))
                            .flex()
                            .items_center()
                            .px_3()
                            .rounded(px(7.))
                            .bg(rgba(palette.raised))
                            .text_color(rgb(palette.text))
                            .child(if running { "Guide" } else { "Start" })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, window, cx| {
                                    let Some((input, _)) = &view.agent_input else {
                                        return;
                                    };
                                    let prompt = input.read(cx).value().trim().to_owned();
                                    if prompt.is_empty() {
                                        return;
                                    }
                                    input.update(cx, |input, cx| input.set_value("", window, cx));
                                    view.agent.update(cx, |seat, cx| seat.submit(prompt, cx));
                                }),
                            ),
                    ),
            )
            .child(seat)
            .into_any_element()
    }
}
