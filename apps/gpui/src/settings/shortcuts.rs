//! Shortcuts listing: Key bindings inspection, filtering, human-readable labels, and key display.

use gpui_kit::component::kbd::Kbd;
use gpui_kit::{div, prelude::*, AnyElement, App, AsKeystroke, IntoElement, Keystroke};
use sjel_gpui_ui::primitives::note;
use sjel_gpui_ui::theme::Palette;

use super::{group, row};
use crate::workspace::{page_title, GoTo, OpenArea};

/// One row of the shortcuts list: the action's label and the strokes bound to it.
type ShortcutRow = (String, Vec<Keystroke>);

/// One group of them: the action-name prefix that selects it, its heading, and its rows.
type ShortcutGroup = (&'static str, &'static str, Vec<ShortcutRow>);

/// Every binding this app registers, read from the keymap itself so the list cannot drift from
/// `workspace::bind_keys` and `editor::bind_keys`.
pub(super) fn shortcuts(
    palette: Palette,
    cx: &App,
    query: &str,
    selected_group: Option<&str>,
) -> Vec<AnyElement> {
    let shortcut_query = if ["shortcut", "shortcuts", "key binding", "key bindings"]
        .iter()
        .any(|term| query.trim().eq_ignore_ascii_case(term))
    {
        ""
    } else {
        query
    };
    let normalized_query = normalize_shortcut_query(shortcut_query);
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let docs = cx.action_documentation();
    let mut groups: [ShortcutGroup; 13] = [
        ("sjel::", "WORKSPACE", Vec::new()),
        ("terminal::", "TERMINAL", Vec::new()),
        ("context_page::", "TODAY", Vec::new()),
        ("calendar::", "CALENDAR", Vec::new()),
        ("finance_page::", "FINANCE", Vec::new()),
        ("people_page::", "PEOPLE", Vec::new()),
        ("backup::", "BACKUP", Vec::new()),
        ("inventory::", "INVENTORY", Vec::new()),
        ("soundscape::", "SOUNDSCAPE", Vec::new()),
        ("feed::", "FEED", Vec::new()),
        ("mail::", "MAIL", Vec::new()),
        ("inbox::", "INBOX", Vec::new()),
        ("editor::", "NOTE EDITOR", Vec::new()),
    ];
    for binding in keymap.bindings() {
        let name = binding.action().name();
        let Some((_, _, rows)) = groups.iter_mut().find(|(ns, _, _)| name.starts_with(ns)) else {
            continue;
        };
        let Some(stroke) = binding.keystrokes().first() else {
            continue;
        };
        let label = (binding.action().as_any().downcast_ref::<GoTo>())
            .map(|GoTo(page)| format!("Go to {}", page_title(*page)))
            .or_else(|| {
                (binding.action().as_any().downcast_ref::<OpenArea>())
                    .map(|OpenArea(area)| format!("Go to {}", area.label()))
            })
            .or_else(|| {
                docs.get(name)
                    .and_then(|doc| doc.lines().next())
                    .map(|doc| doc.trim_end_matches('.').to_owned())
            })
            .unwrap_or_else(|| humanize(name.rsplit("::").next().unwrap_or(name)));
        let stroke = stroke.as_keystroke().clone();
        match rows.iter_mut().find(|(l, _)| *l == label) {
            Some((_, strokes)) => strokes.push(stroke),
            None => rows.push((label, vec![stroke])),
        }
    }
    let matching: Vec<AnyElement> = groups
        .into_iter()
        .filter(|(_, title, rows)| {
            !rows.is_empty() && selected_group.is_none_or(|selected| selected == *title)
        })
        .filter_map(|(_, title, rows)| {
            let rows = rows
                .into_iter()
                .filter(|(label, strokes)| {
                    shortcut_query.trim().is_empty()
                        || label
                            .to_lowercase()
                            .contains(&shortcut_query.to_lowercase())
                        || strokes.iter().any(|stroke| {
                            normalize_shortcut_query(&stroke.unparse(cx.platform_info().os))
                                .to_lowercase()
                                .contains(&normalized_query)
                        })
                })
                .map(|(label, strokes)| {
                    row(
                        label,
                        None,
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .children(strokes.into_iter().map(Kbd::new)),
                        palette,
                    )
                })
                .collect::<Vec<_>>();
            (!rows.is_empty()).then(|| group(title, rows, palette))
        })
        .collect();
    if matching.is_empty() {
        vec![note("No shortcuts match this search.", palette.muted).into_any_element()]
    } else {
        matching
    }
}

pub(crate) fn normalize_shortcut_query(query: &str) -> String {
    let normalized = query
        .to_lowercase()
        .replace("command", "cmd")
        .replace("super", "cmd")
        .replace("option", "alt")
        .replace("control", "ctrl")
        .replace("delete", "backspace")
        .replace('⌘', "cmd-")
        .replace('⌥', "alt-")
        .replace('⌃', "ctrl-")
        .replace('⇧', "shift-")
        .replace('⌫', "backspace")
        .replace('⎋', "escape")
        .replace("cmd ", "cmd-")
        .replace("alt ", "alt-")
        .replace("ctrl ", "ctrl-")
        .replace("shift ", "shift-");
    let normalized = if normalized.trim() == "esc" {
        "escape".to_owned()
    } else {
        normalized.replace("esc ", "escape ")
    };
    normalized
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

/// `OpenSoundscape` → "Open soundscape".
pub(crate) fn humanize(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
