//! The capture tag pill the Mac's triage page draws.

use gpui::{AnyElement, IntoElement};

use crate::ely::data_display::{Badge, Tone};
use crate::Palette;

pub fn capture_tag_pill(tag: &str, _palette: Palette) -> AnyElement {
    let raw = tag.trim_start_matches('#').trim_end_matches('?');
    let tone = match raw {
        "todo" | "task" | "buy" | "price" => Tone::Accent,
        "idea" | "look" => Tone::Warning,
        "place" | "go" | "learn" => Tone::Success,
        _ => Tone::Neutral,
    };
    let label = if tag.starts_with('#') {
        tag.to_owned()
    } else {
        format!("#{tag}")
    };
    Badge::new(label).tone(tone).into_any_element()
}
