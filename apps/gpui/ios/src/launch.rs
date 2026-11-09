//! Launch flags: the state `ios/run-sim` asks for, so a screen no hand opened can be
//! photographed. A phone sets none of them.

use super::*;

/// `SJEL_REGISTER=places` opens the Places side of the People tab, and `SJEL_PERSON=Ada` opens the
/// card of the first person whose name contains that text. Both exist so a screenshot can reach a
/// page a hand did not navigate to, like `SJEL_DAY` and `SJEL_TAB`; a phone never sets either.
pub(crate) fn launch_register() -> sjel_gpui_ui::nsh_people::Register {
    match std::env::var("SJEL_REGISTER").unwrap_or_default().as_str() {
        "places" => sjel_gpui_ui::nsh_people::Register::Places,
        _ => sjel_gpui_ui::nsh_people::Register::People,
    }
}

pub(crate) fn launch_person() -> Option<String> {
    std::env::var("SJEL_PERSON")
        .ok()
        .filter(|name| !name.is_empty())
}

pub(crate) fn label() -> &'static str {
    if cfg!(target_abi = "sim") {
        "Sjel iOS Simulator"
    } else {
        "Sjel iPhone"
    }
}

/// `SJEL_DAY=2026-10-21` opens the calendar on that day, for a screenshot a hand did not navigate
/// to. A date it cannot read is today.
pub(crate) fn launch_day() -> chrono::NaiveDate {
    frozen_day().unwrap_or_else(|| chrono::Local::now().date_naive())
}

/// `SJEL_TAB=calendar` opens that tab at launch. Today is the default, because it is the page the
/// app opens on.
///
/// A screenshot of a page that is not the first one needs no hand on the screen for this: the
/// touch checks (`host/UITests`) and `ios/run-sim` use it, and a phone never sets it.
pub(crate) fn launch_tab() -> Tab {
    match std::env::var("SJEL_TAB").unwrap_or_default().as_str() {
        "projects" => Tab::Projects,
        "calendar" => Tab::Calendar,
        "people" => Tab::People,
        "interior" => Tab::Interior,
        "inbox" => Tab::Inbox,
        "travel" | "map" => Tab::Travel,
        "settings" => Tab::Settings,
        "mail" => Tab::Mail,
        "documents" => Tab::Documents,
        _ => Tab::Today,
    }
}

/// `SJEL_SIDEBAR=soundscape|activity` opens the right edge at launch on that seat. A phone never sets
/// it; `ios/run-sim` does, because a panel no hand opened still has to be photographable.
///
/// `SJEL_SOUNDSCAPE` keeps its old meaning — `1` opens the edge on the sound seat, `0` closes it —
/// so a script written before the edge had seats still does what it says.
pub(crate) fn launch_edge() -> Option<Seat> {
    if let Ok(seat) = std::env::var("SJEL_SIDEBAR") {
        if seat == "0" || seat.is_empty() {
            return None;
        }
        if let Some(seat) = Seat::from_key(&seat) {
            return Some(seat);
        }
    }
    match std::env::var("SJEL_SOUNDSCAPE").ok().as_deref() {
        Some("0") | Some("") => None,
        Some(_) => Some(Seat::Soundscape),
        None => None,
    }
}

/// `SJEL_SIDEBAR=off` (or `SJEL_SOUNDSCAPE=0`) closes the edge for this launch. A run that takes a
/// picture of a page needs the page, not whatever was left open on it last time.
pub(crate) fn launch_edge_off() -> bool {
    let flag = std::env::var("SJEL_SIDEBAR").unwrap_or_default();
    let sound = std::env::var("SJEL_SOUNDSCAPE").unwrap_or_default();
    matches!(flag.as_str(), "off" | "0") || matches!(sound.as_str(), "off" | "0")
}

/// The day a launch flag fixed, for a page that must not move under a screenshot. A phone never
/// sets `SJEL_DAY`, so the live landing page reads the clock on every rebuild.
pub(crate) fn frozen_day() -> Option<chrono::NaiveDate> {
    std::env::var("SJEL_DAY")
        .ok()
        .and_then(|day| sjel_gpui_core::calendar::parse_day(&day))
}
