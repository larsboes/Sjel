//! Research articles reading view and markdown rendering.
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context,
};
use sjel_gpui_core::research::ResearchArticle;

use crate::primitives::{card, dot, note};
use crate::theme::Palette;

use super::ResearchPage;

impl ResearchPage {
    pub(super) fn render_articles_tab(
        &self,
        articles: &[ResearchArticle],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected_article = articles
            .iter()
            .find(|a| a.slug == self.selected_slug)
            .or_else(|| articles.first());

        // Left rail: list of articles
        let article_buttons = articles.iter().map(|art| {
            let active = selected_article.map_or(false, |a| a.slug == art.slug);
            let slug = art.slug.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .p(px(8.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(rgba(palette.surface))
                        .border_l_2()
                        .border_color(rgb(palette.accent))
                })
                .when(!active, |d| d.hover(|h| h.bg(rgba(palette.raised))))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(if active {
                            gpui::FontWeight::SEMIBOLD
                        } else {
                            gpui::FontWeight::NORMAL
                        })
                        .text_color(if active {
                            rgb(palette.text)
                        } else {
                            rgb(palette.muted)
                        })
                        .child(art.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgb(palette.muted))
                        .child(art.file.clone()),
                )
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.selected_slug = slug.clone();
                        cx.notify();
                    }),
                )
                .into_any_element()
        });

        let rail = div()
            .w(px(220.0))
            .flex()
            .flex_col()
            .gap(px(4.0))
            .pr(px(12.0))
            .border_r_1()
            .border_color(rgba(palette.border))
            .children(article_buttons);

        // Right pane: article content
        let content_pane = if let Some(art) = selected_article {
            let body_card = div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .max_w(px(720.0))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .border_b_1()
                        .border_color(rgba(palette.border))
                        .pb(px(8.0))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_size(px(18.0))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(palette.text))
                                        .child(art.title.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.muted))
                                        .child(format!("research/{}", art.file)),
                                ),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .bg(rgba(palette.surface))
                                .rounded(px(4.0))
                                .text_size(px(10.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.accent))
                                .child("Research Document"),
                        ),
                )
                .child(render_markdown_prose(&art.body, palette));

            card(body_card.into_any_element(), palette)
        } else {
            note("No research articles loaded.", palette.muted)
        };

        div()
            .flex()
            .gap(px(16.0))
            .child(rail)
            .child(div().flex_1().child(content_pane))
            .into_any_element()
    }
}

pub(super) fn render_markdown_prose(content: &str, palette: Palette) -> AnyElement {
    let mut elements: Vec<AnyElement> = Vec::new();
    let mut in_code_block = false;
    let mut code_lines = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_code_block {
                let code_text = code_lines.join("\n");
                code_lines.clear();
                elements.push(
                    div()
                        .bg(rgba(palette.surface))
                        .p(px(8.0))
                        .rounded(px(6.0))
                        .text_size(px(11.0))
                        .font_family(".AppleSystemUIFontMonospaced")
                        .text_color(rgb(palette.text))
                        .child(code_text)
                        .into_any_element(),
                );
                in_code_block = false;
            } else {
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            code_lines.push(line.to_string());
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with("# ") {
            elements.push(
                div()
                    .text_size(px(17.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(palette.text))
                    .pt(px(10.0))
                    .pb(px(2.0))
                    .child(trimmed.trim_start_matches("# ").to_string())
                    .into_any_element(),
            );
        } else if trimmed.starts_with("## ") {
            elements.push(
                div()
                    .text_size(px(14.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .pt(px(8.0))
                    .pb(px(2.0))
                    .child(trimmed.trim_start_matches("## ").to_string())
                    .into_any_element(),
            );
        } else if trimmed.starts_with("### ") {
            elements.push(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .pt(px(6.0))
                    .child(trimmed.trim_start_matches("### ").to_string())
                    .into_any_element(),
            );
        } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            elements.push(
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.0))
                    .child(div().pt(px(5.0)).child(dot(palette.accent)))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(palette.text))
                            .line_height(px(18.0))
                            .child(trimmed[2..].to_string()),
                    )
                    .into_any_element(),
            );
        } else if trimmed.starts_with("> ") {
            elements.push(
                div()
                    .border_l_2()
                    .border_color(rgb(palette.accent))
                    .pl(px(8.0))
                    .py(px(2.0))
                    .text_size(px(12.0))
                    .text_color(rgb(palette.muted))
                    .line_height(px(18.0))
                    .child(trimmed[2..].to_string())
                    .into_any_element(),
            );
        } else {
            elements.push(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(palette.text))
                    .line_height(px(19.0))
                    .child(trimmed.to_string())
                    .into_any_element(),
            );
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .children(elements)
        .into_any_element()
}
