//! Cached Mermaid diagram rendering and theme mapping for docs figures.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::theme::Palette;

pub(crate) fn mermaid_source(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let opening = lines.next()?.trim();
    let marker = if opening
        .strip_prefix("```")
        .is_some_and(|language| language.trim() == "mermaid")
    {
        "```"
    } else if opening
        .strip_prefix("~~~")
        .is_some_and(|language| language.trim() == "mermaid")
    {
        "~~~"
    } else {
        return None;
    };
    let source = lines.collect::<Vec<_>>();
    let source = source.strip_suffix(&[marker])?;
    Some(source.join("\n"))
}

pub(crate) fn palette_hex(color: u32) -> String {
    let rgb = if color > 0x00ff_ffff {
        color >> 8
    } else {
        color
    };
    format!("#{rgb:06x}")
}

pub(crate) fn render_mermaid(source: &str, palette: Palette) -> Result<(String, f32, f32), String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Result<(String, f32, f32), String>>>> =
        OnceLock::new();
    let key = format!(
        "{source}\0{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
        palette.surface,
        palette.panel_surface,
        palette.raised,
        palette.border,
        palette.text,
        palette.accent
    );
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(entries) = cache.lock() {
        if let Some(result) = entries.get(&key) {
            return result.clone();
        }
    }
    let result = render_mermaid_uncached(source, palette);
    if let Ok(mut entries) = cache.lock() {
        entries.insert(key, result.clone());
    }
    result
}

fn render_mermaid_uncached(source: &str, palette: Palette) -> Result<(String, f32, f32), String> {
    let mut options = mermaid_rs_renderer::RenderOptions::modern();
    let theme = &mut options.theme;
    theme.background = palette_hex(palette.surface);
    theme.primary_color = palette_hex(palette.raised);
    theme.secondary_color = palette_hex(palette.surface);
    theme.tertiary_color = palette_hex(palette.panel_surface);
    theme.primary_text_color = palette_hex(palette.text);
    theme.text_color = palette_hex(palette.text);
    theme.primary_border_color = palette_hex(palette.border);
    theme.line_color = palette_hex(palette.accent);
    theme.cluster_background = palette_hex(palette.panel_surface);
    theme.cluster_border = palette_hex(palette.border);
    theme.edge_label_background = palette_hex(palette.surface);
    theme.sequence_actor_fill = palette_hex(palette.raised);
    theme.sequence_actor_border = palette_hex(palette.border);
    theme.sequence_actor_line = palette_hex(palette.accent);
    theme.sequence_note_fill = palette_hex(palette.panel_surface);
    theme.sequence_note_border = palette_hex(palette.border);
    theme.sequence_activation_fill = palette_hex(palette.raised);
    theme.sequence_activation_border = palette_hex(palette.border);
    theme.pie_title_text_color = palette_hex(palette.text);
    theme.pie_section_text_color = palette_hex(palette.text);
    theme.pie_legend_text_color = palette_hex(palette.text);
    let dimensions =
        mermaid_rs_renderer::measure(source, options.clone()).map_err(|error| error.to_string())?;
    let svg = mermaid_rs_renderer::render_with_options(source, options)
        .map_err(|error| error.to_string())?;
    Ok((svg, dimensions.width, dimensions.height))
}
