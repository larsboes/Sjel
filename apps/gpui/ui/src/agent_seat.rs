//! Shared Agent Seat state and UI components for Mac (`finder`) and iPhone (`Seat::Agent`).

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, ClickEvent, FontWeight, MouseButton,
    MouseDownEvent, SharedString, Window,
};
use sjel_gpui_core::agent::Event as AgentEvent;
pub use sjel_gpui_core::agent::{tool_summary, AgentRun, Pending, Row, Session, Turn};

use crate::theme::{tint, Palette};

/// Formats one streamed agent event into a human-readable transcript line.
pub fn format_agent_event(event: &AgentEvent) -> String {
    match event.kind.as_str() {
        "turn" => format!("you: {}", event.content),
        "text" => event.content.clone(),
        "tool_call" => format!("{} · {}", event.name, event.arguments),
        "tool_result" => format!("{} · {}", event.name, event.content),
        "needs_approval" => format!("Waiting for approval · {} · {}", event.name, event.preview),
        "approved" => format!("Allowed · {}", event.name),
        "denied" => format!("Denied · {}", event.name),
        "error" => format!("Run error · {}", event.message),
        "done" => "Run finished".to_owned(),
        "stopped" => "Run stopped".to_owned(),
        other => other.to_owned(),
    }
}

/// Renders a session tier selector pill (`sjel`, `project`, `account`).
pub fn tier_pill(
    id_prefix: &str,
    label: &'static str,
    active: bool,
    palette: Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let (bg, text_color, border_color) = if active {
        (
            rgba(tint(palette.accent, 0x24)),
            rgb(palette.accent),
            rgba(tint(palette.accent, 0x50)),
        )
    } else {
        (
            rgba(palette.raised),
            rgb(palette.muted),
            rgba(palette.border),
        )
    };
    div()
        .id(SharedString::from(format!("{id_prefix}-tier-{label}")))
        .flex()
        .items_center()
        .px_2()
        .py(px(3.))
        .rounded(px(5.))
        .cursor_pointer()
        .bg(bg)
        .border_1()
        .border_color(border_color)
        .text_xs()
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .text_color(text_color)
        .on_click(on_click)
        .child(label)
        .into_any_element()
}

/// Renders a session row in the session shelf/list.
pub fn session_row(
    id_prefix: &str,
    session: &Session,
    selected: bool,
    palette: Palette,
    on_select: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let id = &session.id;
    let label = session.title.clone().unwrap_or_else(|| session.root_name());
    div()
        .id(SharedString::from(format!("{id_prefix}-session-{id}")))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded(px(7.))
        .cursor_pointer()
        .when(selected, |row| row.bg(rgba(palette.raised)))
        .text_color(rgb(palette.text))
        .child(div().flex_1().truncate().child(label))
        .child(
            div()
                .text_xs()
                .text_color(rgb(if session.active {
                    palette.green
                } else {
                    palette.muted
                }))
                .child(if session.active { "live" } else { "saved" }),
        )
        .on_mouse_down(MouseButton::Left, on_select)
        .into_any_element()
}
