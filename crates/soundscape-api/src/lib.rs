//! Pure wire models, presets, scene parameters, and route constants for Sjel soundscape.
//!
//! Shared across the conductor daemon (`crates/soundscape`) and the GPUI client
//! (`apps/gpui/core`). Zero platform dependencies (pure `serde`).

use std::time::Duration;
use serde::{Deserialize, Deserializer, Serialize};

/// The six presets the conductor knows, in canonical order.
pub const PRESETS: [&str; 6] = ["edm", "ambient", "lofi", "focus", "relax", "sleep"];

/// The five timed session scenario names.
pub const SCENARIOS: [&str; 5] = ["deep-work", "reading", "reset", "wind-down", "timer"];

/// Maximum duration for a timed focus session (8 hours).
pub const MAX_SESSION_MS: u64 = 8 * 60 * 60 * 1000;

/// Host lease time-to-live in seconds.
pub const HOST_TTL_SECS: u64 = 15;

/// Host lease time-to-live as Duration.
pub const HOST_TTL: Duration = Duration::from_secs(HOST_TTL_SECS);

pub const ROUTE_SOUNDSCAPE_HEALTH: &str = "/api/soundscape/health";
pub const ROUTE_SOUNDSCAPE_STATE: &str = "/api/soundscape/state";
pub const ROUTE_SOUNDSCAPE_STREAM: &str = "/api/soundscape/stream";
pub const ROUTE_SOUNDSCAPE_HOST_CLAIM: &str = "/api/soundscape/host/claim";
pub const ROUTE_SOUNDSCAPE_HOST_RELEASE: &str = "/api/soundscape/host/release";
pub const PROXY_ROUTE_SOUNDSCAPE_STATE: &str = "/soundscape/api/soundscape/state";

/// One preset's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    Edm,
    Ambient,
    Lofi,
    Focus,
    Relax,
    Sleep,
}

impl Preset {
    pub const ALL: [Self; 6] = [
        Self::Edm,
        Self::Ambient,
        Self::Lofi,
        Self::Focus,
        Self::Relax,
        Self::Sleep,
    ];

    /// The conductor's string, which is also the wire state field.
    pub fn key(self) -> &'static str {
        match self {
            Self::Edm => "edm",
            Self::Ambient => "ambient",
            Self::Lofi => "lofi",
            Self::Focus => "focus",
            Self::Relax => "relax",
            Self::Sleep => "sleep",
        }
    }

    /// The label the UI shows above the transport.
    pub fn label(self) -> &'static str {
        match self {
            Self::Edm => "CLASSICAL × EDM · 126 BPM",
            Self::Ambient => "EPIC AMBIENT · 72 BPM",
            Self::Lofi => "LO-FI · 78 BPM",
            Self::Focus => "FOCUS",
            Self::Relax => "RELAX",
            Self::Sleep => "SLEEP",
        }
    }

    /// The fixed tempo of the three written performances; `None` for scapes whose tempo follows pace.
    pub fn bpm(self) -> Option<f32> {
        match self {
            Self::Edm => Some(126.),
            Self::Ambient => Some(72.),
            Self::Lofi => Some(78.),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        PRESETS
            .iter()
            .position(|key| *key == self.key())
            .unwrap_or(0)
    }

    /// A preset from the conductor's string.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "edm" => Some(Self::Edm),
            "ambient" => Some(Self::Ambient),
            "lofi" => Some(Self::Lofi),
            "focus" => Some(Self::Focus),
            "relax" => Some(Self::Relax),
            "sleep" => Some(Self::Sleep),
            _ => None,
        }
    }

    /// The gain before the master, so a preset change is not a volume change.
    pub fn trim(self) -> f32 {
        match self {
            Self::Lofi => 2.1,
            _ => 1.,
        }
    }

    /// The echo as (delay in beats, feedback, wet).
    pub fn echo(self) -> (f32, f32, f32) {
        match self {
            Self::Edm => (0.75, 0.32, 0.5),
            Self::Ambient => (1.5, 0.42, 0.34),
            Self::Lofi => (0.75, 0.24, 0.4),
            Self::Focus => (0.75, 0.28, 0.4),
            Self::Relax => (1., 0.34, 0.4),
            Self::Sleep => (1.5, 0.38, 0.32),
        }
    }

    /// How much air the noise bed adds.
    pub fn air(self) -> f32 {
        match self {
            Self::Focus => 0.8,
            Self::Relax => 1.,
            Self::Sleep => 1.3,
            _ => 1.,
        }
    }

    /// Where the preset sits on the arrangement.
    pub fn scape(self) -> Option<Scale> {
        match self {
            Self::Focus => Some(SCAPE_FOCUS),
            Self::Relax => Some(SCAPE_RELAX),
            Self::Sleep => Some(SCAPE_SLEEP),
            _ => None,
        }
    }

    /// The four colours the visuals are drawn in.
    pub fn palette(self) -> [[f32; 3]; 4] {
        match self {
            Self::Edm => [
                [0.55, 0.18, 1.0],
                [0.95, 0.2, 0.85],
                [0.3, 0.08, 0.95],
                [1.0, 0.25, 0.55],
            ],
            Self::Ambient => [
                [0.08, 0.55, 0.95],
                [0.15, 0.9, 0.75],
                [0.95, 0.65, 0.25],
                [0.35, 0.45, 1.0],
            ],
            Self::Lofi => [
                [1.0, 0.55, 0.25],
                [0.95, 0.35, 0.4],
                [0.75, 0.45, 0.85],
                [1.0, 0.78, 0.35],
            ],
            Self::Focus => [
                [0.1, 0.5, 0.95],
                [0.2, 0.85, 0.9],
                [0.35, 0.4, 1.0],
                [0.15, 0.9, 0.65],
            ],
            Self::Relax => [
                [1.0, 0.6, 0.3],
                [0.9, 0.42, 0.5],
                [0.3, 0.7, 0.7],
                [1.0, 0.8, 0.45],
            ],
            Self::Sleep => [
                [0.28, 0.22, 0.75],
                [0.42, 0.26, 0.85],
                [0.16, 0.16, 0.55],
                [0.5, 0.32, 0.95],
            ],
        }
    }
}

/// A scape's scale, root and default parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub bpm_base: f32,
    pub root: i32,
    pub scale: &'static [i32],
    pub defaults: SoundscapeParams,
}

impl Scale {
    pub const fn tones(&self) -> &'static [i32] {
        self.scale
    }

    pub fn degrees(&self) -> i32 {
        self.scale.len() as i32
    }
}

pub const SCAPE_FOCUS: Scale = Scale {
    bpm_base: 96.,
    root: 48,
    scale: &[0, 2, 4, 7, 9],
    defaults: SoundscapeParams {
        pace: 0.55,
        density: 0.5,
        brightness: 0.6,
        space: 0.35,
        pulse: 0.55,
        texture: 0.35,
    },
};

pub const SCAPE_RELAX: Scale = Scale {
    bpm_base: 66.,
    root: 45,
    scale: &[0, 3, 5, 7, 10],
    defaults: SoundscapeParams {
        pace: 0.4,
        density: 0.35,
        brightness: 0.42,
        space: 0.6,
        pulse: 0.3,
        texture: 0.45,
    },
};

pub const SCAPE_SLEEP: Scale = Scale {
    bpm_base: 52.,
    root: 38,
    scale: &[0, 3, 7, 10],
    defaults: SoundscapeParams {
        pace: 0.22,
        density: 0.15,
        brightness: 0.2,
        space: 0.85,
        pulse: 0.15,
        texture: 0.55,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamName {
    Pace,
    Density,
    Brightness,
    Space,
    Pulse,
    Texture,
}

impl ParamName {
    pub const ALL: [Self; 6] = [
        Self::Pace,
        Self::Density,
        Self::Brightness,
        Self::Space,
        Self::Pulse,
        Self::Texture,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pace => "pace",
            Self::Density => "density",
            Self::Brightness => "brightness",
            Self::Space => "space",
            Self::Pulse => "pulse",
            Self::Texture => "texture",
        }
    }
}

/// The six scene parameters, same names and range as the browser engine's.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundscapeParams {
    pub pace: f32,
    pub density: f32,
    pub brightness: f32,
    pub space: f32,
    pub pulse: f32,
    pub texture: f32,
}

pub type Params = SoundscapeParams;

impl Default for SoundscapeParams {
    fn default() -> Self {
        Self {
            pace: 0.5,
            density: 0.5,
            brightness: 0.5,
            space: 0.5,
            pulse: 0.4,
            texture: 0.4,
        }
    }
}

impl SoundscapeParams {
    pub fn clamped(self) -> Self {
        Self {
            pace: self.pace.clamp(0.0, 1.0),
            density: self.density.clamp(0.0, 1.0),
            brightness: self.brightness.clamp(0.0, 1.0),
            space: self.space.clamp(0.0, 1.0),
            pulse: self.pulse.clamp(0.0, 1.0),
            texture: self.texture.clamp(0.0, 1.0),
        }
    }

    pub fn get(&self, name: ParamName) -> f32 {
        match name {
            ParamName::Pace => self.pace,
            ParamName::Density => self.density,
            ParamName::Brightness => self.brightness,
            ParamName::Space => self.space,
            ParamName::Pulse => self.pulse,
            ParamName::Texture => self.texture,
        }
    }

    pub fn set(&mut self, name: ParamName, value: f32) {
        let value = value.clamp(0., 1.);
        match name {
            ParamName::Pace => self.pace = value,
            ParamName::Density => self.density = value,
            ParamName::Brightness => self.brightness = value,
            ParamName::Space => self.space = value,
            ParamName::Pulse => self.pulse = value,
            ParamName::Texture => self.texture = value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerName {
    Drums,
    Bass,
    Pads,
    Melody,
    Texture,
}

impl LayerName {
    pub const ALL: [Self; 5] = [
        Self::Drums,
        Self::Bass,
        Self::Pads,
        Self::Melody,
        Self::Texture,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Drums => "drums",
            Self::Bass => "bass",
            Self::Pads => "pads",
            Self::Melody => "melody",
            Self::Texture => "texture",
        }
    }
}

/// How much of each layer is in the mix, 0-1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundscapeLayers {
    pub drums: f32,
    pub bass: f32,
    pub pads: f32,
    pub melody: f32,
    pub texture: f32,
}

pub type Layers = SoundscapeLayers;

impl Default for SoundscapeLayers {
    fn default() -> Self {
        Self {
            drums: 1.,
            bass: 1.,
            pads: 1.,
            melody: 1.,
            texture: 1.,
        }
    }
}

impl SoundscapeLayers {
    pub fn clamped(self) -> Self {
        Self {
            drums: self.drums.clamp(0.0, 1.0),
            bass: self.bass.clamp(0.0, 1.0),
            pads: self.pads.clamp(0.0, 1.0),
            melody: self.melody.clamp(0.0, 1.0),
            texture: self.texture.clamp(0.0, 1.0),
        }
    }

    pub fn get(&self, layer: LayerName) -> f32 {
        match layer {
            LayerName::Drums => self.drums,
            LayerName::Bass => self.bass,
            LayerName::Pads => self.pads,
            LayerName::Melody => self.melody,
            LayerName::Texture => self.texture,
        }
    }

    pub fn set(&mut self, layer: LayerName, value: f32) {
        let value = value.clamp(0., 1.);
        match layer {
            LayerName::Drums => self.drums = value,
            LayerName::Bass => self.bass = value,
            LayerName::Pads => self.pads = value,
            LayerName::Melody => self.melody = value,
            LayerName::Texture => self.texture = value,
        }
    }
}

/// A timed session: a scenario, how long it runs, and how much of it has run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundscapeSession {
    pub scenario: String,
    pub duration_ms: u64,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub running_since_ms: Option<u64>,
}

pub type Session = SoundscapeSession;

impl SoundscapeSession {
    pub fn remaining_at(&self, now: u64) -> u64 {
        let running = self
            .running_since_ms
            .map(|started| now.saturating_sub(started))
            .unwrap_or(0);
        self.duration_ms
            .saturating_sub(self.elapsed_ms.saturating_add(running))
    }

    pub fn is_valid(&self, now: u64) -> bool {
        SCENARIOS.contains(&self.scenario.as_str())
            && self.duration_ms > 0
            && self.duration_ms <= MAX_SESSION_MS
            && self.elapsed_ms <= self.duration_ms
            && self
                .running_since_ms
                .is_none_or(|started| started <= now.saturating_add(60_000))
    }

    pub fn pause_at(&mut self, now: u64) {
        if let Some(started) = self.running_since_ms.take() {
            self.elapsed_ms = self
                .duration_ms
                .min(self.elapsed_ms.saturating_add(now.saturating_sub(started)));
        }
    }
}

pub fn default_volume() -> f32 {
    0.65
}

pub fn default_energy() -> f32 {
    0.7
}

/// The soundscape playback state (`GET /soundscape/api/soundscape/state`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundscapeState {
    pub preset: String,
    #[serde(default)]
    pub playing: bool,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default = "default_energy")]
    pub energy: f32,
    #[serde(default)]
    pub params: SoundscapeParams,
    #[serde(default)]
    pub layers: SoundscapeLayers,
    #[serde(default)]
    pub seed: u32,
    #[serde(default)]
    pub session: Option<SoundscapeSession>,
}

pub type Scape = SoundscapeState;

impl Default for SoundscapeState {
    fn default() -> Self {
        Self {
            preset: "edm".into(),
            playing: false,
            volume: default_volume(),
            energy: default_energy(),
            params: SoundscapeParams::default(),
            layers: SoundscapeLayers::default(),
            seed: 0,
            session: None,
        }
    }
}

impl SoundscapeState {
    pub fn preset(&self) -> Preset {
        Preset::from_key(&self.preset).unwrap_or(Preset::Edm)
    }

    pub fn params(&self) -> SoundscapeParams {
        self.params
    }

    pub fn layers(&self) -> SoundscapeLayers {
        self.layers
    }
}

pub fn deserialize_session_patch<'de, D>(
    deserializer: D,
) -> Result<Option<Option<SoundscapeSession>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<SoundscapeSession>::deserialize(deserializer).map(Some)
}

/// A surface sends what it changed, not the whole state.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundscapePatch {
    pub preset: Option<String>,
    pub params: Option<SoundscapeParams>,
    pub layers: Option<SoundscapeLayers>,
    pub seed: Option<u32>,
    pub playing: Option<bool>,
    pub volume: Option<f32>,
    pub energy: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_session_patch")]
    pub session: Option<Option<SoundscapeSession>>,
    pub origin: Option<String>,
}

pub type Patch = SoundscapePatch;

/// The client currently making sound.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundscapeHost {
    pub id: String,
    pub label: String,
    pub since_ms: u64,
}

pub type Host = SoundscapeHost;

/// What every surface reads: the scape, plus whether anyone is actually sounding it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundscapeStateView {
    #[serde(flatten)]
    pub scape: SoundscapeState,
    #[serde(default)]
    pub host: Option<SoundscapeHost>,
}

pub type StateView = SoundscapeStateView;

/// A state as it goes out on the stream: the view plus who caused it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundscapeChange {
    #[serde(flatten)]
    pub view: SoundscapeStateView,
    #[serde(default)]
    pub origin: Option<String>,
}

pub type Change = SoundscapeChange;

/// A client claiming the audio output, or keeping its claim alive.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundscapeClaim {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub takeover: bool,
}

pub type Claim = SoundscapeClaim;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundscapeRelease {
    pub id: String,
}

pub type Release = SoundscapeRelease;
