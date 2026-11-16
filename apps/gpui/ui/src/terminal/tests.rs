use crate::terminal_screen::{ansi_256, spans, tone_of, Screen, ScreenLine};
use alacritty_terminal::{index::Side, selection::SelectionType};
const WHEEL_LINES: i32 = 3;

fn host() -> Host {
    Host::Shell(std::sync::Arc::new(
        shell::Shell::new("http://127.0.0.1:1").unwrap(),
    ))
}

fn peer() -> Peer {
    serde_json::from_value(serde_json::json!({"id":"test", "alive":false})).unwrap()
}

#[gpui::test]
fn ely_grid_keeps_keyboard_focus_selection_and_copy(cx: &mut gpui::TestAppContext) {
    use gpui::Focusable;
    let (view, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
        TerminalView::new(peer(), host(), 0, 20, 3, Palette::dark(), cx)
    });
    view.update(cx, |view, cx| {
        view.apply(
            Update::Output {
                offset: 0,
                data: b"copy me".to_vec(),
            },
            cx,
        );
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        cx.bind_keys(keys());
        view.read(cx).focus_handle(cx).focus(window, cx);
    });
    cx.simulate_keystrokes("cmd-a cmd-c");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert!(cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap()
            .starts_with("copy me"))
    });
    view.read_with(cx, |view, cx| {
        assert!(view.terminal.read(cx).selected_text().is_some())
    });
}

#[gpui::test]
fn remote_grid_obeys_offsets_and_restarts_after_truncation(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
        TerminalView::new(peer(), host(), 0, 20, 3, Palette::dark(), cx)
    });
    view.update(cx, |view, cx| {
        assert!(view.apply(
            Update::Output {
                offset: 0,
                data: b"\x1b[3".to_vec()
            },
            cx
        ));
        assert!(view.apply(
            Update::Output {
                offset: 4,
                data: b"1mx".to_vec()
            },
            cx
        ));
        assert!(!view.apply(
            Update::Output {
                offset: 0,
                data: b"\x1b[31mx".to_vec()
            },
            cx
        ));
        assert_eq!(view.position(), 7);
    });
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert_eq!(view.terminal.read(cx).text(), "x")
    });
    view.update(cx, |view, cx| {
        view.terminal
            .update(cx, |terminal, cx| terminal.select_all(cx));
        assert!(view.apply(Update::Truncated { first_offset: 42 }, cx));
        assert!(view.terminal.read(cx).selected_text().is_none());
        assert!(view.apply(
            Update::Output {
                offset: 42,
                data: b"fresh".to_vec()
            },
            cx
        ));
        assert_eq!(view.position(), 47);
    });
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert_eq!(view.terminal.read(cx).text(), "fresh");
        assert_eq!(view.dimensions, view.terminal.read(cx).dimensions());
    });
}

#[gpui::test]
fn exit_disables_the_remote_transport_without_discarding_the_grid(cx: &mut gpui::TestAppContext) {
    let (view, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
        TerminalView::new(peer(), host(), 0, 20, 3, Palette::dark(), cx)
    });
    view.update(cx, |view, cx| {
        assert!(view.apply(
            Update::Output {
                offset: 0,
                data: b"done".to_vec()
            },
            cx
        ));
        assert!(view.apply(Update::Command(Default::default()), cx));
        assert!(view.apply(
            Update::Exit {
                code: 7,
                signal: None
            },
            cx
        ));
        assert!(!view.peer.alive);
        assert_eq!(view.peer.exit_code, Some(7));
        view.send(b"ignored".to_vec(), cx);
        assert!(view.outgoing.is_empty());
        view.terminal.update(cx, |terminal, cx| {
            assert!(terminal.feed_remote(b"late", cx).is_err())
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, cx| {
        assert_eq!(view.terminal.read(cx).text(), "done")
    });
}
use super::keys::TOUCH_KEYS;
use super::*;
use crate::theme::MONOKAI;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use gpui::{Keystroke, Modifiers};

fn typed(key: &str, key_char: Option<&str>) -> Option<Vec<u8>> {
    key_bytes(&Keystroke {
        key: key.to_owned(),
        key_char: key_char.map(str::to_owned),
        modifiers: Modifiers::default(),
    })
}

fn with(modifiers: Modifiers, key: &str, key_char: Option<&str>) -> Option<Vec<u8>> {
    key_bytes(&Keystroke {
        key: key.to_owned(),
        key_char: key_char.map(str::to_owned),
        modifiers,
    })
}

fn text_of(line: &ScreenLine) -> String {
    line.runs.iter().map(|run| run.text.as_str()).collect()
}

#[test]
fn a_letter_is_the_letter_it_printed() {
    assert_eq!(typed("a", Some("a")), Some(b"a".to_vec()));
    assert_eq!(typed("space", Some(" ")), Some(b" ".to_vec()));
    // A key with no character of its own — a dead key — sends nothing rather than a guess.
    assert_eq!(typed("dead_acute", None), None);
}

#[test]
fn the_editing_keys_are_the_bytes_a_pty_expects() {
    assert_eq!(typed("enter", None), Some(b"\r".to_vec()));
    assert_eq!(typed("tab", None), Some(b"\t".to_vec()));
    assert_eq!(typed("backspace", None), Some(vec![0x7f]));
    assert_eq!(typed("escape", None), Some(vec![0x1b]));
    assert_eq!(typed("up", None), Some(b"\x1b[A".to_vec()));
    assert_eq!(typed("down", None), Some(b"\x1b[B".to_vec()));
    assert_eq!(typed("delete", None), Some(b"\x1b[3~".to_vec()));
    assert_eq!(typed("home", None), Some(b"\x1b[H".to_vec()));
    assert_eq!(typed("pageup", None), Some(b"\x1b[5~".to_vec()));
}

#[test]
fn control_and_option_are_what_a_shell_reads() {
    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    assert_eq!(with(control, "c", None), Some(vec![0x03]));
    assert_eq!(with(control, "d", None), Some(vec![0x04]));
    assert_eq!(with(control, "l", Some("l")), Some(vec![0x0c]));
    assert_eq!(with(control, "[", None), Some(vec![0x1b]));

    let alt = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    // Option-b is "back one word" to readline: ESC then the letter.
    assert_eq!(with(alt, "b", Some("b")), Some(b"\x1bb".to_vec()));
}

#[test]
fn a_window_shortcut_stays_with_the_window() {
    let platform = Modifiers {
        platform: true,
        ..Modifiers::default()
    };
    // ⌘C copies; a terminal that swallowed it would make the app unusable around it.
    assert_eq!(with(platform, "c", Some("c")), None);
}

#[test]
fn a_dragged_selection_copies_the_cells_it_covers() {
    let mut screen = Screen::new(20, 3);
    screen.feed(b"hello world");
    let start = screen.point_at(0, 0);
    let end = screen.point_at(4, 0);
    screen.select(SelectionType::Simple, start, Side::Left);
    screen.extend_selection(end, Side::Right);
    assert_eq!(screen.selected_text().as_deref(), Some("hello"));
    // The selected cells draw on the scheme's selection colour, the rest of the line does not.
    let mut lines = screen.lines();
    let spans = spans(lines.remove(0), &MONOKAI);
    assert_eq!(spans[0].text, "hello");
    assert_eq!(spans[0].bg, Some(MONOKAI.selection));
    assert_eq!(spans[1].bg, None);
}

#[test]
fn a_wide_character_keeps_its_second_cell() {
    // The grid draws every character on one cell's advance, so the wide character's second
    // cell must stay in the text as a space or everything after it shifts left by one.
    let mut screen = Screen::new(20, 3);
    screen.feed("漢a".as_bytes());
    let runs = &screen.lines()[0].runs;
    let text: String = runs.iter().map(|run| run.text.as_str()).collect();
    assert_eq!(text, "漢 a");
}

#[test]
fn a_click_without_a_drag_leaves_nothing_selected() {
    let mut screen = Screen::new(20, 3);
    screen.feed(b"hello");
    let at = screen.point_at(2, 0);
    screen.select(SelectionType::Simple, at, Side::Left);
    screen.drop_empty_selection();
    assert_eq!(screen.selected_text(), None);
}

#[test]
fn scrolled_back_history_draws_on_its_own_rows() {
    let mut screen = Screen::new(10, 3);
    screen.feed(b"1\r\n2\r\n3\r\n4\r\n5");
    screen.scroll(2);
    let lines = screen.lines();
    let shown: Vec<String> = lines.iter().map(text_of).collect();
    assert_eq!(shown, ["1", "2", "3"]);
}

#[gpui::test]
fn a_paste_types_returns_and_cannot_end_a_bracketed_paste_early(cx: &mut gpui::TestAppContext) {
    use crate::ely::terminal::RemoteTransport;
    use gpui::AppContext;
    use std::{cell::RefCell, rc::Rc};
    let bytes = Rc::new(RefCell::new(Vec::new()));
    let typed = bytes.clone();
    let terminal = cx.new(|cx| {
        Terminal::remote(
            20,
            3,
            RemoteTransport::new(
                move |input| {
                    typed.borrow_mut().extend_from_slice(input);
                    Ok(())
                },
                |_, _| Ok(()),
            ),
            cx,
        )
    });
    terminal.update(cx, |terminal, cx| terminal.paste_text("ls\nexit\r\n", cx));
    assert_eq!(*bytes.borrow(), b"ls\rexit\r");
    bytes.borrow_mut().clear();
    terminal.update(cx, |terminal, cx| {
        terminal.feed_remote(b"\x1b[?2004h", cx).unwrap();
        terminal.paste_text("a\x1b[201~rm -rf", cx);
    });
    assert_eq!(*bytes.borrow(), b"\x1b[200~a[201~rm -rf\x1b[201~");
}

#[test]
fn the_touch_key_row_sends_what_a_keyboard_would() {
    let bytes = |label: &str| {
        TOUCH_KEYS
            .iter()
            .find(|(name, _)| *name == label)
            .map(|(_, bytes)| *bytes)
            .unwrap_or(&[][..])
    };
    assert_eq!(bytes("esc"), &[0x1b]);
    assert_eq!(bytes("tab"), &[0x09]);
    assert_eq!(bytes("ctrl-c"), &[0x03]);
    assert_eq!(bytes("\u{2191}"), &b"\x1b[A"[..]);
    // The row offers the three characters a phone keyboard hides.
    for character in ["|", "~", "/"] {
        assert_eq!(bytes(character), character.as_bytes());
    }
}

#[test]
fn the_ansi_palette_is_the_tones_a_program_asks_for() {
    // The first sixteen are the scheme's: Monokai's red is its pink.
    assert_eq!(ansi_256(1, &MONOKAI), 0xf92672);
    assert_eq!(
        tone_of(Color::Named(NamedColor::Red), &MONOKAI),
        Some(0xf92672)
    );
    assert_eq!(
        tone_of(Color::Named(NamedColor::Foreground), &MONOKAI),
        Some(MONOKAI.foreground)
    );
    assert_eq!(
        tone_of(Color::Named(NamedColor::Background), &MONOKAI),
        None
    );
    // The cube: 196 is full red, 16 is black, 231 white.
    assert_eq!(ansi_256(196, &MONOKAI), 0xff0000);
    assert_eq!(ansi_256(16, &MONOKAI), 0x000000);
    assert_eq!(ansi_256(231, &MONOKAI), 0xffffff);
    // The greys run from 8 to 238.
    assert_eq!(ansi_256(232, &MONOKAI), 0x080808);
    assert_eq!(ansi_256(255, &MONOKAI), 0xeeeeee);
}

#[test]
fn the_screen_keeps_what_the_program_printed() {
    let mut screen = Screen::new(20, 4);
    screen.feed(b"hello");
    let lines = screen.lines();
    assert_eq!(lines.len(), 4);
    assert_eq!(text_of(&lines[0]), "hello");
}

#[test]
fn an_escape_sequence_split_across_frames_still_lands() {
    let mut screen = Screen::new(20, 3);
    // A red x, split mid-sequence: the parser is kept between frames.
    screen.feed(b"\x1b[3");
    screen.feed(b"1mx");
    let lines = screen.lines();
    let red = lines[0]
        .runs
        .iter()
        .find(|run| run.text.contains('x'))
        .expect("the x was drawn");
    assert_eq!(
        red.style.fg,
        Color::Named(NamedColor::Red),
        "the colour was lost when the sequence was split"
    );
}

#[test]
fn a_resize_reaches_the_parser_and_keeps_its_content() {
    let mut screen = Screen::new(40, 10);
    screen.feed(b"one\r\ntwo");
    screen.resize(20, 5);
    assert_eq!((screen.cols(), screen.rows()), (20, 5));
    assert_eq!(text_of(&screen.lines()[0]), "one");
}

#[test]
fn the_cursor_is_on_the_line_the_program_stopped_at() {
    let mut screen = Screen::new(20, 5);
    screen.feed(b"first\r\nsecond");
    let lines = screen.lines();
    assert_eq!(lines[1].cursor, Some(6));
    assert_eq!(lines[0].cursor, None);
}

#[test]
fn the_wheel_moves_through_history_and_stops_at_the_live_screen() {
    let mut screen = Screen::new(20, 3);
    for index in 0..10 {
        screen.feed(format!("line{index}\r\n").as_bytes());
    }
    assert_eq!(screen.scrolled(), 0);
    screen.scroll(WHEEL_LINES);
    assert_eq!(screen.scrolled(), WHEEL_LINES as usize);
    // The live screen is where a reader cannot scroll past.
    screen.scroll(10_000);
    assert_eq!(screen.scrolled(), screen.term.grid().history_size());
}

#[test]
fn the_cursor_cell_is_inverted_and_the_rest_of_the_line_survives() {
    let mut screen = Screen::new(20, 3);
    screen.feed(b"abc");
    let mut lines = screen.lines();
    let mut line = lines.remove(0);
    line.cursor = Some(1);
    let spans = spans(line, &MONOKAI);
    // "a", the inverted "b", then "c": the cursor splits the run rather than replacing it.
    let texts: Vec<&str> = spans.iter().map(|span| span.text.as_str()).collect();
    assert_eq!(texts, ["a", "b", "c"]);
    assert_eq!(spans[1].bg, Some(MONOKAI.cursor));
    assert_eq!(spans[1].fg, MONOKAI.background);
    // A cursor past the text draws a cell of its own at its column.
    let mut line = screen.lines().remove(0);
    line.cursor = Some(5);
    let spans = crate::terminal_screen::spans(line, &MONOKAI);
    assert_eq!(spans.last().map(|span| span.column), Some(5));
}
