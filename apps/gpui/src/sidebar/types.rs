/// The column the sidebar occupies, and the gap the glass panel leaves to the window edge.
pub const WIDTH: f32 = 208.;
pub const GLASS_INSET: f32 = 8.;

pub fn uptime(seconds: u64) -> String {
    match seconds {
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d {}h", s / 86_400, s % 86_400 / 3600),
    }
}
