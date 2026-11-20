use soundscape_api::bedside::{SleepEnding, SleepRoom, BEDSIDE, MINUTE_MS, WIND_DOWN_MS};
use soundscape_api::SoundscapeSession;

/// Bedside mix targets at a particular point in a timer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BedsideFrame {
    pub room: SleepRoom,
    pub music: f32,
    pub floor: f32,
    pub dawn: f32,
}

impl BedsideFrame {
    /// Derive the mix from the conductor's elapsed playback and remaining time.
    pub fn at(session: &SoundscapeSession, now_ms: u64) -> Option<Self> {
        let options = session.bedside?;
        let elapsed = session.elapsed_at(now_ms);
        let remaining = session.remaining_at(now_ms);
        let fade_duration = 3 * MINUTE_MS;
        let floor = smooth(remaining as f32 / fade_duration as f32);
        let wind_down = WIND_DOWN_MS.min(session.duration_ms.saturating_sub(fade_duration));
        let music = if session.scenario == BEDSIDE && wind_down > 0 {
            smooth(1. - elapsed as f32 / wind_down as f32) * floor
        } else {
            0.
        };
        let dawn = if session.scenario == BEDSIDE
            && options.ending == SleepEnding::UntilMorning
            && options.dawn
        {
            smooth(1. - remaining as f32 / (12 * MINUTE_MS) as f32)
        } else {
            0.
        };
        Some(Self {
            room: options.room,
            music,
            floor,
            dawn,
        })
    }
}

fn smooth(value: f32) -> f32 {
    let x = value.clamp(0., 1.);
    x * x * (3. - 2. * x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soundscape_api::bedside::{
        BedsideOptions, ALL_NIGHT_MS, BACK_TO_SLEEP, BACK_TO_SLEEP_MS, FADE_OUT_MS,
    };

    #[test]
    fn the_room_holds_after_wind_down_and_a_short_restart_skips_music_and_dawn() {
        let mut session = SoundscapeSession {
            scenario: BEDSIDE.into(),
            duration_ms: ALL_NIGHT_MS,
            elapsed_ms: 0,
            running_since_ms: Some(0),
            bedside: Some(BedsideOptions {
                ending: SleepEnding::UntilMorning,
                dawn: true,
                ..BedsideOptions::default()
            }),
            deadline_ms: None,
        };
        assert_eq!(BedsideFrame::at(&session, 0).unwrap().music, 1.);
        let settled = BedsideFrame::at(&session, WIND_DOWN_MS).unwrap();
        assert_eq!((settled.music, settled.floor, settled.dawn), (0., 1., 0.));
        let morning = BedsideFrame::at(&session, ALL_NIGHT_MS - 6 * MINUTE_MS).unwrap();
        assert_eq!((morning.music, morning.floor, morning.dawn), (0., 1., 0.5));

        session.duration_ms = FADE_OUT_MS;
        assert!(BedsideFrame::at(&session, 23 * MINUTE_MS).unwrap().music > 0.);
        assert_eq!(BedsideFrame::at(&session, WIND_DOWN_MS).unwrap().music, 0.);

        session.scenario = BACK_TO_SLEEP.into();
        session.duration_ms = BACK_TO_SLEEP_MS;
        let restart = BedsideFrame::at(&session, 0).unwrap();
        assert_eq!((restart.music, restart.floor, restart.dawn), (0., 1., 0.));
        assert_eq!(
            BedsideFrame::at(&session, BACK_TO_SLEEP_MS).unwrap().floor,
            0.
        );
    }
}
