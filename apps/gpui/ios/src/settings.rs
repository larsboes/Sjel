//! The state the NSH Settings screen draws (`sjel_gpui_ui::nsh_settings`), and what its presses do.

use super::*;
use sjel_gpui_ui::nsh_settings::{Choice, FoundMac, SettingsAction, SettingsState};

fn choice(id: &'static str, title: &'static str, options: Vec<(String, bool)>) -> Choice {
    Choice { id, title, options }
}

impl Capabilities {
    /// Everything Settings shows, from the shell's own fields.
    pub(super) fn settings_state(&self, cx: &gpui::App) -> SettingsState {
        let settings = self.appearance;
        let on = self.notify_prefs.enabled;
        let denied = self.notify_state.authorization == notify::Authorization::Denied;
        let reminders_sentence = match notify::disabled_reason() {
            Some(reason) => reason.to_string(),
            None if !on => "Off · nothing is announced".to_owned(),
            None => match self.notify_state.next {
                Some((at, ref title)) => format!(
                    "{} · {}",
                    self.notify_state.authorization.phrase(),
                    reminders::when(at, title, chrono::Local::now())
                ),
                None if self.notify_state.scheduled > 0 => format!(
                    "{} · {} held",
                    self.notify_state.authorization.phrase(),
                    self.notify_state.scheduled
                ),
                None => format!(
                    "{} · nothing scheduled",
                    self.notify_state.authorization.phrase()
                ),
            },
        };
        let dropped = self.notify_plan.dropped;
        let lead = self.notify_prefs.lead();
        SettingsState {
            status: self.status.to_string(),
            synced: self.synced.as_ref().map(ToString::to_string),
            paired: !self.rows.is_empty(),
            joining: self.joining,
            found: self
                .found
                .iter()
                .map(|found| FoundMac {
                    name: found.name.clone(),
                    address: found.address(),
                })
                .collect(),
            capabilities: self.rows.clone(),
            room_status: self.roomplan_status.to_string(),
            room_can_sync: self.client.is_some(),
            reminders_on: on,
            reminders_sentence,
            reminders_denied: denied,
            reminders_dropped: (on && dropped > 0).then(|| {
                format!(
                    "{} further date{} past the {} iOS holds",
                    dropped,
                    if dropped == 1 { "" } else { "s" },
                    reminders::MAX_PENDING
                )
            }),
            lead: on.then(|| {
                choice(
                    "lead",
                    "HOW EARLY",
                    reminders::Lead::ALL
                        .iter()
                        .map(|option| (option.label().to_owned(), lead == *option))
                        .collect(),
                )
            }),
            appearance: vec![
                choice(
                    "theme",
                    "THEME MODE",
                    ThemeMode::ALL
                        .iter()
                        .map(|m| (m.label().to_owned(), settings.theme_mode == *m))
                        .collect(),
                ),
                choice(
                    "accent",
                    "ACCENT COLOR",
                    AccentColor::ALL
                        .iter()
                        .map(|a| (a.label().to_owned(), settings.accent == *a))
                        .collect(),
                ),
                choice(
                    "tone",
                    "BACKGROUND TONE",
                    BackgroundTone::ALL
                        .iter()
                        .map(|t| (t.label().to_owned(), settings.tone == *t))
                        .collect(),
                ),
                choice(
                    "density",
                    "TOUCH DENSITY",
                    [Density::Mobile, Density::Comfortable]
                        .iter()
                        .map(|d| (density_label(*d).to_owned(), settings.density == *d))
                        .collect(),
                ),
            ],
            terminal: vec![
                choice(
                    "term_font",
                    "TERMINAL FONT",
                    TerminalFont::available(cx)
                        .iter()
                        .map(|f| (f.label().to_owned(), settings.terminal_font == *f))
                        .collect(),
                ),
                choice(
                    "term_size",
                    "TERMINAL FONT SIZE",
                    TERMINAL_FONT_SIZES
                        .iter()
                        .map(|s| (s.to_string(), settings.terminal_font_size == *s))
                        .collect(),
                ),
                choice(
                    "term_tone",
                    "TERMINAL BACKGROUND",
                    BackgroundTone::ALL
                        .iter()
                        .map(|t| (t.label().to_owned(), settings.terminal_tone == *t))
                        .collect(),
                ),
                choice(
                    "term_opacity",
                    "TERMINAL OPACITY",
                    WindowOpacity::ALL
                        .iter()
                        .map(|o| (o.short_label().to_owned(), settings.terminal_opacity == *o))
                        .collect(),
                ),
            ],
        }
    }

    /// One press on the Settings screen.
    pub(super) fn on_settings(&mut self, action: SettingsAction, cx: &mut Context<Self>) {
        match action {
            SettingsAction::Back => self.select_tab(Tab::Today, cx),
            SettingsAction::Scan => {
                if !self.joining {
                    self.status = "Point the camera at the code on your Mac.".into();
                    present_pair_scanner();
                }
            }
            SettingsAction::Join(ix) => {
                if let (false, Some(found)) = (self.joining, self.found.get(ix).cloned()) {
                    let _ = self.events.unbounded_send(Event::Join(found));
                }
            }
            SettingsAction::CaptureRoom => present_roomplan(),
            SettingsAction::SyncRoomScans => load_roomplan_upload(),
            SettingsAction::ToggleReminders => self.toggle_reminders(cx),
            SettingsAction::OpenSystemSettings => notify::open_settings(),
            SettingsAction::Choose { id: "lead", option } => {
                if let Some(lead) = reminders::Lead::ALL.get(option) {
                    self.notify_prefs.set_lead(*lead);
                    let _ = self.notify_prefs.save();
                    self.rebuild_reminders(cx);
                }
            }
            SettingsAction::Choose { id, option } => {
                let fonts = TerminalFont::available(cx);
                let a = &mut self.appearance;
                let applied = match id {
                    "theme" => ThemeMode::ALL.get(option).map(|v| a.theme_mode = *v),
                    "accent" => AccentColor::ALL.get(option).map(|v| a.accent = *v),
                    "tone" => BackgroundTone::ALL.get(option).map(|v| a.tone = *v),
                    "density" => [Density::Mobile, Density::Comfortable]
                        .get(option)
                        .map(|v| a.density = *v),
                    "term_font" => fonts.get(option).map(|v| a.terminal_font = *v),
                    "term_size" => TERMINAL_FONT_SIZES
                        .get(option)
                        .map(|v| a.terminal_font_size = *v),
                    "term_tone" => BackgroundTone::ALL
                        .get(option)
                        .map(|v| a.terminal_tone = *v),
                    "term_opacity" => WindowOpacity::ALL
                        .get(option)
                        .map(|v| a.terminal_opacity = *v),
                    _ => None,
                };
                if applied.is_some() {
                    let _ = self.appearance.save();
                }
            }
        }
        cx.notify();
    }
}

fn density_label(density: Density) -> &'static str {
    match density {
        Density::Mobile => "Touch 44pt",
        _ => "Compact 36pt",
    }
}
