//! The right edge: one panel, more than one seat.
//!
//! `apps/gpui/PORTING_PLAN.md` §3 item 2 asks for a docked panel beside the page — the seat of the
//! later adaptive UI, where the operator watches Sjel work instead of only receiving the result —
//! and notes that the soundscape drawer already slides in from that same edge. Two panels cannot own
//! one edge, so the edge is one object: a frame that draws the title, the seat chips, the close
//! button and the width, and a *seat* that is the content a host hands in.
//!
//! What lives here is the part both clients answer the same way — which seat is showing, what it is
//! called, and that the choice survives the next launch. On the phone the frame is an overlay, because
//! a phone has no width to dock beside a page; the same frame docks on a Mac, which is why the
//! decision belongs here rather than in either host.
//!
//! The shared seats are soundscape and activity; the desktop adds the agent run as a third seat.

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, FontWeight, MouseButton, SharedString, Window,
};

use crate::theme::SidebarPrefs;
use crate::Palette;

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

/// The frame: title, seat chips, close, and the body a host hands in.
///
/// `on_seat` and `on_close` are called from the tap handlers; the host decides what those mean (the
/// phone pauses the sound when the edge goes away). Two seats always get their chips, because a seat
/// nobody can see is one nobody switches to.
pub fn sidebar_frame(
    palette: Palette,
    seat: Seat,
    body: AnyElement,
    on_seat: impl Fn(Seat, &mut Window, &mut App) + 'static,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let mut chips = div().flex().items_center().gap_1();
    // One callback for every chip: an `Rc` because each chip's handler owns its own closure, and the
    // frame is drawn on one thread.
    let on_seat = std::rc::Rc::new(on_seat);
    for option in Seat::ALL {
        let is_active = option == seat;
        let on_seat = on_seat.clone();
        chips = chips.child(
            div()
                .id(SharedString::from(format!("sidebar-seat-{}", option.key())))
                .px_3()
                .py(px(6.))
                .rounded_full()
                .text_xs()
                .when(is_active, |chip| {
                    chip.bg(rgb(palette.accent))
                        .text_color(rgb(0xffffff))
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .when(!is_active, |chip| {
                    chip.bg(rgba(palette.raised)).text_color(rgb(palette.muted))
                })
                .child(option.label())
                .on_mouse_down(MouseButton::Left, move |_, window, app| {
                    on_seat(option, window, app)
                }),
        );
    }
    div()
        .id("sidebar")
        .size_full()
        .flex()
        .flex_col()
        // The terminal seat paints its own background at the opacity its setting asks for, so
        // the frame under it stays clear: two opaque layers would hide what the setting lets
        // show through.
        .when(seat != Seat::Terminal, |frame| {
            frame.bg(rgba(palette.panel()))
        })
        .text_color(rgb(palette.text))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .p_4()
                .pb(px(0.))
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child(seat.title()),
                )
                .child(chips)
                .child(
                    div()
                        .id("sidebar-close")
                        .flex_none()
                        .size(px(36.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgba(palette.raised))
                        .text_color(rgb(palette.muted))
                        .child("✕")
                        .on_mouse_down(MouseButton::Left, move |_, window, app| {
                            on_close(window, app)
                        }),
                ),
        )
        .child(div().flex_1().min_h_0().child(body))
        .into_any_element()
}

/// The width of the overlay edge on a phone: an edge panel, not a full-screen sheet, so the page it
/// slid over stays visible enough to know where you are.
const OVERLAY_WIDTH: f32 = 340.;

/// The scrim behind an overlay edge, which is what the phone draws: the panel stands at the right
/// edge at full height, and a tap anywhere on the dimmed page beside it puts it away. A docked panel
/// has no scrim, so this stays a separate piece a host may skip.
pub fn sidebar_scrim(
    on_tap: impl Fn(&mut Window, &mut App) + 'static,
    body: AnyElement,
) -> AnyElement {
    div()
        .id("sidebar-overlay")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .bg(rgba(0x00000059))
        // A tap that closes the edge must not reach the page underneath it. The close button sits
        // exactly where Today's gear does, so without this a tap on ✕ also opened Settings: the frame
        // had already taken the overlay away when the finger came up, and the up landed on the page.
        .occlude()
        .flex()
        .flex_row()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .on_mouse_down(MouseButton::Left, move |_, window, app| on_tap(window, app)),
        )
        .child(
            div()
                .flex_none()
                .w(px(OVERLAY_WIDTH))
                .h_full()
                .flex()
                .flex_col()
                .child(body),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A seat is a key, not a label: a file written by this build reads back as the same seat, and a
    /// key this build does not know is the seat the edge was built for rather than a blank panel.
    #[test]
    fn a_seat_is_a_key_and_an_unknown_one_falls_back() {
        for seat in Seat::ALL {
            assert_eq!(Seat::from_key(seat.key()), Some(seat));
            assert_ne!(seat.label(), seat.title(), "a chip is not a heading");
        }
        assert_eq!(Seat::from_key("settings"), None);
        let stale = Sidebar::with_prefs(SidebarPrefs {
            open: true,
            seat: "roomplan".to_owned(),
            ..SidebarPrefs::default()
        });
        assert_eq!(stale.seat(), Seat::Soundscape);
        assert!(stale.is_open(), "an unknown seat is not a closed edge");
    }

    /// Opening names the seat it wants, the chips keep the edge open, and closing keeps the seat for
    /// the next open — the three moves a person makes on this edge.
    #[test]
    fn opening_names_a_seat_and_closing_keeps_it() {
        let mut sidebar = Sidebar::with_prefs(SidebarPrefs::default());
        assert!(!sidebar.is_open(), "a phone opens nothing by itself");

        sidebar.open_on(Seat::Activity);
        assert!(sidebar.is_open());
        assert_eq!(sidebar.seat(), Seat::Activity);

        sidebar.choose(Seat::Soundscape);
        assert!(
            sidebar.is_open(),
            "choosing a seat does not put the edge away"
        );
        assert_eq!(sidebar.seat(), Seat::Soundscape);

        sidebar.close();
        assert!(!sidebar.is_open());
        assert_eq!(
            sidebar.seat(),
            Seat::Soundscape,
            "closing is not forgetting"
        );
    }
}
