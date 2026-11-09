/// What the edge is showing.
///
/// The key is what `sidebar.json` stores, so renaming a variant does not lose a person's place: an
/// unknown key falls back to [`Seat::Soundscape`], which is the seat the edge was built for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Seat {
    /// The soundscape drawer's content: transport, presets, volume, mixer.
    #[default]
    Soundscape,
    /// What this phone is doing: the Mac's answer, the queue, the reminders it holds.
    Activity,
    /// A run the owner can watch while moving between pages.
    Agent,
    /// A shell on the Mac: another agent's terminal, driven from this window.
    Terminal,
}

impl Seat {
    pub const ALL: [Seat; 4] = [
        Seat::Soundscape,
        Seat::Activity,
        Seat::Agent,
        Seat::Terminal,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::Soundscape => "soundscape",
            Self::Activity => "activity",
            Self::Agent => "agent",
            Self::Terminal => "terminal",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|seat| seat.key() == key)
    }

    /// The chip's word, in the case a chip takes.
    pub fn label(self) -> &'static str {
        match self {
            Self::Soundscape => "Sound",
            Self::Activity => "Activity",
            Self::Agent => "Agent",
            Self::Terminal => "Terminal",
        }
    }

    /// The panel's own title, which is the seat and not the app: "SOUNDSCAPE", "ACTIVITY".
    pub fn title(self) -> &'static str {
        match self {
            Self::Soundscape => "SOUNDSCAPE",
            Self::Activity => "ACTIVITY",
            Self::Agent => "AGENT",
            Self::Terminal => "TERMINAL",
        }
    }
}
