//! Cached Mermaid diagram rendering, theme mapping, and `TextView` plugin for docs figures.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::{markdown_ast, MarkdownNode, MarkdownParseContext, MarkdownPlugin};
use gpui_kit::{div, prelude::*, px, rgb, App, IntoElement, Window};

use crate::theme::{Palette, ThemeContext};

const MERMAID_NODE_NAME: &str = "mermaid";

type RenderedDiagram = Result<(String, f32, f32), String>;
type DiagramCache = OnceLock<Mutex<HashMap<String, RenderedDiagram>>>;

/// A [`MarkdownPlugin`] that intercepts fenced `mermaid` code blocks in a [`TextView`]
/// and renders them as cached, palette-themed SVG diagrams via `mermaid-rs-renderer`.
#[derive(Default, Clone, Copy)]
pub struct MermaidPlugin;

impl MermaidPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl MarkdownPlugin for MermaidPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        MERMAID_NODE_NAME
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Code(code) = node else {
            return None;
        };
        if !code
            .lang
            .as_deref()
            .is_some_and(|lang| lang.trim().eq_ignore_ascii_case("mermaid"))
        {
            return None;
        }
        let source = code.value.clone();
        let fallback_md = format!("```mermaid\n{source}\n```");
        let raw_md = cx.node_source(node).unwrap_or(fallback_md.as_str());
        Some(
            MarkdownNode::new(MERMAID_NODE_NAME, source.clone())
                .text(source)
                .markdown(raw_md.to_owned()),
        )
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let source = node.data::<String>().map_or("", String::as_str);
        match render_mermaid(source, palette) {
            Ok((svg, width, height)) => div()
                .my_2()
                .overflow_x_scrollbar()
                .child(
                    gpui_kit::svg()
                        .data(svg.as_bytes())
                        .w(px(width))
                        .h(px(height)),
                )
                .into_any_element(),
            Err(error) => div()
                .p_3()
                .rounded(px(8.))
                .bg(rgb(0x1e1e1e))
                .text_xs()
                .text_color(rgb(palette.amber))
                .child(format!("Could not render Mermaid diagram: {error}"))
                .child(
                    div()
                        .pt_2()
                        .text_color(rgb(0xd4d4d4))
                        .child(source.to_owned()),
                )
                .into_any_element(),
        }
    }
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
    static CACHE: DiagramCache = OnceLock::new();
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
