//! Documentation and interactive manual viewer for Sjel.

mod diagrams;
mod parser;
mod types;

use gpui::StatefulInteractiveElement;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::{
    FrontmatterPlugin, MarkdownExtensions, TextView, TextViewPlugin as _, TextViewState,
};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, Context, Entity, IntoElement, ScrollHandle, SharedString,
    StyleRefinement, Subscription, Window,
};

pub use diagrams::MermaidPlugin;
pub(crate) use parser::{doc_slug, source_url};
pub(crate) use types::{anchor_source_range, summary, title, DocSource};

use crate::doc_view::text_view_style;
use crate::theme::ThemeContext;

include!(concat!(env!("OUT_DIR"), "/docs.rs"));

pub struct DocsPage {
    selected: String,
    query: String,
    filter: Option<(Entity<InputState>, Subscription)>,
    text_view_state: Option<Entity<TextViewState>>,
    scroll: ScrollHandle,
    nav_scroll: ScrollHandle,
    pending_anchor: Option<String>,
}

impl DocsPage {
    pub fn new() -> Self {
        Self {
            selected: "index".into(),
            query: String::new(),
            filter: None,
            text_view_state: None,
            scroll: ScrollHandle::new(),
            nav_scroll: ScrollHandle::new(),
            pending_anchor: None,
        }
    }

    pub fn scroll_handle(&self) -> ScrollHandle {
        self.scroll.clone()
    }

    pub fn open(&mut self, slug: String, cx: &mut Context<Self>) {
        if DOCS.iter().any(|doc| doc.slug == slug) {
            self.selected = slug;
            self.pending_anchor = None;
            self.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
            cx.notify();
        }
    }

    fn open_anchor(&mut self, slug: String, anchor: Option<String>, cx: &mut Context<Self>) {
        if DOCS.iter().any(|doc| doc.slug == slug) {
            let changed = self.selected != slug;
            self.selected = slug;
            if changed && anchor.is_none() {
                self.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
            }
            self.pending_anchor = anchor;
            cx.notify();
        }
    }

    fn text_state(&mut self, body: &str, cx: &mut Context<Self>) -> Entity<TextViewState> {
        let state = self
            .text_view_state
            .get_or_insert_with(|| cx.new(|cx| TextViewState::markdown(body, cx).selectable(true)))
            .clone();
        state.update(cx, |s, cx| s.set_text(body, cx));
        state
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

    pub(crate) fn follow_link(&mut self, href: &str, file: &str, cx: &mut Context<Self>) {
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

impl Default for DocsPage {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for DocsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeContext::of(window, cx);
        let palette = theme.palette;
        let density = theme.density;
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
                        .when(selected, |row| row.bg(rgba(palette.raised)))
                        .cursor_pointer()
                        .hover(move |style| style.bg(rgba(palette.raised)))
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
        let text_state = self.text_state(&body, cx);
        if let Some(anchor) = self.pending_anchor.take() {
            if let Some(source_range) = anchor_source_range(&body, &anchor) {
                text_state.update(cx, |s, cx| {
                    let rendered = s.rendered_text();
                    if rendered.source() == body {
                        if let Some(rendered_range) = rendered.range_for_source(source_range) {
                            let _ = s.reveal_range(rendered_range.start..rendered_range.start, cx);
                        }
                    }
                });
            }
        }
        let source_url = format!("https://github.com/larsboes/Sjel/blob/main/{}", source.file);
        let file = source.file.to_owned();
        let entity = cx.entity().downgrade();
        let scroll = self.scroll.clone();
        let blockquote = StyleRefinement::default()
            .border_color(rgb(palette.accent))
            .bg(rgba(palette.raised))
            .rounded(px(4.));
        let markdown_view = MermaidPlugin
            .setup(
                FrontmatterPlugin.setup(
                    TextView::new(&text_state)
                        .markdown_extensions(MarkdownExtensions::default().frontmatter()),
                ),
            )
            .style(text_view_style(palette, density))
            .blockquote_style(blockquote)
            .text_color(rgb(palette.text))
            .selectable(true)
            .on_link_click(move |href, _, _, cx| {
                let href = href.to_string();
                let file = file.clone();
                let _ = entity.update(cx, |this, cx| {
                    this.follow_link(&href, &file, cx);
                });
            })
            .on_reveal(move |line, _, _| {
                let viewport = scroll.bounds();
                let mut offset = scroll.offset();
                if line.bottom() > viewport.bottom() {
                    offset.y -= line.bottom() - viewport.bottom();
                } else if line.top() < viewport.top() {
                    offset.y += viewport.top() - line.top();
                }
                scroll.set_offset(offset);
            });
        let doc = div()
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
            )
            .child(markdown_view);
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
                            .id("docs-nav")
                            .flex_1()
                            .min_h_0()
                            .track_scroll(&self.nav_scroll)
                            .overflow_y_scroll()
                            .vertical_scrollbar(&self.nav_scroll)
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .children(nav),
                    ),
            )
            .child(div().flex_1().min_w_0().child(doc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Palette;
    use diagrams::render_mermaid;
    use gpui_kit::component::text::MarkdownPlugin as _;

    #[test]
    fn anchor_source_range_finds_headings_and_skips_code_fences() {
        let md = "Intro\n\n```markdown\n## Fake Heading\n```\n\n## Real Heading\n\n<a id=\"custom-id\"></a>\n";
        let fake = anchor_source_range(md, "fake-heading");
        assert!(fake.is_none());

        let real = anchor_source_range(md, "real-heading").expect("real heading range");
        assert!(md[real].starts_with("## Real Heading"));

        let custom = anchor_source_range(md, "custom-id").expect("custom anchor range");
        assert!(md[custom].starts_with("<a id=\"custom-id\"></a>"));
    }

    #[test]
    fn mermaid_plugin_parses_mermaid_code_blocks_and_renders_svg() {
        let plugin = MermaidPlugin::new();
        assert!(plugin.is_block());
        assert_eq!(plugin.name(), "mermaid");

        let palette = Palette::dark();
        let (svg, width, height) =
            render_mermaid("flowchart LR\n  A[\"Docs\"] --> B[\"TextView\"]", palette)
                .expect("valid mermaid svg");
        assert!(svg.contains("<svg"));
        assert!(width > 0.0);
        assert!(height > 0.0);
    }

    #[test]
    fn doc_slug_resolves_relative_and_root_docs_links() {
        assert_eq!(
            doc_slug("../plans/gpui.md", "docs/guides/setup.md").as_deref(),
            Some("plans/gpui")
        );
        assert_eq!(
            doc_slug("/docs/README.md", "docs/plans/gpui.md").as_deref(),
            Some("index")
        );
    }
}
