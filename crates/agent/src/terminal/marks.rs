//! What a shell's own marks say about the command it is running.
//!
//! A shell with shell integration marks its prompt (`A`), the command line (`B`), the output
//! (`C`) and the end of the command with its exit code (`D;<code>`) in OSC 133 sequences. The
//! marks travel in the pty's byte stream, so the capability reads a command's outcome from bytes
//! it already holds, and a reader that never sees the screen — the phone — can say how it ended.
//!
//! `alacritty_terminal` 0.26 does not pass these to the grid: its `vte` 0.15 handles titles,
//! colours, hyperlinks and the clipboard, and drops the rest. A client therefore draws no marks,
//! and this module is their only reader.
//!
//! Marks arrive split across reads as often as whole ones, so this is a state machine over bytes
//! rather than a search over chunks. Only the seven-bit `ESC ]` introducer counts: `0x9d` is the
//! eight-bit one, but it is also an ordinary continuation byte in UTF-8, so treating it as one
//! would swallow the middle of a character.

use serde::Serialize;

/// What the marks describe.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Marks {
    /// At least one mark arrived, so this shell reports its commands. False is the honest answer
    /// for a shell that does not, and for one the user configured not to.
    pub integration: bool,
    /// A command is running: its executing mark arrived and its end mark has not.
    pub running: bool,
    /// The exit code of the last command that ended.
    pub last_exit_code: Option<i32>,
    /// When that command ended, in Unix seconds.
    pub last_finished_at: Option<i64>,
}

/// How much of an OSC payload the scanner holds. A mark is a few bytes; the longer sequences that
/// share the channel are titles and clipboard writes, which nothing here reads.
const MAX_PAYLOAD: usize = 1024;

/// The scanner and what it has learned, behind one lock.
#[derive(Debug, Default)]
pub struct Tracker {
    state: State,
    payload: Vec<u8>,
    marks: Marks,
}

/// Which part of an escape sequence the scanner is inside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    /// Ordinary output.
    #[default]
    Text,
    /// An escape character, waiting for what it introduces.
    Escape,
    /// An operating-system command whose payload so far is too short to be skipped.
    Osc,
    /// Inside an OSC, after an escape character: `\` closes it, anything else abandons it.
    OscEscape,
    /// An OSC longer than any mark, being skipped to its end.
    Ignored,
    /// Inside an ignored OSC, after an escape character.
    IgnoredEscape,
}

impl Tracker {
    /// Feed output as it arrives. `at` stamps a command's end, in Unix seconds.
    pub fn feed(&mut self, data: &[u8], at: i64) {
        for &byte in data {
            self.byte(byte, at);
        }
    }

    pub fn marks(&self) -> &Marks {
        &self.marks
    }

    fn byte(&mut self, byte: u8, at: i64) {
        match self.state {
            State::Text => {
                if byte == 0x1b {
                    self.state = State::Escape;
                }
            }
            State::Escape => {
                self.state = match byte {
                    b']' => {
                        self.payload.clear();
                        State::Osc
                    }
                    0x1b => State::Escape,
                    _ => State::Text,
                }
            }
            State::Osc => match byte {
                0x07 => self.end(at),
                0x1b => self.state = State::OscEscape,
                _ if self.payload.len() >= MAX_PAYLOAD => {
                    self.payload.clear();
                    self.state = State::Ignored;
                }
                _ => self.payload.push(byte),
            },
            // A stray escape inside a sequence ends it: the sequence cannot be read from here on,
            // and the byte after it may start another.
            State::OscEscape => match byte {
                b'\\' => self.end(at),
                _ => {
                    self.payload.clear();
                    self.state = State::Text;
                    self.byte(byte, at);
                }
            },
            State::Ignored => {
                if byte == 0x07 {
                    self.state = State::Text;
                } else if byte == 0x1b {
                    self.state = State::IgnoredEscape;
                }
            }
            State::IgnoredEscape => {
                self.state = match byte {
                    b'\\' | 0x07 => State::Text,
                    0x1b => State::IgnoredEscape,
                    _ => State::Ignored,
                }
            }
        }
    }

    /// A complete payload, as `133;D;1`.
    fn end(&mut self, at: i64) {
        let payload = std::mem::take(&mut self.payload);
        self.state = State::Text;
        let mut params = payload.split(|&byte| byte == b';');
        if params.next() != Some(b"133".as_slice()) {
            return;
        }
        let kind = params.next().unwrap_or_default();
        if kind == b"A" {
            // The prompt: nothing runs until a command line follows it.
            self.marks.integration = true;
            self.marks.running = false;
        } else if kind == b"B" || kind == b"C" {
            // The command line is in, and by its output mark the command is running.
            self.marks.integration = true;
            self.marks.running = true;
        } else if kind == b"D" {
            self.marks.integration = true;
            self.marks.running = false;
            self.marks.last_exit_code = params
                .next()
                .and_then(|code| std::str::from_utf8(code).ok())
                .and_then(|code| code.trim().parse().ok());
            self.marks.last_finished_at = Some(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(chunks: &[&[u8]]) -> Marks {
        let mut tracker = Tracker::default();
        for chunk in chunks {
            tracker.feed(chunk, 1_700_000_000);
        }
        tracker.marks().clone()
    }

    #[test]
    fn a_prompt_and_a_command_end_are_read_from_the_stream() {
        let marks = fed(&[b"$ ls\x1b]133;C\x1b\\file\n\x1b]133;D;1\x1b\\"]);
        assert!(marks.integration);
        assert!(!marks.running);
        assert_eq!(marks.last_exit_code, Some(1));
        assert_eq!(marks.last_finished_at, Some(1_700_000_000));
    }

    #[test]
    fn a_command_is_running_between_its_two_marks() {
        let mut tracker = Tracker::default();
        tracker.feed(b"\x1b]133;A\x1b\\$ ", 1);
        assert!(!tracker.marks().running, "a prompt runs nothing");
        tracker.feed(b"\x1b]133;B\x1b\\\x1b]133;C\x1b\\", 2);
        assert!(tracker.marks().running);
        tracker.feed(b"\x1b]133;D;0\x1b\\", 3);
        assert!(!tracker.marks().running);
        assert_eq!(tracker.marks().last_exit_code, Some(0));
    }

    #[test]
    fn a_mark_split_across_reads_is_still_a_mark() {
        // The pty hands over whatever it has, so a mark lands in pieces more often than whole.
        let marks = fed(&[b"out\x1b]1", b"33;D;", b"7\x1b", b"\\next"]);
        assert_eq!(marks.last_exit_code, Some(7));
        assert!(marks.integration);
    }

    #[test]
    fn a_bell_ends_a_mark_as_well_as_a_string_terminator() {
        let marks = fed(&[b"\x1b]133;D;2\x07"]);
        assert_eq!(marks.last_exit_code, Some(2));
    }

    #[test]
    fn a_command_that_ended_without_a_code_still_ended() {
        let marks = fed(&[b"\x1b]133;D\x1b\\"]);
        assert!(!marks.running);
        assert_eq!(marks.last_exit_code, None);
        assert!(marks.last_finished_at.is_some());
    }

    #[test]
    fn output_that_only_looks_like_a_mark_is_not_one() {
        // The sequences without an introducer are text a program printed.
        assert_eq!(fed(&[b"echo 133;D;9\n"]), Marks::default());
        // Another OSC code, and a mark this reader does not know, change nothing either.
        assert_eq!(fed(&[b"\x1b]0;a title\x1b\\"]), Marks::default());
        assert_eq!(fed(&[b"\x1b]133;Z\x1b\\"]), Marks::default());
    }

    #[test]
    fn an_unterminated_sequence_does_not_swallow_the_rest_of_the_stream() {
        // A payload longer than a mark is abandoned, and the stream is read again after it.
        let mut long = b"\x1b]133;note;".to_vec();
        long.extend(std::iter::repeat_n(b'x', MAX_PAYLOAD + 10));
        long.extend_from_slice(b"\x1b\\");
        let marks = fed(&[&long, b"\x1b]133;D;4\x1b\\"]);
        assert_eq!(marks.last_exit_code, Some(4));
    }

    #[test]
    fn a_utf8_continuation_byte_is_not_an_eight_bit_introducer() {
        // U+061D is 0xd8 0x9d; reading 0x9d as an OSC introducer would eat the next bytes.
        let marks = fed(&["\u{61d}".as_bytes(), b"rest\x1b]133;D;5\x1b\\"]);
        assert_eq!(marks.last_exit_code, Some(5));
    }
}
