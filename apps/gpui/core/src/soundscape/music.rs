//! What each preset is made of, and how a scape changes over an hour.
//!
//! This is the browser engine's own tables (`capabilities/soundscape/ui/src/lib/symphony/
//! audioEngine.ts`), moved to Rust so the plugin and the dashboard play the same piece. The
//! material is not invented here: chord voicings, motif, scales, roots, the six drift LFOs and
//! the arrangement arc are all the dashboard's, and a change to one without the other makes two
//! surfaces that disagree about what a preset is.
//!
//! What is *not* ported is the Web Audio graph. Voices, filters and rooms live in `synth.rs` and
//! `fx.rs`, which say where they deliberately differ.

use super::rng::Mulberry32;

/// The six presets the conductor knows, in the order its own list uses.
pub const PRESETS: [&str; 6] = ["edm", "ambient", "lofi", "focus", "relax", "sleep"];

/// One preset's identity. `None` index means "not a preset this build knows".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Edm,
    Ambient,
    Lofi,
    Focus,
    Relax,
    Sleep,
}

impl Preset {
    /// The conductor's string, which is also the state field.
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

    /// The label the dashboard shows above the transport.
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

    /// The fixed tempo of the three written performances; `None` for the scapes, whose tempo
    /// follows `pace`.
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

    /// A preset from the conductor's string. Unknown names are not a preset, and the caller
    /// decides whether that is an error or a fallback; the conductor already rejects them.
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

    /// The gain before the master, so a preset change is not a volume change. Measured with
    /// `engine::quality::mix_report` on 2026-10-23: at one trim Lo-fi sat at 0.044 RMS against
    /// 0.087–0.105 for the other five, about 6.5 dB under them.
    pub fn trim(self) -> f32 {
        match self {
            Self::Lofi => 2.1,
            _ => 1.,
        }
    }

    /// The echo as (delay in beats, feedback, wet). Three sixteenths, 0.32 and 0.5 is the
    /// browser's one setting for all six; Ambient wants a longer and quieter repeat, the scapes
    /// a softer one, so the echo is part of the preset's room rather than a fixed effect.
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

    /// How much air the noise bed adds: the scapes get their wash, scaled so Focus stays dry
    /// and Sleep sits in a room. Lo-fi's vinyl is its own bed; the two written performances
    /// have none.
    pub fn air(self) -> f32 {
        match self {
            Self::Focus => 0.8,
            Self::Relax => 1.,
            Self::Sleep => 1.3,
            _ => 1.,
        }
    }

    /// Where the preset sits on the arrangement: a scape breathes over an hour, a written
    /// performance carries its own arrangement in its step function.
    pub fn scape(self) -> Option<Scale> {
        match self {
            Self::Focus => Some(SCAPE_FOCUS),
            Self::Relax => Some(SCAPE_RELAX),
            Self::Sleep => Some(SCAPE_SLEEP),
            _ => None,
        }
    }

    /// The four colours the visuals are drawn in. Kept here beside the music because they are
    /// the same identity: the dashboard's palette array, same order, same numbers.
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
    /// The degrees above the root that the scale is made of, and also how many there are:
    /// a triad is three degrees up, so a four-note scale already spans an octave.
    pub scale: &'static [i32],
    pub defaults: Params,
}

impl Scale {
    pub const fn tones(&self) -> &'static [i32] {
        self.scale
    }

    /// How many degrees the scale has. A triad is three degrees up, so a four-note scale
    /// already spans an octave.
    pub fn degrees(&self) -> i32 {
        self.scale.len() as i32
    }
}

pub const SCAPE_FOCUS: Scale = Scale {
    bpm_base: 96.,
    root: 48,
    scale: &[0, 2, 4, 7, 9],
    defaults: Params {
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
    defaults: Params {
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
    defaults: Params {
        pace: 0.22,
        density: 0.15,
        brightness: 0.2,
        space: 0.85,
        pulse: 0.15,
        texture: 0.55,
    },
};

/// The six scene parameters, same names and range as the conductor's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub pace: f32,
    pub density: f32,
    pub brightness: f32,
    pub space: f32,
    pub pulse: f32,
    pub texture: f32,
}

impl Default for Params {
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

impl Params {
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

/// How much of each layer is in the mix, 0-1. Not booleans: "pads forward, drums almost out"
/// is a mix, and mute is just the bottom of that range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layers {
    pub drums: f32,
    pub bass: f32,
    pub pads: f32,
    pub melody: f32,
    pub texture: f32,
}

impl Default for Layers {
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

impl Layers {
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

// The fixed performances' material, verbatim from the browser engine.
pub const EDM_CHORDS: [[i32; 3]; 4] = [[48, 51, 55], [44, 48, 51], [46, 50, 53], [43, 47, 50]];
pub const EDM_ROOTS: [i32; 4] = [36, 32, 34, 31];
/// Step within a 32-step phrase to (midi, sixteenths).
pub const EDM_MOTIF: [(u32, i32, u8); 7] = [
    (2, 67, 6),
    (4, 67, 2),
    (6, 67, 2),
    (8, 63, 8),
    (18, 65, 6),
    (20, 65, 2),
    (22, 65, 2),
];

pub const AMB_CHORDS: [[i32; 3]; 4] = [[57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62]];
pub const AMB_ROOTS: [i32; 4] = [45, 41, 36, 43];

pub const LOFI_CHORDS: [[i32; 4]; 4] = [
    [53, 57, 60, 64],
    [52, 55, 59, 62],
    [50, 53, 57, 60],
    [48, 52, 55, 59],
];
pub const LOFI_ROOTS: [i32; 4] = [41, 40, 38, 36];
pub const LOFI_PENTA: [i32; 5] = [72, 74, 76, 79, 81];

/// Midi note to hertz. `440 * 2^((m - 69) / 12)`.
pub fn hz(midi: i32) -> f32 {
    440. * 2f32.powf((midi as f32 - 69.) / 12.)
}

/// What one point on the arrangement arc does to the mix. `energy` 1 is the full arrangement,
/// 0 the emptiest point of the rest. The drum gate is deliberately hard rather than faded -- a
/// layer that thins out is not felt returning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcShape {
    pub energy: f32,
    pub density: f32,
    pub pulse: f32,
    pub brightness_add: f32,
    pub drums: f32,
    pub phase: ArcPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArcPhase {
    Bloom,
    Ebb,
    Rest,
    Swell,
    Settle,
    Drift,
    Still,
}

impl ArcPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bloom => "bloom",
            Self::Ebb => "ebb",
            Self::Rest => "rest",
            Self::Swell => "swell",
            Self::Settle => "settle",
            Self::Drift => "drift",
            Self::Still => "still",
        }
    }
}

fn shape_arc(energy: f32) -> ArcShape {
    ArcShape {
        energy,
        density: 0.4 + 0.75 * energy,
        pulse: 0.25 + 0.75 * energy,
        brightness_add: -0.12 + 0.2 * energy,
        drums: if energy < 0.25 { 0. } else { 1. },
        phase: ArcPhase::Bloom,
    }
}

/// The drift LFOs: rate in hertz and phase in radians. Six parameters, six slow oscillators,
/// so a held scape is never exactly still.
const DRIFT: [(f32, f32); 6] = [
    (0.0035, 0.),
    (0.0061, 2.1),
    (0.0047, 4.2),
    (0.0028, 1.3),
    (0.0072, 3.4),
    (0.0041, 5.5),
];

fn drift_index(name: ParamName) -> usize {
    match name {
        ParamName::Pace => 0,
        ParamName::Density => 1,
        ParamName::Brightness => 2,
        ParamName::Space => 3,
        ParamName::Pulse => 4,
        ParamName::Texture => 5,
    }
}

/// The arrangement in progress: everything the step functions need to know that is not a
/// voice. Owned by the engine, read by the visuals for the phase label.
#[derive(Clone, Debug)]
pub struct Arrangement {
    pub preset: Preset,
    pub params: Params,
    pub layers: Layers,
    pub energy: f32,
    /// Seconds since this preset began, from the audio clock.
    pub elapsed: f32,
    /// Bring `pace` toward the operator's activity level, when adapting is on.
    pub adapt: bool,
    pub activity: f32,
}

impl Arrangement {
    pub fn new() -> Self {
        Self {
            preset: Preset::Edm,
            params: Params::default(),
            layers: Layers::default(),
            energy: 0.7,
            elapsed: 0.,
            adapt: false,
            activity: 0.,
        }
    }

    pub fn is_scape(&self) -> bool {
        self.preset.scape().is_some()
    }

    /// The slow shape over a scape. Without it a scape is structurally identical at minute 3
    /// and minute 47 -- the only thing moving is six drift LFOs at +-0.08.
    pub fn arc(&self) -> ArcShape {
        // Sleep does not come back around. It is supposed to end somewhere.
        if self.preset == Preset::Sleep {
            let e = (1. - self.elapsed / 2700.).max(0.);
            let mut shape = shape_arc(e);
            shape.phase = if e > 0.66 {
                ArcPhase::Settle
            } else if e > 0.33 {
                ArcPhase::Drift
            } else {
                ArcPhase::Still
            };
            return shape;
        }

        let cycle = 1080. - 600. * self.params.pace;
        let pos = (self.elapsed % cycle) / cycle;
        // Starts at the top: pressing play should sound like the whole piece, and the sparse
        // part is something the arc arrives at rather than opens with.
        let e = 0.5 + 0.5 * (2. * std::f32::consts::PI * pos).cos();
        let mut shape = shape_arc(e);
        shape.phase = if pos < 0.25 {
            ArcPhase::Bloom
        } else if pos < 0.5 {
            ArcPhase::Ebb
        } else if pos < 0.7 {
            ArcPhase::Rest
        } else {
            ArcPhase::Swell
        };
        shape
    }

    /// Layer levels as actually played: what the operator set, scaled by the arc.
    pub fn mix(&self) -> Layers {
        if !self.is_scape() {
            return self.layers;
        }
        let arc = self.arc();
        Layers {
            drums: self.layers.drums * arc.drums,
            ..self.layers
        }
    }

    /// A parameter with its drift, the adaptation and the arc applied. The three step
    /// functions read this rather than `params`, which is why a scape keeps moving.
    pub fn eff(&self, name: ParamName) -> f32 {
        let (rate, phase) = DRIFT[drift_index(name)];
        let mut value = self.params.get(name)
            + 0.08 * (2. * std::f32::consts::PI * rate * self.elapsed + phase).sin();
        if self.adapt
            && matches!(
                name,
                ParamName::Density | ParamName::Pace | ParamName::Pulse
            )
        {
            value += (self.activity - 0.3) * 0.18;
        }
        if self.is_scape() {
            // On top of the drift, never instead of it.
            let arc = self.arc();
            match name {
                ParamName::Density => value *= arc.density,
                ParamName::Pulse => value *= arc.pulse,
                ParamName::Brightness => value += arc.brightness_add,
                _ => {}
            }
        }
        value.clamp(0., 1.)
    }

    /// Tempo. A written performance has its own; a scape follows `pace`.
    pub fn bpm(&self) -> f32 {
        if let Some(bpm) = self.preset.bpm() {
            return bpm;
        }
        match self.preset.scape() {
            Some(scape) => scape.bpm_base * (0.75 + 0.5 * self.eff(ParamName::Pace)),
            None => 120.,
        }
    }

    /// Seconds per beat.
    pub fn spb(&self) -> f32 {
        60. / self.bpm()
    }

    /// How large the room should be right now, 0-1. A scape follows its `space`; a fixed
    /// performance gets the room its arrangement was written for.
    pub fn room_target(&self) -> f32 {
        if self.is_scape() {
            return self.eff(ParamName::Space);
        }
        match self.preset {
            Preset::Edm => 0.18,
            Preset::Ambient => 0.85,
            Preset::Lofi => 0.3,
            _ => 0.4,
        }
    }

    /// Reverb send level for the current preset.
    pub fn reverb_send(&self) -> f32 {
        if self.is_scape() {
            return 0.1 + 0.6 * self.eff(ParamName::Space);
        }
        match self.preset {
            Preset::Edm => 0.16,
            Preset::Ambient => 0.55,
            Preset::Lofi => 0.22,
            _ => 0.4,
        }
    }

    /// A scape's triad on `degree`: root, third and fifth of the scale, wrapped by octaves.
    pub fn scape_triad(&self, degree: i32) -> [i32; 3] {
        let Some(scape) = self.preset.scape() else {
            return [60, 64, 67];
        };
        let tones = scape.tones();
        let len = scape.degrees();
        let tone = |i: i32| {
            let step = degree + i;
            scape.root + 12 * step.div_euclid(len) + tones[step.rem_euclid(len) as usize]
        };
        [tone(0), tone(2), tone(4)]
    }

    /// The degree whose triad shares the most pitch classes with the chord that is still
    /// sounding: two scales that share notes hand over on those notes.
    pub fn nearest_degree(&self, from: [i32; 3]) -> i32 {
        let Some(scape) = self.preset.scape() else {
            return 0;
        };
        let mut best = 0;
        let mut best_shared = -1;
        for degree in 0..scape.degrees() {
            let triad = self.scape_triad(degree);
            let shared = triad
                .iter()
                .filter(|m| from.iter().any(|f| f.rem_euclid(12) == m.rem_euclid(12)))
                .count() as i32;
            if shared > best_shared {
                best_shared = shared;
                best = degree;
            }
        }
        best
    }

    /// A fresh draw stream for the material the operator asked for.
    pub fn rng(&self, seed: u32) -> Mulberry32 {
        Mulberry32::new(seed)
    }
}

impl Default for Arrangement {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_key_round_trips() {
        assert_eq!(PRESETS.len(), 6);
        for key in PRESETS {
            let preset = Preset::from_key(key).expect(key);
            assert_eq!(preset.key(), key);
            assert!(preset.index() < 6);
        }
        assert_eq!(Preset::from_key("dnb"), None);
    }

    /// The scapes' defaults are the dashboard's; a drift here would make the plugin open a
    /// preset at the wrong place.
    #[test]
    fn scape_defaults_match_the_browser() {
        let focus = Preset::Focus.scape().expect("focus");
        assert_eq!(focus.root, 48);
        assert_eq!(focus.tones(), &[0, 2, 4, 7, 9]);
        assert!((focus.bpm_base - 96.).abs() < 1e-6);
        assert!((focus.defaults.pace - 0.55).abs() < 1e-6);

        let sleep = Preset::Sleep.scape().expect("sleep");
        assert_eq!(sleep.tones(), &[0, 3, 7, 10]);
        assert!((sleep.defaults.space - 0.85).abs() < 1e-6);
    }

    #[test]
    fn a_triad_climbs_the_scale_and_wraps_by_an_octave() {
        let mut arrangement = Arrangement::new();
        arrangement.preset = Preset::Focus;
        // Focus is a major pentatonic from C3: degree 0 is C E A.
        assert_eq!(arrangement.scape_triad(0), [48, 52, 57]);
        // Degree 4 is A, a third above the octave, so the triad gains an octave.
        assert_eq!(arrangement.scape_triad(4), [57, 62, 67]);
        // Degree 5 wraps to the next octave's C.
        assert_eq!(arrangement.scape_triad(5)[0], 60);
    }

    #[test]
    fn the_arc_starts_full_and_returns_to_full() {
        let mut arrangement = Arrangement::new();
        arrangement.preset = Preset::Focus;
        arrangement.params.pace = 0.5;
        arrangement.elapsed = 0.;
        assert!(arrangement.arc().energy > 0.99);
        let cycle = 1080. - 600. * 0.5;
        arrangement.elapsed = cycle;
        assert!(arrangement.arc().energy > 0.99);
        arrangement.elapsed = cycle * 0.5;
        assert!(arrangement.arc().energy < 0.01);
        // The drum gate is hard, not faded.
        arrangement.elapsed = cycle * 0.5;
        assert_eq!(arrangement.arc().drums, 0.);
    }

    #[test]
    fn sleep_ends_instead_of_cycling() {
        let mut arrangement = Arrangement::new();
        arrangement.preset = Preset::Sleep;
        arrangement.elapsed = 0.;
        assert_eq!(arrangement.arc().phase, ArcPhase::Settle);
        arrangement.elapsed = 1000.;
        assert_eq!(arrangement.arc().phase, ArcPhase::Drift);
        arrangement.elapsed = 2700.;
        assert_eq!(arrangement.arc().phase, ArcPhase::Still);
        assert_eq!(arrangement.arc().energy, 0.);
    }

    #[test]
    fn drift_never_leaves_the_unit_range() {
        let mut arrangement = Arrangement::new();
        arrangement.preset = Preset::Focus;
        for step in 0..2000 {
            arrangement.elapsed = step as f32 * 1.7;
            for name in ParamName::ALL {
                let value = arrangement.eff(name);
                assert!((0.0..=1.0).contains(&value), "{name:?} {value}");
            }
        }
    }

    #[test]
    fn a_scape_changes_tempo_with_pace_and_a_performance_does_not() {
        let mut arrangement = Arrangement::new();
        arrangement.preset = Preset::Focus;
        arrangement.params.pace = 0.;
        let slow = arrangement.bpm();
        arrangement.params.pace = 1.;
        assert!(arrangement.bpm() > slow);

        arrangement.preset = Preset::Edm;
        arrangement.params.pace = 0.;
        assert!((arrangement.bpm() - 126.).abs() < 1e-6);
        arrangement.params.pace = 1.;
        assert!((arrangement.bpm() - 126.).abs() < 1e-6);
    }

    #[test]
    fn a_preset_hands_over_on_shared_notes() {
        let mut arrangement = Arrangement::new();
        arrangement.preset = Preset::Relax;
        // C major (C E G) shares C and G with the five-degree relax scale from A.
        let degree = arrangement.nearest_degree([48, 52, 55]);
        let triad = arrangement.scape_triad(degree);
        let shared = triad
            .iter()
            .filter(|m: &&i32| {
                [48i32, 52, 55]
                    .iter()
                    .any(|f| f.rem_euclid(12) == m.rem_euclid(12))
            })
            .count();
        assert!(shared >= 2, "{triad:?}");
    }
}
