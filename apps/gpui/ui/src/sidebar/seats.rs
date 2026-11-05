use super::types::Seat;
use crate::theme::SidebarPrefs;

/// Which seat the right edge is on, and whether it is showing at all.
///
/// Plain state rather than a GPUI entity: the frame is a function ([`sidebar_frame`]) so a host can
/// hand in whatever it has — the phone's soundscape, a Mac's docked panel — and this stays the one
/// place that decides which seat that is. Every change is written to `sidebar.json` beside
/// `appearance.json`, so the edge comes back where it was left.
#[derive(Clone, Debug, PartialEq)]
pub struct Sidebar {
    prefs: SidebarPrefs,
}

impl Sidebar {
    pub fn load() -> Self {
        Self {
            prefs: SidebarPrefs::load(),
        }
    }

    /// A sidebar with settings a host decided, for a launch flag or a test.
    pub fn with_prefs(prefs: SidebarPrefs) -> Self {
        Self { prefs }
    }

    pub fn prefs(&self) -> &SidebarPrefs {
        &self.prefs
    }

    pub fn is_open(&self) -> bool {
        self.prefs.open
    }

    pub fn seat(&self) -> Seat {
        Seat::from_key(&self.prefs.seat).unwrap_or_default()
    }

    /// Opens the edge on `seat`, whether it was away or showing something else. This is what a
    /// header's button does: it names the seat it wants, so one tap never has to know what was
    /// showing before.
    pub fn open_on(&mut self, seat: Seat) {
        self.prefs.open = true;
        self.prefs.seat = seat.key().to_owned();
        let _ = self.prefs.save();
    }

    /// The seat chips: stays open, changes what it shows.
    pub fn choose(&mut self, seat: Seat) {
        if self.seat() == seat && self.prefs.open {
            return;
        }
        self.open_on(seat);
    }

    /// The docked width. A drag sets it on every move and saves once, when the drag ends.
    pub fn width(&self) -> f32 {
        self.prefs.width
    }

    pub fn set_width(&mut self, width: f32, save: bool) {
        self.prefs.width = width;
        if save {
            let _ = self.prefs.save();
        }
    }

    /// Puts the edge away and remembers that, keeping the seat for the next time it opens.
    pub fn close(&mut self) {
        if !self.prefs.open {
            return;
        }
        self.prefs.open = false;
        let _ = self.prefs.save();
    }
}
