//! Media: Exact-byte ingest gate, volume integrity ledger, and run inspection.
//!
//! Replaces the dashboard `/media` route. Backed by `capabilities/media` via sjel-status.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, IntoElement, SharedString, Window};

use crate::data::{self, Remote, Repo};
use crate::theme::Palette;
use crate::primitives::{card, dot, note, placeholder, section};
use sjel_gpui_core::media::{
    format_bytes, run_label, sort_run_items, total_volume_bytes, MediaIngestDetail,
    MediaIngestItem, MediaIngestRun, MediaStatus, MediaView, MediaVolume,
};

pub struct MediaPage {
    repo: Repo,
    media: Remote<MediaView>,
    notice: Option<String>,

    selected_run_id: Option<i64>,
    selected_run_detail: Remote<MediaIngestDetail>,
    filter_disposition: String,
}

impl MediaPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            media: Remote::default(),
            notice: None,

            selected_run_id: None,
            selected_run_detail: Remote::default(),
            filter_disposition: "all".to_string(),
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.media,
            &repo,
            |shell| async move { shell.media_view().await },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
        if let Some(run_id) = self.selected_run_id {
            self.load_run_detail(run_id, cx);
        }
    }

    fn load_run_detail(&mut self, id: i64, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.selected_run_detail,
            &repo,
            move |shell| async move { shell.media_ingest(id).await },
        );
    }

    fn select_run(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.selected_run_id == Some(id) {
            self.selected_run_id = None;
            self.selected_run_detail = Remote::default();
        } else {
            self.selected_run_id = Some(id);
            self.filter_disposition = "all".to_string();
            self.load_run_detail(id, cx);
        }
        cx.notify();
    }

    // ── Components ────────────────────────────────────────────────────────────

    fn render_index_summary(
        &self,
        status: &MediaStatus,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let total_bytes = total_volume_bytes(status);
        let volume_count = status.volumes.len();

        let header = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(dot(palette.accent))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child("Media Ledger & Ingest Gate"),
                            ),
                    )
                    .child(
                        Button::new("media-refresh")
                            .small()
                            .ghost()
                            .label("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(20.0))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(palette.text))
                                    .child(status.files.to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("unique digests"),
                            ),
                    )
                    .child(div().w(px(1.0)).h(px(28.0)).bg(rgb(palette.border)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(20.0))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(palette.text))
                                    .child(status.locations.to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child("indexed paths"),
                            ),
                    )
                    .child(div().w(px(1.0)).h(px(28.0)).bg(rgb(palette.border)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .text_size(px(20.0))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(palette.text))
                                    .child(format_bytes(total_bytes)),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child(format!("across {} volume(s)", volume_count)),
                            ),
                    ),
            );

        card(header.into_any_element(), palette)
    }

    fn render_volumes_table(&self, volumes: &[MediaVolume], palette: Palette) -> AnyElement {
        let volume_rows = if volumes.is_empty() {
            div()
                .p(px(12.0))
                .text_size(px(12.0))
                .text_color(rgb(palette.muted))
                .child("No volumes indexed yet. Run `media index` against a mounted library.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .children(volumes.iter().enumerate().map(|(idx, vol)| {
                    div()
                        .id(SharedString::from(format!("vol-row-{}", idx)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .p(px(8.0))
                        .rounded(px(6.0))
                        .bg(if idx % 2 == 0 {
                            rgb(palette.surface)
                        } else {
                            rgb(palette.window_tint)
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .w(px(240.0))
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(vol.label.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.muted))
                                        .font_family(".AppleSystemUIFontMonospaced")
                                        .child(vol.uuid.clone()),
                                ),
                        )
                        .child(
                            div()
                                .w(px(100.0))
                                .text_size(px(12.0))
                                .text_color(rgb(palette.text))
                                .child(format!("{} paths", vol.locations)),
                        )
                        .child(
                            div()
                                .w(px(100.0))
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(palette.text))
                                .child(format_bytes(vol.bytes)),
                        )
                        .child(
                            div()
                                .w(px(160.0))
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(format!("Last seen: {}", vol.last_seen)),
                        )
                }))
                .into_any_element()
        };

        let content = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child(format!("Mounted Volumes ({})", volumes.len())),
            )
            .child(volume_rows);

        card(content.into_any_element(), palette)
    }

    fn render_ingest_runs_table(
        &self,
        runs: &[MediaIngestRun],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected_id = self.selected_run_id;

        let run_rows = if runs.is_empty() {
            div()
                .p(px(12.0))
                .text_size(px(12.0))
                .text_color(rgb(palette.muted))
                .child("No ingest runs recorded yet.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .children(runs.iter().enumerate().map(|(idx, run)| {
                    let run_id = run.id;
                    let is_selected = selected_id == Some(run_id);
                    let label = run_label(run);
                    let finished_str = run.finished.as_deref().unwrap_or("in progress…");

                    let inspect_btn =
                        Button::new(SharedString::from(format!("run-inspect-{}", run_id)))
                            .small()
                            .when(!is_selected, |b| b.ghost())
                            .label(if is_selected { "Close" } else { "Inspect" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_run(run_id, cx);
                            }));

                    div()
                        .id(SharedString::from(format!("run-row-{}", idx)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .p(px(8.0))
                        .rounded(px(6.0))
                        .bg(if is_selected {
                            rgb(palette.raised)
                        } else if idx % 2 == 0 {
                            rgb(palette.surface)
                        } else {
                            rgb(palette.window_tint)
                        })
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .w(px(220.0))
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.muted))
                                        .truncate()
                                        .child(run.staging.clone()),
                                ),
                        )
                        .child(
                            div()
                                .w(px(80.0))
                                .text_size(px(11.0))
                                .text_color(rgb(palette.text))
                                .child(format!("{} seen", run.considered)),
                        )
                        .child(
                            div()
                                .w(px(80.0))
                                .text_size(px(11.0))
                                .text_color(rgb(palette.green))
                                .child(format!("{} imported", run.imported)),
                        )
                        .child(
                            div()
                                .w(px(80.0))
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(format!("{} dup", run.duplicates)),
                        )
                        .child(div().w(px(80.0)).child(if run.refused > 0 {
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(rgb(palette.amber))
                                .text_size(px(10.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(0xffffff))
                                .child(format!("{} refused", run.refused))
                        } else {
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child("0 refused")
                        }))
                        .child(div().w(px(70.0)).child(if run.failed > 0 {
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(rgb(palette.red))
                                .text_size(px(10.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(0xffffff))
                                .child(format!("{} failed", run.failed))
                        } else {
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child("0 failed")
                        }))
                        .child(
                            div()
                                .w(px(130.0))
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(finished_str.to_string()),
                        )
                        .child(inspect_btn)
                }))
                .into_any_element()
        };

        let content = div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child(format!("Recent Ingest Runs ({})", runs.len())),
            )
            .child(run_rows);

        card(content.into_any_element(), palette)
    }

    fn render_run_detail_card(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let run_id = self.selected_run_id?;

        let body: AnyElement = if let Some(detail) = self.selected_run_detail.value.as_ref() {
            let run = &detail.run;
            let sorted = sort_run_items(&detail.items);

            let filter = self.filter_disposition.as_str();
            let filtered_items: Vec<&MediaIngestItem> = match filter {
                "attention" => sorted
                    .iter()
                    .filter(|i| matches!(i.disposition.as_str(), "failed" | "refused"))
                    .collect(),
                "imported" => sorted
                    .iter()
                    .filter(|i| i.disposition == "imported")
                    .collect(),
                "duplicate" => sorted
                    .iter()
                    .filter(|i| i.disposition == "duplicate")
                    .collect(),
                _ => sorted.iter().collect(),
            };

            let close_btn = Button::new("close-run-detail")
                .small()
                .ghost()
                .label("Close Inspector")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.selected_run_id = None;
                    this.selected_run_detail = Remote::default();
                    cx.notify();
                }));

            let filter_tabs = div()
                .flex()
                .gap(px(6.0))
                .child(
                    Button::new("disp-all")
                        .small()
                        .when(filter != "all", |b| b.ghost())
                        .label(format!("All ({})", sorted.len()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.filter_disposition = "all".to_string();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("disp-attention")
                        .small()
                        .when(filter != "attention", |b| b.ghost())
                        .label(format!("Attention ({})", run.refused + run.failed))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.filter_disposition = "attention".to_string();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("disp-imported")
                        .small()
                        .when(filter != "imported", |b| b.ghost())
                        .label(format!("Imported ({})", run.imported))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.filter_disposition = "imported".to_string();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("disp-duplicate")
                        .small()
                        .when(filter != "duplicate", |b| b.ghost())
                        .label(format!("Duplicate ({})", run.duplicates))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.filter_disposition = "duplicate".to_string();
                            cx.notify();
                        })),
                );

            let item_list = if filtered_items.is_empty() {
                div()
                    .p(px(8.0))
                    .text_size(px(11.0))
                    .text_color(rgb(palette.muted))
                    .child("No paths matching this filter.")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .children(filtered_items.into_iter().enumerate().map(|(idx, item)| {
                        let badge_bg = match item.disposition.as_str() {
                            "failed" => rgb(palette.red),
                            "refused" => rgb(palette.amber),
                            "imported" => rgb(palette.green),
                            "resumed" => rgb(palette.accent),
                            _ => rgb(palette.muted),
                        };

                        let imported_text = item
                            .imported_relpath
                            .as_deref()
                            .map(|p| format!(" → {}", p))
                            .unwrap_or_default();

                        div()
                            .id(SharedString::from(format!("item-{}", idx)))
                            .flex()
                            .items_center()
                            .justify_between()
                            .p(px(6.0))
                            .rounded(px(4.0))
                            .bg(rgb(palette.surface))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(6.0))
                                            .child(
                                                div()
                                                    .px(px(6.0))
                                                    .py(px(1.0))
                                                    .rounded(px(3.0))
                                                    .bg(badge_bg)
                                                    .text_size(px(10.0))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(rgb(0xffffff))
                                                    .child(item.disposition.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(12.0))
                                                    .font_weight(gpui::FontWeight::MEDIUM)
                                                    .text_color(rgb(palette.text))
                                                    .child(item.relpath.clone()),
                                            )
                                            .children(item.verified.then(|| {
                                                div()
                                                    .px(px(4.0))
                                                    .py(px(1.0))
                                                    .rounded(px(3.0))
                                                    .bg(rgb(palette.green))
                                                    .text_size(px(9.0))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(rgb(0xffffff))
                                                    .child("verified")
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(rgb(palette.muted))
                                            .child(format!("{}{}", item.reason, imported_text)),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(palette.muted))
                                    .child(item.size.map(format_bytes).unwrap_or_default()),
                            )
                    }))
                    .into_any_element()
            };

            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(format!(
                                            "Ingest Run Inspection: {}",
                                            run_label(run)
                                        )),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.muted))
                                        .child(format!(
                                            "Source: {} | Staging: {}",
                                            run.source, run.staging
                                        )),
                                ),
                        )
                        .child(close_btn),
                )
                .child(filter_tabs)
                .child(item_list)
                .into_any_element()
        } else if let Some(err) = &self.selected_run_detail.error {
            div()
                .p(px(8.0))
                .text_size(px(12.0))
                .text_color(rgb(palette.red))
                .child(format!("Could not load run #{}: {}", run_id, err))
                .into_any_element()
        } else {
            placeholder(120.0).into_any_element()
        };

        Some(card(body, palette))
    }
}

impl Render for MediaPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());

        let Some(view) = self.media.value.clone() else {
            let body = match &self.media.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Media", body, palette).into_any_element();
        };

        let mut panels = Vec::new();

        if let Some(notice) = &self.notice {
            panels.push(note(notice.as_str(), palette.green));
        }

        panels.push(self.render_index_summary(&view.status, palette, cx));
        panels.push(self.render_volumes_table(&view.status.volumes, palette));
        panels.push(self.render_ingest_runs_table(&view.runs, palette, cx));

        if let Some(inspector) = self.render_run_detail_card(palette, cx) {
            panels.push(inspector);
        }

        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .p(px(16.0))
            .children(panels)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_page_initial_state() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = MediaPage::new(repo);
        assert!(page.selected_run_id.is_none());
        assert_eq!(page.filter_disposition, "all");
        assert!(page.notice.is_none());
    }
}
