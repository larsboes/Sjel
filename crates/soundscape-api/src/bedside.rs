//! Settings and timer limits shared by Bedside clients and the conductor.

use serde::{Deserialize, Serialize};

pub const BEDSIDE: &str = "bedside";
pub const BACK_TO_SLEEP: &str = "back-to-sleep";
pub const MINUTE_MS: u64 = 60_000;
pub const WIND_DOWN_MS: u64 = 25 * MINUTE_MS;
pub const BACK_TO_SLEEP_MS: u64 = 20 * MINUTE_MS;
pub const FADE_OUT_MS: u64 = 45 * MINUTE_MS;
pub const ALL_NIGHT_MS: u64 = 12 * 60 * MINUTE_MS;
pub const MORNING_LIMIT_MS: u64 = 26 * 60 * MINUTE_MS;

/// The sound sources available at bedtime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SleepRoom {
    #[default]
    Rain,
    Shore,
    Fan,
}

impl SleepRoom {
    pub const ALL: [Self; 3] = [Self::Rain, Self::Shore, Self::Fan];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Rain => "Rain behind glass",
            Self::Shore => "Far shore",
            Self::Fan => "Warm fan",
        }
    }
}

/// How the session ends after the wind-down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SleepEnding {
    FadeOut,
    #[default]
    AllNight,
    UntilMorning,
}

impl SleepEnding {
    pub const ALL: [Self; 3] = [Self::FadeOut, Self::AllNight, Self::UntilMorning];

    pub const fn label(self) -> &'static str {
        match self {
            Self::FadeOut => "Fade out",
            Self::AllNight => "All night",
            Self::UntilMorning => "Until morning",
        }
    }
}

/// The most recently used bedtime settings. Morning is a local wall-clock time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BedsideOptions {
    pub room: SleepRoom,
    pub ending: SleepEnding,
    pub morning_minute: u16,
    pub dawn: bool,
}

impl Default for BedsideOptions {
    fn default() -> Self {
        Self {
            room: SleepRoom::Rain,
            ending: SleepEnding::AllNight,
            morning_minute: 7 * 60,
            dawn: false,
        }
    }
}

impl BedsideOptions {
    /// Validate Bedside timer lengths while allowing a local morning interval to cross DST.
    pub(crate) fn accepts_duration(self, scenario: &str, duration_ms: u64) -> bool {
        self.morning_minute < 24 * 60
            && match scenario {
                BACK_TO_SLEEP => duration_ms == BACK_TO_SLEEP_MS,
                BEDSIDE => match self.ending {
                    SleepEnding::FadeOut => duration_ms == FADE_OUT_MS,
                    SleepEnding::AllNight => duration_ms == ALL_NIGHT_MS,
                    SleepEnding::UntilMorning => (1..=MORNING_LIMIT_MS).contains(&duration_ms),
                },
                _ => false,
            }
    }
}
