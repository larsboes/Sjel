//! The two one-way channels between the audio thread and everything else.
//!
//! The audio callback may not allocate, take a lock or wait, so nothing here is a `Mutex`: the
//! interface thread writes plain atomics and the callback reads them once per block. Floats
//! travel as their bit patterns because there is no `AtomicF32` in core.
//!
//! Direction matters. [`Control`] is what the app *asks for* -- preset, parameters, layer mix,
//! volume. [`Visual`] is what the synth *reports* -- spectrum, level, event counters. A visual
//! that disagrees with the sound by a frame is invisible; a control path that blocks the audio
//! callback is a dropout, so the two never share a structure.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, Ordering};

use super::music::{LayerName, Layers, ParamName, Params, Preset};

/// Number of spectrum bands the visuals draw.
pub const BANDS: usize = 32;

#[inline]
fn store_f32(target: &AtomicU32, value: f32) {
    target.store(value.clamp(0., 4.).to_bits(), Ordering::Relaxed);
}

/// Signed values (the pan of the last note) need their own range: clamping a float to the
/// unsigned range would silently centre every note the visuals place to the left.
#[inline]
fn store_signed_f32(target: &AtomicU32, value: f32) {
    target.store(value.clamp(-4., 4.).to_bits(), Ordering::Relaxed);
}

#[inline]
fn load_f32(target: &AtomicU32) -> f32 {
    f32::from_bits(target.load(Ordering::Relaxed))
}

/// What the app asks the synth for. Every field has a sane default, so a synth that is never
/// written to still produces silence rather than noise.
pub struct Control {
    playing: AtomicBool,
    preset: AtomicU8,
    volume: AtomicU32,
    energy: AtomicU32,
    seed: AtomicU32,
    adapt: AtomicBool,
    activity: AtomicU32,
    params: [AtomicU32; 6],
    layers: [AtomicU32; 5],
}

impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

impl Control {
    pub fn new() -> Self {
        let params = Params::default();
        let layers = Layers::default();
        Self {
            playing: AtomicBool::new(false),
            preset: AtomicU8::new(Preset::Edm.index() as u8),
            volume: AtomicU32::new(0.65f32.to_bits()),
            energy: AtomicU32::new(0.7f32.to_bits()),
            seed: AtomicU32::new(0),
            adapt: AtomicBool::new(false),
            activity: AtomicU32::new(0.3f32.to_bits()),
            params: std::array::from_fn(|index| {
                AtomicU32::new(params.get(ParamName::ALL[index]).to_bits())
            }),
            layers: std::array::from_fn(|index| {
                AtomicU32::new(layers.get(LayerName::ALL[index]).to_bits())
            }),
        }
    }

    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::Relaxed);
    }

    pub fn playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn set_preset(&self, preset: Preset) {
        self.preset.store(preset.index() as u8, Ordering::Relaxed);
    }

    pub fn preset(&self) -> Preset {
        Preset::from_key(super::music::PRESETS[self.preset.load(Ordering::Relaxed) as usize % 6])
            .unwrap_or(Preset::Edm)
    }

    pub fn set_volume(&self, volume: f32) {
        store_f32(&self.volume, volume);
    }

    pub fn volume(&self) -> f32 {
        load_f32(&self.volume)
    }

    pub fn set_energy(&self, energy: f32) {
        store_f32(&self.energy, energy);
    }

    pub fn energy(&self) -> f32 {
        load_f32(&self.energy)
    }

    pub fn set_seed(&self, seed: u32) {
        self.seed.store(seed, Ordering::Relaxed);
    }

    pub fn seed(&self) -> u32 {
        self.seed.load(Ordering::Relaxed)
    }

    pub fn set_adapt(&self, adapt: bool) {
        self.adapt.store(adapt, Ordering::Relaxed);
    }

    pub fn adapt(&self) -> bool {
        self.adapt.load(Ordering::Relaxed)
    }

    pub fn set_activity(&self, activity: f32) {
        store_f32(&self.activity, activity);
    }

    pub fn activity(&self) -> f32 {
        load_f32(&self.activity)
    }

    pub fn set_param(&self, name: ParamName, value: f32) {
        let index = ParamName::ALL
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or(0);
        store_f32(&self.params[index], value);
    }

    pub fn param(&self, name: ParamName) -> f32 {
        let index = ParamName::ALL
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or(0);
        load_f32(&self.params[index])
    }

    pub fn set_params(&self, params: Params) {
        for name in ParamName::ALL {
            self.set_param(name, params.get(name));
        }
    }

    pub fn params(&self) -> Params {
        let mut params = Params::default();
        for name in ParamName::ALL {
            params.set(name, self.param(name));
        }
        params
    }

    pub fn set_layer(&self, name: LayerName, value: f32) {
        let index = LayerName::ALL
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or(0);
        store_f32(&self.layers[index], value);
    }

    pub fn layer(&self, name: LayerName) -> f32 {
        let index = LayerName::ALL
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or(0);
        load_f32(&self.layers[index])
    }

    pub fn set_layers(&self, layers: Layers) {
        for name in LayerName::ALL {
            self.set_layer(name, layers.get(name));
        }
    }

    pub fn layers(&self) -> Layers {
        let mut layers = Layers::default();
        for name in LayerName::ALL {
            layers.set(name, self.layer(name));
        }
        layers
    }
}

/// What the synth reports back, for the visuals.
pub struct Visual {
    bands: [AtomicU32; BANDS],
    rms: AtomicU32,
    peak: AtomicU32,
    /// Bumped on every kick and every note, so a viewer can tell a new event from the last one
    /// without keeping a queue.
    kick_seq: AtomicU32,
    note_seq: AtomicU32,
    /// The most recent note, as midi and stereo position.
    midi: AtomicI32,
    pan: AtomicU32,
    /// Voices sounding, which is a better readout of "how full is this" than any meter.
    voices: AtomicU32,
    /// The preset and the arc phase the synth is actually playing, which can lag what the app
    /// asked for by one bar.
    preset: AtomicU8,
    phase: AtomicU8,
    energy: AtomicU32,
    step: AtomicU32,
}

impl Default for Visual {
    fn default() -> Self {
        Self::new()
    }
}

impl Visual {
    pub fn new() -> Self {
        Self {
            bands: std::array::from_fn(|_| AtomicU32::new(0)),
            rms: AtomicU32::new(0),
            peak: AtomicU32::new(0),
            kick_seq: AtomicU32::new(0),
            note_seq: AtomicU32::new(0),
            midi: AtomicI32::new(60),
            pan: AtomicU32::new(0),
            voices: AtomicU32::new(0),
            preset: AtomicU8::new(0),
            phase: AtomicU8::new(0),
            energy: AtomicU32::new(0.7f32.to_bits()),
            step: AtomicU32::new(0),
        }
    }

    pub fn bands(&self) -> [f32; BANDS] {
        std::array::from_fn(|index| load_f32(&self.bands[index]))
    }

    pub fn rms(&self) -> f32 {
        load_f32(&self.rms)
    }

    pub fn peak(&self) -> f32 {
        load_f32(&self.peak)
    }

    pub fn kicks(&self) -> u32 {
        self.kick_seq.load(Ordering::Relaxed)
    }

    pub fn notes(&self) -> u32 {
        self.note_seq.load(Ordering::Relaxed)
    }

    pub fn midi(&self) -> i32 {
        self.midi.load(Ordering::Relaxed)
    }

    pub fn pan(&self) -> f32 {
        load_f32(&self.pan)
    }

    pub fn voices(&self) -> u32 {
        self.voices.load(Ordering::Relaxed)
    }

    pub fn preset(&self) -> Preset {
        Preset::from_key(super::music::PRESETS[self.preset.load(Ordering::Relaxed) as usize % 6])
            .unwrap_or(Preset::Edm)
    }

    pub fn phase(&self) -> super::music::ArcPhase {
        use super::music::ArcPhase;
        match self.phase.load(Ordering::Relaxed) {
            0 => ArcPhase::Bloom,
            1 => ArcPhase::Ebb,
            2 => ArcPhase::Rest,
            3 => ArcPhase::Swell,
            4 => ArcPhase::Settle,
            5 => ArcPhase::Drift,
            _ => ArcPhase::Still,
        }
    }

    pub fn energy(&self) -> f32 {
        load_f32(&self.energy)
    }

    pub fn step(&self) -> u32 {
        self.step.load(Ordering::Relaxed)
    }

    // ---- written by the audio thread, and by a test that needs a spectrum to draw ---------------

    /// Publish one update. Called from the audio callback about sixty times a second, and from a
    /// test that needs a spectrum without a device.
    pub fn write_spectrum(&self, bands: [f32; BANDS], rms: f32, peak: f32) {
        for (target, value) in self.bands.iter().zip(bands) {
            store_f32(target, value);
        }
        store_f32(&self.rms, rms);
        store_f32(&self.peak, peak);
    }

    pub fn write_events(&self, kicks: u32, notes: u32, midi: i32, pan: f32) {
        if kicks > 0 {
            self.kick_seq.fetch_add(kicks, Ordering::Relaxed);
        }
        if notes > 0 {
            self.note_seq.fetch_add(notes, Ordering::Relaxed);
            self.midi.store(midi, Ordering::Relaxed);
            store_signed_f32(&self.pan, pan);
        }
    }

    pub fn write_state(
        &self,
        preset: Preset,
        phase: super::music::ArcPhase,
        energy: f32,
        voices: u32,
        step: u32,
    ) {
        self.preset.store(preset.index() as u8, Ordering::Relaxed);
        self.phase.store(
            match phase {
                super::music::ArcPhase::Bloom => 0,
                super::music::ArcPhase::Ebb => 1,
                super::music::ArcPhase::Rest => 2,
                super::music::ArcPhase::Swell => 3,
                super::music::ArcPhase::Settle => 4,
                super::music::ArcPhase::Drift => 5,
                super::music::ArcPhase::Still => 6,
            },
            Ordering::Relaxed,
        );
        store_f32(&self.energy, energy);
        self.voices.store(voices, Ordering::Relaxed);
        self.step.store(step, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_round_trips_every_parameter() {
        let control = Control::new();
        let mut params = Params::default();
        params.set(ParamName::Brightness, 0.83);
        control.set_params(params);
        assert!((control.param(ParamName::Brightness) - 0.83).abs() < 1e-6);
        assert!((control.params().pace - params.pace).abs() < 1e-6);

        let layers = Layers {
            drums: 0.,
            bass: 0.5,
            pads: 1.,
            melody: 0.25,
            texture: 0.75,
        };
        control.set_layers(layers);
        assert!((control.layer(LayerName::Texture) - 0.75).abs() < 1e-6);
        assert_eq!(control.layers().drums, 0.);
    }

    #[test]
    fn control_clamps_what_it_stores() {
        let control = Control::new();
        control.set_volume(4.5);
        assert!(control.volume() <= 4.);
        control.set_volume(-2.);
        assert_eq!(control.volume(), 0.);
    }

    #[test]
    fn the_preset_round_trips_through_the_atomic() {
        let control = Control::new();
        for preset in [
            Preset::Edm,
            Preset::Ambient,
            Preset::Lofi,
            Preset::Focus,
            Preset::Relax,
            Preset::Sleep,
        ] {
            control.set_preset(preset);
            assert_eq!(control.preset(), preset);
        }
    }

    #[test]
    fn visual_reports_what_the_synth_wrote() {
        let visual = Visual::new();
        let mut bands = [0f32; BANDS];
        bands[3] = 0.5;
        visual.write_spectrum(bands, 0.2, 0.4);
        assert!((visual.bands()[3] - 0.5).abs() < 1e-6);
        assert!((visual.rms() - 0.2).abs() < 1e-6);

        visual.write_events(2, 3, 64, -0.4);
        assert_eq!(visual.kicks(), 2);
        assert_eq!(visual.notes(), 3);
        assert_eq!(visual.midi(), 64);
        assert!((visual.pan() + 0.4).abs() < 1e-6);

        visual.write_state(
            Preset::Lofi,
            super::super::music::ArcPhase::Drift,
            0.5,
            7,
            12,
        );
        assert_eq!(visual.preset(), Preset::Lofi);
        assert_eq!(visual.phase(), super::super::music::ArcPhase::Drift);
        assert_eq!(visual.voices(), 7);
        assert_eq!(visual.step(), 12);
    }
}
