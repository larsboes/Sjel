//! The capture tag pill the Mac's triage page draws.

use gpui::AnyElement;

use crate::primitives::tone_chip;
use crate::Palette;

pub fn capture_tag_pill(tag: &str, palette: Palette) -> AnyElement {
    let raw = tag.trim_start_matches('#').trim_end_matches('?');
    let tag_color = match raw {
        "todo" | "task" | "buy" | "price" => palette.accent,
        "idea" | "look" => palette.amber,
        "place" | "go" | "learn" => palette.green,
        _ => palette.muted,
    };
    let label = if tag.starts_with('#') {
        tag.to_owned()
    } else {
        format!("#{tag}")
    };
    tone_chip(label, tag_color)
}
