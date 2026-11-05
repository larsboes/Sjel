//! Key bindings, escape sequence mapping, and touch-bar actions for terminal inputs.

use gpui::{actions, KeyBinding, Keystroke};

actions!(
    terminal,
    [
        /// Copy the selection.
        Copy,
        /// Paste into the terminal.
        Paste,
        /// Select the whole scrollback.
        SelectAll,
        /// Start a new terminal: choose the program and the folder.
        NewTerminal,
        /// Show the next terminal tab.
        NextTab,
        /// Show the previous terminal tab.
        PrevTab,
    ]
);

/// The keys the terminal takes from the window while its grid has focus. `Terminal` is deeper than
/// any page context, so these win over a binding of the same keys further out.
pub fn keys() -> [KeyBinding; 8] {
    [
        KeyBinding::new("cmd-c", Copy, Some("Terminal")),
        KeyBinding::new("cmd-v", Paste, Some("Terminal")),
        KeyBinding::new("cmd-a", SelectAll, Some("Terminal")),
        KeyBinding::new("cmd-t", NewTerminal, Some("Terminal")),
        // The keys that step a window's tabs in Safari and Terminal step the terminal's here.
        KeyBinding::new("shift-cmd-]", NextTab, Some("Terminal")),
        KeyBinding::new("shift-cmd-[", PrevTab, Some("Terminal")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Terminal")),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Terminal")),
    ]
}

/// What a phone's key row offers, and the bytes each key sends. `Ctrl-C` is here rather than a
/// modifier chip because interrupting is the one control a shell cannot be used without.
pub(crate) const TOUCH_KEYS: [(&str, &[u8]); 11] = [
    ("esc", b"\x1b"),
    ("tab", b"\t"),
    ("\u{2190}", b"\x1b[D"),
    ("\u{2191}", b"\x1b[A"),
    ("\u{2193}", b"\x1b[B"),
    ("\u{2192}", b"\x1b[C"),
    ("|", b"|"),
    ("~", b"~"),
    ("/", b"/"),
    ("\u{23ce}", b"\r"),
    ("ctrl-c", &[0x03]),
];

/// What a paste sends. Line breaks become Return, which is what a typed newline is. A program that
/// turned on bracketed paste gets the text between the markers, minus any end marker inside it, so
/// pasted text cannot end the paste early and run as typed commands.
pub(crate) fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let inner = text.replace("\x1b[201~", "");
        format!("\x1b[200~{inner}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

/// The bytes a keystroke sends to a pty.
///
/// `None` means the view keeps the key: a `⌘`-combination belongs to the window (copy, close), and
/// a terminal that swallowed it would make the app unusable around it.
pub fn key_bytes(keystroke: &Keystroke) -> Option<Vec<u8>> {
    let key = keystroke.key.as_str();
    if keystroke.modifiers.platform {
        return None;
    }
    if keystroke.modifiers.control {
        if let Some(bytes) = control_bytes(key) {
            return Some(bytes);
        }
    }
    if keystroke.modifiers.alt {
        // Option is Meta: ESC then the character, which is how readline binds a word.
        let mut bytes = vec![0x1b];
        bytes.extend_from_slice(&plain_bytes(key, keystroke.key_char.as_deref())?);
        return Some(bytes);
    }
    match key {
        "enter" => Some(b"\r".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "backspace" => Some(vec![0x7f]),
        "escape" => Some(vec![0x1b]),
        "delete" => Some(b"\x1b[3~".to_vec()),
        "up" => Some(b"\x1b[A".to_vec()),
        "down" => Some(b"\x1b[B".to_vec()),
        "right" => Some(b"\x1b[C".to_vec()),
        "left" => Some(b"\x1b[D".to_vec()),
        "home" => Some(b"\x1b[H".to_vec()),
        "end" => Some(b"\x1b[F".to_vec()),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "insert" => Some(b"\x1b[2~".to_vec()),
        "space" => Some(b" ".to_vec()),
        _ => plain_bytes(key, keystroke.key_char.as_deref()),
    }
}

/// The character a key typed, or the key's own name when the layout has no character for it.
fn plain_bytes(key: &str, key_char: Option<&str>) -> Option<Vec<u8>> {
    let typed = key_char
        .filter(|character| !character.is_empty())
        .unwrap_or(key);
    if typed.is_empty() || typed.chars().count() > 1 {
        return None;
    }
    Some(typed.as_bytes().to_vec())
}

/// `Ctrl` plus a key, as the single byte a shell receives for it.
fn control_bytes(key: &str) -> Option<Vec<u8>> {
    if key.len() == 1 {
        let byte = key.as_bytes()[0];
        if byte.is_ascii_lowercase() {
            return Some(vec![byte - b'a' + 1]);
        }
        if byte.is_ascii_uppercase() {
            return Some(vec![byte - b'A' + 1]);
        }
    }
    let byte = match key {
        "@" | "space" => 0x00,
        "[" => 0x1b,
        "\\" => 0x1c,
        "]" => 0x1d,
        "^" => 0x1e,
        "_" => 0x1f,
        "?" => 0x7f,
        _ => return None,
    };
    Some(vec![byte])
}
