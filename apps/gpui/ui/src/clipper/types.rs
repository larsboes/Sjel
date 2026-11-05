//! Clipper types, constants, and rich clipboard integration.
use std::sync::OnceLock;

pub type RichClipboardReader = fn() -> Option<(String, bool)>;

static RICH_CLIPBOARD: OnceLock<RichClipboardReader> = OnceLock::new();

/// Register the host platform's rich clipboard reader for HTML-to-markdown paste.
pub fn set_rich_clipboard_reader(reader: RichClipboardReader) {
    let _ = RICH_CLIPBOARD.set(reader);
}

pub(super) fn read_rich_clipboard() -> Option<(String, bool)> {
    RICH_CLIPBOARD.get().and_then(|reader| reader())
}

pub(super) const CLIP_PAGE_BLOCKS: usize = 80;
