//! Rich clipboard access on macOS for HTML paste with markdown conversion.

#[cfg(target_os = "macos")]
pub fn read_rich_clipboard() -> Option<(String, bool)> {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::ns_string;
    use sjel_gpui_core::clipper::html_to_markdown;

    let pboard = NSPasteboard::generalPasteboard();
    if let Some(html) = pboard.stringForType(ns_string!("public.html")) {
        let html_str = html.to_string();
        if !html_str.trim().is_empty() {
            let md = html_to_markdown(&html_str);
            if !md.trim().is_empty() {
                return Some((md, true));
            }
        }
    }
    if let Some(plain) = pboard.stringForType(ns_string!("public.utf8-plain-text")) {
        let text = plain.to_string();
        if !text.trim().is_empty() {
            return Some((text, false));
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
pub fn read_rich_clipboard() -> Option<(String, bool)> {
    None
}
