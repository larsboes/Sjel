//! The browser engine's random stream, ported bit for bit.
//!
//! `mulberry32` is the whole contract: the conductor stores a seed, and two surfaces that
//! draw from that seed in the same order must hear the same material. Anything with a
//! different algorithm here would make the plugin and the dashboard disagree about a
//! session while both claim to be playing it.

/// Mutable 32-bit generator. Not `Clone` by accident: two clones would replay the same
/// notes twice, which is the one thing a shared stream must not do.
pub struct Mulberry32 {
    state: u32,
}

impl Mulberry32 {
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// The next draw in `0.0..1.0`.
    pub fn next_f32(&mut self) -> f32 {
        self.state = self.state.wrapping_add(0x6d2b_79f5);
        let mut t = self.state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        ((t ^ (t >> 14)) as f64 / 4_294_967_296.0) as f32
    }

    /// A draw an octave of intent coarser: which of `n` equal outcomes to take.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        ((self.next_f32() * n as f32) as usize).min(n - 1)
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }

    /// A draw in `-1.0..1.0`, for detune and stereo jitter.
    pub fn signed(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

/// Base36, so a seed is short enough to read off the screen and type back in.
pub fn format_seed(seed: u32) -> String {
    let mut digits = Vec::new();
    let mut n = seed;
    loop {
        let digit = n % 36;
        digits.push(std::char::from_digit(digit, 36).unwrap_or('0'));
        n /= 36;
        if n == 0 {
            break;
        }
    }
    digits.iter().rev().collect()
}

/// Inverse of [`format_seed`]; `None` on anything unparseable.
pub fn parse_seed(text: &str) -> Option<u32> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    u32::from_str_radix(trimmed, 36).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same seed has to produce the same stream in the dashboard, so the first four
    /// draws are pinned against the browser's output for seed 0.
    #[test]
    fn matches_the_browser_stream() {
        let mut rng = Mulberry32::new(0);
        let draws: Vec<f32> = (0..4).map(|_| rng.next_f32()).collect();
        assert!((draws[0] - 0.266_429_2).abs() < 1e-7, "{draws:?}");
        assert!((draws[1] - 0.000_329_745_7).abs() < 1e-7, "{draws:?}");
        assert!((draws[2] - 0.223_272_03).abs() < 1e-7, "{draws:?}");
        assert!((draws[3] - 0.146_202_15).abs() < 1e-7, "{draws:?}");
    }

    #[test]
    fn draws_stay_in_unit_range() {
        let mut rng = Mulberry32::new(0xdead_beef);
        for _ in 0..10_000 {
            let v = rng.next_f32();
            assert!((0.0..1.0).contains(&v), "{v}");
            let s = rng.signed();
            assert!((-1.0..1.0).contains(&s), "{s}");
            assert!(rng.below(5) < 5);
        }
    }

    #[test]
    fn a_seed_round_trips_through_base36() {
        for seed in [0, 1, 42, 0xffff_ffff, 0x0bad_f00d] {
            assert_eq!(parse_seed(&format_seed(seed)), Some(seed), "{seed}");
        }
        assert_eq!(parse_seed(""), None);
        assert_eq!(parse_seed("  "), None);
        assert_eq!(parse_seed("!!!"), None);
    }
}
