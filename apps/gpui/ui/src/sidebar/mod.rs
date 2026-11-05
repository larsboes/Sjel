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

pub mod types;
pub mod seats;
pub mod frame;

pub use types::Seat;
pub use seats::Sidebar;
pub use frame::{sidebar_frame, sidebar_scrim, OVERLAY_WIDTH};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::SidebarPrefs;

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
