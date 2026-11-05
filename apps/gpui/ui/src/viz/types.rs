//! Dynamic animation particles, transient kick rings, and time-based smoothing.

/// One point of light travelling along a ridge.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Comet {
    pub(crate) active: bool,
    /// Seconds since it started.
    pub(crate) age: f32,
    /// Which ridge it rides.
    pub(crate) stratum: usize,
    /// Where it started, 0 (left) to 1 (right), and how far it travels.
    pub(crate) from: f32,
    pub(crate) span: f32,
    pub(crate) strength: f32,
}

/// A ring from a kick: the one thing in the picture that is an event rather than a level.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ring {
    pub(crate) active: bool,
    pub(crate) age: f32,
    pub(crate) strength: f32,
}

/// A value moving towards a target with a time constant, in seconds.
///
/// The whole reason the motion is smooth at any frame rate: `tau` is a duration, not a step.
pub(crate) fn approach(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    if tau <= 0. {
        return target;
    }
    current + (target - current) * (1. - (-dt / tau).exp())
}
