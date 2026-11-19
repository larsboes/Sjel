use chrono::{DateTime, Local, NaiveDateTime, TimeDelta, TimeZone, Timelike};
use reqwest::Method;
use soundscape_api::bedside::{
    BedsideOptions, SleepEnding, ALL_NIGHT_MS, BACK_TO_SLEEP, BACK_TO_SLEEP_MS, BEDSIDE,
    FADE_OUT_MS,
};
use soundscape_api::{Preset, SoundscapeSession, SoundscapeState, PROXY_ROUTE_SOUNDSCAPE_STATE};

use crate::shell::{Shell, ShellError, ShellResult};

/// A timed starting point for a listening session.
#[derive(Clone, Copy)]
pub struct Scenario {
    pub key: &'static str,
    pub label: &'static str,
    pub minutes: u64,
    pub preset: Preset,
    energy: f32,
}

impl Scenario {
    pub const DEEP_WORK: Self = Self {
        key: "deep-work",
        label: "Deep focus",
        minutes: 50,
        preset: Preset::Focus,
        energy: 0.72,
    };
    pub const READING: Self = Self {
        key: "reading",
        label: "Reading",
        minutes: 25,
        preset: Preset::Focus,
        energy: 0.42,
    };
    pub const RESET: Self = Self {
        key: "reset",
        label: "Reset",
        minutes: 10,
        preset: Preset::Relax,
        energy: 0.38,
    };
    pub const WIND_DOWN: Self = Self {
        key: "wind-down",
        label: "Wind down",
        minutes: 45,
        preset: Preset::Sleep,
        energy: 0.24,
    };
    pub const ALL: [Self; 4] = [Self::DEEP_WORK, Self::READING, Self::RESET, Self::WIND_DOWN];

    /// Label a saved scenario, including the generic timer.
    pub fn label_for(key: &str) -> &'static str {
        match key {
            BEDSIDE => return "Bedside",
            BACK_TO_SLEEP => return "Back to sleep",
            _ => {}
        }
        Self::ALL
            .iter()
            .find(|scenario| scenario.key == key)
            .map_or("timer", |scenario| scenario.label)
    }
}

/// A local recommendation and mix input derived from time and calendar occupancy.
#[derive(Clone, Copy)]
pub struct ListeningContext {
    pub scenario: Scenario,
    pub activity: f32,
    pub reason: &'static str,
}

impl ListeningContext {
    /// Derive a recommendation from the current local hour and any active committed block.
    pub fn at(hour: u32, occupied: bool) -> Self {
        let (scenario, activity, reason) = match (hour, occupied) {
            (21..=23 | 0..=6, _) => (Scenario::WIND_DOWN, 0.15, "Nighttime"),
            (_, true) => (Scenario::RESET, 0.15, "Calendar busy"),
            (17..=20, false) => (Scenario::RESET, 0.25, "Evening"),
            _ => (Scenario::DEEP_WORK, 0.65, "Daytime"),
        };
        Self {
            scenario,
            activity,
            reason,
        }
    }
}

impl Shell {
    /// Select a mode and apply its scene defaults in the same request.
    pub async fn select_soundscape_mode(&self, preset: Preset) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "preset": preset.key(),
            "params": preset.scape().map(|scape| scape.defaults),
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }

    /// Fetch local calendar occupancy. Time of day supplies the offline fallback.
    pub async fn soundscape_context(&self) -> ListeningContext {
        let now = Local::now().naive_local();
        let day = now.date();
        let Some(next) = day.succ_opt() else {
            return ListeningContext::at(now.hour(), false);
        };
        let entries = self.calendar(&day.to_string(), &next.to_string()).await;
        let occupied = entries.is_ok_and(|entries| {
            entries.iter().any(|entry| {
                !entry.all_day
                    && entry.commitment == "committed"
                    && entry
                        .starts_at
                        .parse::<NaiveDateTime>()
                        .is_ok_and(|start| start <= now)
                    && entry
                        .ends_at
                        .parse::<NaiveDateTime>()
                        .is_ok_and(|end| now < end)
            })
        });
        ListeningContext::at(now.hour(), occupied)
    }

    /// Start the transport and apply a scenario's preset and parameters in one patch.
    pub async fn start_soundscape_scenario(
        &self,
        scenario: Scenario,
    ) -> ShellResult<SoundscapeState> {
        let session = SoundscapeSession {
            scenario: scenario.key.into(),
            duration_ms: scenario.minutes * 60_000,
            elapsed_ms: 0,
            running_since_ms: None,
            bedside: None,
            deadline_ms: None,
        };
        self.start_timed_soundscape(scenario.preset, scenario.energy, session)
            .await
    }

    /// Start a Bedside session or its short restart.
    pub async fn start_bedside(
        &self,
        options: BedsideOptions,
        back_to_sleep: bool,
    ) -> ShellResult<SoundscapeState> {
        let (scenario, duration_ms) = if back_to_sleep {
            (BACK_TO_SLEEP, BACK_TO_SLEEP_MS)
        } else {
            let duration = match options.ending {
                SleepEnding::FadeOut => FADE_OUT_MS,
                SleepEnding::AllNight => ALL_NIGHT_MS,
                SleepEnding::UntilMorning => local_morning_duration(options.morning_minute)?,
            };
            (BEDSIDE, duration)
        };
        self.start_timed_soundscape(
            Preset::Sleep,
            0.24,
            SoundscapeSession {
                scenario: scenario.into(),
                duration_ms,
                elapsed_ms: 0,
                running_since_ms: None,
                bedside: Some(options),
                deadline_ms: None,
            },
        )
        .await
    }

    async fn start_timed_soundscape(
        &self,
        preset: Preset,
        energy: f32,
        session: SoundscapeSession,
    ) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({
            "preset": preset.key(),
            "params": preset.scape().map(|scape| scape.defaults),
            "energy": energy,
            "playing": true,
            "session": session,
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }

    /// Stop the Bedside transport and clear its timer in one patch.
    pub async fn stop_bedside(&self) -> ShellResult<SoundscapeState> {
        let body = serde_json::json!({ "playing": false, "session": null, "origin": "gpui" });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }

    /// Repeat the conductor's last timed session from the beginning.
    pub async fn repeat_soundscape_session(&self) -> ShellResult<SoundscapeState> {
        let state = self.soundscape_state().await?;
        let morning = state
            .last_session
            .filter(|saved| saved.scenario == BEDSIDE)
            .and_then(|saved| saved.bedside)
            .filter(|options| options.ending == SleepEnding::UntilMorning)
            .map(|options| local_morning_duration(options.morning_minute))
            .transpose()?;
        let body = serde_json::json!({
            "repeat_last_session": true,
            "repeat_morning_duration_ms": morning,
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }
}

fn local_morning_duration(minute: u16) -> ShellResult<u64> {
    morning_duration(Local::now(), minute)
        .ok_or_else(|| ShellError::Refused(400, Some("Invalid morning time".into())))
}

fn morning_duration<Tz: TimeZone>(now: DateTime<Tz>, minute: u16) -> Option<u64> {
    let time = chrono::NaiveTime::from_num_seconds_from_midnight_opt(u32::from(minute) * 60, 0)?;
    let today = now.date_naive();
    for day in [today, today.succ_opt()?] {
        let target = day.and_time(time);
        for shift in 0..=180 {
            let local = target.checked_add_signed(TimeDelta::minutes(shift))?;
            let candidates = match now.timezone().from_local_datetime(&local) {
                chrono::LocalResult::None => continue,
                chrono::LocalResult::Single(at) => [Some(at), None],
                chrono::LocalResult::Ambiguous(first, last) => [Some(first), Some(last)],
            };
            let next = candidates
                .into_iter()
                .flatten()
                .filter(|time| time > &now)
                .min();
            if let Some(next) = next {
                return u64::try_from(next.signed_duration_since(&now).num_milliseconds()).ok();
            }
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_morning_time_resolves_to_its_next_occurrence() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 1, 1, 23, 0, 0).unwrap();
        assert_eq!(morning_duration(now, 7 * 60), Some(8 * 60 * 60 * 1000));
        let now = chrono::Utc.with_ymd_and_hms(2026, 1, 2, 6, 45, 0).unwrap();
        assert_eq!(morning_duration(now, 7 * 60), Some(15 * 60 * 1000));
        assert_eq!(morning_duration(now, 24 * 60), None);
    }

    #[test]
    fn a_committed_block_changes_the_daytime_suggestion() {
        assert_eq!(
            ListeningContext::at(11, false).scenario.preset,
            Preset::Focus
        );
        assert_eq!(
            ListeningContext::at(11, true).scenario.preset,
            Preset::Relax
        );
        assert_eq!(
            ListeningContext::at(22, false).scenario.preset,
            Preset::Sleep
        );
    }
}
