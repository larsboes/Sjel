use gpui::StatefulInteractiveElement;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontStyle, FontWeight,
    HighlightStyle, IntoElement, MouseButton, MouseDownEvent, ScrollHandle, SharedString,
    StrikethroughStyle, StyledText, Subscription, UnderlineStyle, Window,
};
use sjel_gpui_core::blocks::{Block, Doc, Kind};
use sjel_gpui_core::inline::{Inline, Mark};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::theme::Palette;

struct DocSource {
    slug: &'static str,
    category: &'static str,
    file: &'static str,
    markdown: &'static str,
}

enum MarkdownBlock {
    Content(Block),
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

fn title(doc: &DocSource) -> &str {
    doc.markdown
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .unwrap_or(doc.slug)
}

fn summary(doc: &DocSource) -> &str {
    doc.markdown
        .lines()
        .find(|line| {
            let line = line.trim();
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with('`')
                && !line.starts_with('|')
                && !line.starts_with('>')
                && !line.starts_with('-')
        })
        .unwrap_or("")
        .trim()
}

include!(concat!(env!("OUT_DIR"), "/docs.rs"));

pub struct DocsPage {
    selected: String,
    query: String,
    filter: Option<(Entity<InputState>, Subscription)>,
    scroll: ScrollHandle,
    pending_anchor: Option<String>,
}

impl DocsPage {
    pub fn new() -> Self {
        Self {
            selected: "index".into(),
            query: String::new(),
            filter: None,
            scroll: ScrollHandle::new(),
            pending_anchor: None,
        }
    }

    pub fn open(&mut self, slug: String, cx: &mut Context<Self>) {
        if DOCS.iter().any(|doc| doc.slug == slug) {
            self.selected = slug;
            self.pending_anchor = None;
            cx.notify();
        }
    }

    fn open_anchor(&mut self, slug: String, anchor: Option<String>, cx: &mut Context<Self>) {
        if DOCS.iter().any(|doc| doc.slug == slug) {
            self.selected = slug;
            self.pending_anchor = anchor;
            cx.notify();
        }
    }

    fn filter(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.filter {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search documentation…"));
        let subscription =
            cx.subscribe_in(&input, window, |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = input.read(cx).value().to_string();
                    cx.notify();
                }
            });
        self.filter = Some((input.clone(), subscription));
        input
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.filter(window, cx);
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn source(&self) -> &'static DocSource {
        DOCS.iter()
            .find(|doc| doc.slug == self.selected)
            .unwrap_or(&DOCS[0])
    }

    fn follow_link(&mut self, href: &str, file: &str, cx: &mut Context<Self>) {
        let (path, anchor) = href.split_once('#').map_or((href, None), |(path, anchor)| {
            (path, Some(anchor.to_owned()))
        });
        if path.is_empty() {
            if let Some(anchor) = anchor {
                self.pending_anchor = Some(anchor);
                cx.notify();
            }
            return;
        }
        if let Some(slug) = doc_slug(path, file) {
            if DOCS.iter().any(|doc| doc.slug == slug) {
                self.open_anchor(slug, anchor, cx);
                return;
            }
        }
        if href.starts_with("http://")
            || href.starts_with("https://")
            || href.starts_with("mailto:")
        {
            cx.open_url(href);
        } else if href.starts_with("//") {
            cx.open_url(&format!("https:{href}"));
        } else if let Some(url) = source_url(href, file) {
            cx.open_url(&url);
        }
    }
}

fn doc_slug(href: &str, file: &str) -> Option<String> {
    if href.starts_with('#') || href.starts_with("//") || href.contains("://") {
        return None;
    }
    let path = href.split(['?', '#']).next()?;
    if path.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if !path.starts_with('/') {
        if let Some((parent, _)) = file.rsplit_once('/') {
            parts.extend(parent.split('/'));
        }
    }
    for part in path.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    let joined = parts.join("/");
    let path = joined.strip_prefix("docs/")?;
    let mut slug = path.strip_suffix(".md")?.to_owned();
    if slug == "README" {
        slug = "index".into();
    } else if let Some(parent) = slug.strip_suffix("/index") {
        slug = parent.to_owned();
    }
    Some(slug)
}

fn source_url(href: &str, file: &str) -> Option<String> {
    if href.starts_with("https://") || href.starts_with("http://") || href.starts_with("mailto:") {
        return Some(href.to_owned());
    }
    if href.starts_with('#') || href.starts_with("//") {
        return None;
    }
    let path = href.split(['?', '#']).next()?;
    let mut parts = Vec::new();
    if !path.starts_with('/') {
        if let Some((parent, _)) = file.rsplit_once('/') {
            parts.extend(parent.split('/'));
        }
    }
    for part in path.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    let suffix = href.find(['?', '#']).map_or("", |at| &href[at..]);
    Some(format!(
        "https://github.com/larsboes/Sjel/blob/main/{}{suffix}",
        parts.join("/")
    ))
}

fn heading_id(text: &str, seen: &mut std::collections::HashSet<String>) -> String {
    let text = text.trim_end_matches(|ch: char| ch == '#' || ch.is_whitespace());
    let inline = Inline::parse(text);
    let base = inline
        .text
        .to_lowercase()
        .chars()
        .filter_map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '_' | '-') {
                Some(ch)
            } else if ch.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>();
    let mut id = base.clone();
    let mut suffix = 1;
    while seen.contains(&id) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    seen.insert(id.clone());
    id
}

fn explicit_anchor(text: &str) -> Option<String> {
    let id = text
        .trim()
        .strip_prefix("<a id=\"")?
        .strip_suffix("\"></a>")?;
    (!id.is_empty()
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')))
    .then(|| id.to_owned())
}

fn anchor_index(blocks: &[MarkdownBlock], target: &str) -> Option<usize> {
    let mut seen = std::collections::HashSet::new();
    blocks.iter().enumerate().find_map(|(index, block)| {
        let MarkdownBlock::Content(block) = block else {
            return None;
        };
        let id = match block.kind {
            Kind::Heading(_) => Some(heading_id(&block.text, &mut seen)),
            _ => explicit_anchor(&block.text),
        }?;
        (id == target).then_some(index)
    })
}

fn table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

fn is_table_separator(line: &str) -> bool {
    line.contains('|')
        && line.contains('-')
        && line
            .chars()
            .all(|ch| ch.is_whitespace() || matches!(ch, '|' | ':' | '-'))
}

fn parse_markdown(content: &str) -> Vec<MarkdownBlock> {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut index = 0;
    let mut fence: Option<&str> = None;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            text.push_str(line);
            text.push('\n');
            if trimmed.starts_with(marker) {
                fence = None;
            }
            index += 1;
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            text.push_str(line);
            text.push('\n');
            index += 1;
            continue;
        }
        if index + 1 < lines.len() && line.contains('|') && is_table_separator(lines[index + 1]) {
            if !text.is_empty() {
                blocks.extend(
                    Doc::parse(&text)
                        .blocks
                        .into_iter()
                        .map(MarkdownBlock::Content),
                );
                text.clear();
            }
            let headers = table_cells(line);
            index += 2;
            let mut rows = Vec::new();
            while index < lines.len()
                && lines[index].contains('|')
                && !lines[index].trim().is_empty()
            {
                rows.push(table_cells(lines[index]));
                index += 1;
            }
            blocks.push(MarkdownBlock::Table { headers, rows });
            continue;
        }
        text.push_str(line);
        text.push('\n');
        index += 1;
    }
    if !text.is_empty() {
        blocks.extend(
            Doc::parse(&text)
                .blocks
                .into_iter()
                .map(MarkdownBlock::Content),
        );
    }
    blocks
}

fn inline_highlights(
    marks: &[(std::ops::Range<usize>, Mark)],
    palette: Palette,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let style = |mark| match mark {
        Mark::Bold => HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
        Mark::Italic => HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        },
        Mark::Code => HighlightStyle {
            background_color: Some(rgba(palette.raised).into()),
            ..Default::default()
        },
        Mark::Strike => HighlightStyle {
            strikethrough: Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            }),
            ..Default::default()
        },
        Mark::Link => HighlightStyle {
            color: Some(rgb(palette.accent).into()),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: None,
                wavy: false,
            }),
            ..Default::default()
        },
        Mark::Highlight => HighlightStyle {
            background_color: Some(rgba(palette.amber).into()),
            ..Default::default()
        },
    };
    let mut cuts: Vec<usize> = marks
        .iter()
        .flat_map(|(range, _)| [range.start, range.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|pair| {
            let range = pair[0]..pair[1];
            marks
                .iter()
                .filter(|(mark, _)| mark.start <= range.start && range.end <= mark.end)
                .map(|(_, mark)| style(*mark))
                .reduce(HighlightStyle::highlight)
                .map(|style| (range, style))
        })
        .collect()
}

fn render_inline_text(
    text: &str,
    file: String,
    palette: Palette,
    cx: &mut Context<DocsPage>,
) -> AnyElement {
    let inline = Inline::parse(text);
    let links = inline.links.clone();
    let shown: SharedString = if inline.text.is_empty() {
        " ".into()
    } else {
        inline.text.clone().into()
    };
    let styled = StyledText::new(shown).with_highlights(inline_highlights(&inline.marks, palette));
    let layout = styled.layout().clone();
    let mut element = div().child(styled);
    if !links.is_empty() {
        element = element.cursor_pointer().on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                let shown = layout
                    .index_for_position(event.position)
                    .unwrap_or_else(|nearest| nearest);
                if let Some((_, href)) = links.iter().find(|(range, _)| range.contains(&shown)) {
                    this.follow_link(href, &file, cx);
                    cx.stop_propagation();
                }
            }),
        );
    }
    element.into_any_element()
}

fn mermaid_source(text: &str) -> Option<String> {
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

fn palette_hex(color: u32) -> String {
    let rgb = if color > 0x00ff_ffff {
        color >> 8
    } else {
        color
    };
    format!("#{rgb:06x}")
}

fn render_mermaid(source: &str, palette: Palette) -> Result<(String, f32, f32), String> {
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

fn render_doc_block(
    index: usize,
    block: &MarkdownBlock,
    file: String,
    palette: Palette,
    cx: &mut Context<DocsPage>,
) -> AnyElement {
    let MarkdownBlock::Content(block) = block else {
        let MarkdownBlock::Table { headers, rows } = block else {
            unreachable!()
        };
        let columns = headers.len().max(1);
        let cell = |value: &str, heading: bool, cx: &mut Context<DocsPage>| {
            let mut item = div()
                .w(px(180.))
                .flex_none()
                .px_3()
                .py_2()
                .border_r_1()
                .border_color(rgba(palette.border))
                .child(render_inline_text(value, file.clone(), palette, cx));
            if heading {
                item = item
                    .bg(rgba(palette.raised))
                    .font_weight(FontWeight::SEMIBOLD);
            }
            item.into_any_element()
        };
        let header = div()
            .flex()
            .children(headers.iter().map(|value| cell(value, true, cx)));
        let mut table = div()
            .w(px(columns as f32 * 180.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .overflow_hidden()
            .child(header);
        for (row_index, row) in rows.iter().enumerate() {
            let mut rendered = div()
                .flex()
                .when(row_index % 2 == 1, |row| row.bg(rgba(palette.surface)));
            for column in 0..columns {
                rendered =
                    rendered.child(cell(row.get(column).map_or("", String::as_str), false, cx));
            }
            table = table.child(rendered);
        }
        return div().overflow_x_scrollbar().child(table).into_any_element();
    };
    if explicit_anchor(&block.text).is_some() {
        return div().h(px(1.)).into_any_element();
    }
    let content_text = render_inline_text(&block.text, file, palette, cx);
    let content = match block.kind {
        Kind::Heading(level) => {
            let size = match level {
                1 => px(23.),
                2 => px(20.),
                3 => px(17.),
                _ => px(15.),
            };
            div()
                .pt_2()
                .text_size(size)
                .font_weight(if level <= 2 {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(palette.text))
                .child(content_text)
        }
        Kind::Code => {
            if let Some(source) = mermaid_source(&block.text) {
                return match render_mermaid(&source, palette) {
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
                        .child(div().pt_2().text_color(rgb(0xd4d4d4)).child(source))
                        .into_any_element(),
                };
            }
            return div()
                .p_3()
                .rounded(px(8.))
                .bg(rgb(0x1e1e1e))
                .text_xs()
                .text_color(rgb(0xd4d4d4))
                .child(block.text.clone())
                .into_any_element();
        }
        Kind::Quote => div()
            .px_3()
            .border_l_2()
            .border_color(rgb(palette.accent))
            .text_sm()
            .text_color(rgb(palette.muted))
            .child(content_text),
        Kind::Bullet => div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(div().text_color(rgb(palette.muted)).child("•"))
            .child(div().flex_1().child(content_text)),
        Kind::Numbered => div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .child(block.prefix.trim().to_owned()),
            )
            .child(div().flex_1().child(content_text)),
        Kind::Todo(done) => div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .child(if done { "☑" } else { "☐" }),
            )
            .child(div().flex_1().child(content_text)),
        _ => div()
            .text_sm()
            .text_color(rgb(palette.text))
            .child(content_text),
    };
    if matches!(block.kind, Kind::Heading(_)) {
        content
            .id(SharedString::from(format!("doc-heading-{index}")))
            .into_any_element()
    } else {
        content.into_any_element()
    }
}

impl Render for DocsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        let input = self.filter(window, cx);
        let query = self.query.to_lowercase();
        let matches = |doc: &&DocSource| {
            query.is_empty()
                || title(doc).to_lowercase().contains(&query)
                || summary(doc).to_lowercase().contains(&query)
                || doc.slug.to_lowercase().contains(&query)
                || doc.file.to_lowercase().contains(&query)
        };
        let mut categories = Vec::new();
        for doc in DOCS.iter().filter(|doc| matches(doc)) {
            if categories.last().copied() != Some(doc.category) {
                categories.push(doc.category);
            }
        }
        let mut nav = Vec::new();
        for category in categories {
            nav.push(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(category.replace('-', " "))
                    .into_any_element(),
            );
            for doc in DOCS
                .iter()
                .filter(|doc| doc.category == category)
                .filter(|doc| matches(doc))
            {
                let title = title(doc);
                let slug = doc.slug.to_owned();
                let selected = self.selected == doc.slug;
                nav.push(
                    div()
                        .id(SharedString::from(format!("doc-{}", doc.slug)))
                        .px_2()
                        .py(px(5.))
                        .rounded(px(5.))
                        .when(selected, |row| row.bg(rgb(palette.raised)))
                        .cursor_pointer()
                        .hover(move |style| style.bg(rgb(palette.raised)))
                        .text_sm()
                        .text_color(rgb(if selected {
                            palette.text
                        } else {
                            palette.muted
                        }))
                        .child(title.to_owned())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open(slug.clone(), cx);
                        }))
                        .into_any_element(),
                );
            }
        }
        let source = self.source();
        let title = title(source);
        let body = source.markdown.replacen(&format!("# {title}"), "", 1);
        let parsed = parse_markdown(&body);
        if let Some(anchor) = self.pending_anchor.take() {
            if let Some(index) = anchor_index(&parsed, &anchor) {
                self.scroll.scroll_to_top_of_item(index + 2);
            }
        }
        let source_url = format!("https://github.com/larsboes/Sjel/blob/main/{}", source.file);
        let mut doc = div()
            .id("docs-document-scroll")
            .flex()
            .flex_col()
            .gap(px(14.))
            .size_full()
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .vertical_scrollbar(&self.scroll)
            .child(
                div()
                    .text_size(px(22.))
                    .text_color(rgb(palette.text))
                    .child(title.to_owned()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(source.file),
                    )
                    .child(
                        Button::new("docs-source")
                            .small()
                            .ghost()
                            .label("Open source")
                            .on_click(move |_, _, cx| cx.open_url(&source_url)),
                    ),
            );
        for (index, block) in parsed.iter().enumerate() {
            doc = doc.child(render_doc_block(
                index,
                block,
                source.file.to_owned(),
                palette,
                cx,
            ));
        }
        div()
            .size_full()
            .flex()
            .gap(px(18.))
            .p(px(16.))
            .child(
                div()
                    .w(px(245.))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(Input::new(&input))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .children(nav),
                    ),
            )
            .child(div().flex_1().min_w_0().child(doc))
    }
}
