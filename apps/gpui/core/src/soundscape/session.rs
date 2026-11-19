use chrono::{Local, NaiveDateTime, Timelike};
use reqwest::Method;
use soundscape_api::{Preset, SoundscapeSession, SoundscapeState, PROXY_ROUTE_SOUNDSCAPE_STATE};

use crate::shell::{Shell, ShellResult};

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
        };
        let body = serde_json::json!({
            "preset": scenario.preset.key(),
            "params": scenario.preset.scape().map(|scape| scape.defaults),
            "energy": scenario.energy,
            "playing": true,
            "session": session,
            "origin": "gpui",
        });
        self.json(Method::POST, PROXY_ROUTE_SOUNDSCAPE_STATE, Some(body), 3)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
