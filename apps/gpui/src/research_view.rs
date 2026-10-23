//! Research: Architecture, Local Decision Models, Upstreams & Benchmarks.
//!
//! Replaces the dashboard `/research` routes (`/research`, `/research/projects`, `/research/benchmarks`).
//! Backed by `core::research` and `POST /api/sjel-status/upstreams/watch`.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, AnyElement, Context, Entity, IntoElement, Subscription, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::theme::Palette;
use crate::ui::{card, dot, note, placeholder, section};
use sjel_gpui_core::research::{
    group_upstreams, BenchmarkSuite, ResearchArticle, ResearchView, SystemRow, UpstreamRow,
};

const SUMMARY_MAX: usize = 110;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchTab {
    Articles,
    Projects,
    Benchmarks,
}

pub struct ResearchPage {
    repo: Repo,
    research: Remote<ResearchView>,
    busy: Busy,
    notice: Option<String>,
    write_error: Option<String>,

    active_tab: ResearchTab,
    selected_slug: String,
    filter_query: String,
    show_add_drawer: bool,

    filter_input: Option<(Entity<InputState>, Subscription)>,
    add_url_input: Option<(Entity<InputState>, Subscription)>,
    add_summary_input: Option<(Entity<InputState>, Subscription)>,
    add_name_input: Option<(Entity<InputState>, Subscription)>,
}

impl ResearchPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            research: Remote::default(),
            busy: Busy::default(),
            notice: None,
            write_error: None,

            active_tab: ResearchTab::Articles,
            selected_slug: "index".to_string(),
            filter_query: String::new(),
            show_add_drawer: false,

            filter_input: None,
            add_url_input: None,
            add_summary_input: None,
            add_name_input: None,
        }
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.research,
            &repo,
            |shell| async move { shell.research_view().await },
        );
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    pub fn switch_tab(&mut self, tab: ResearchTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    fn ensure_filter_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.filter_input {
            return input.clone();
        }
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder("Filter by name, summary, license…"));
        let subscription =
            cx.subscribe_in(&input, window, |this, input, _event: &InputEvent, _, cx| {
                this.filter_query = input.read(cx).value().to_string();
                cx.notify();
            });
        self.filter_input = Some((input.clone(), subscription));
        input
    }

    fn ensure_add_url_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.add_url_input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repo"));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.add_url_input = Some((input.clone(), subscription));
        input
    }

    fn ensure_add_summary_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.add_summary_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("What it is, and why it is worth a look")
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.add_summary_input = Some((input.clone(), subscription));
        input
    }

    fn ensure_add_name_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = &self.add_name_input {
            return input.clone();
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Optional name (derived from URL if empty)")
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |_view, _input, _event: &InputEvent, _, cx| {
                cx.notify();
            },
        );
        self.add_name_input = Some((input.clone(), subscription));
        input
    }

    fn add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url_field = self.ensure_add_url_input(window, cx);
        let summary_field = self.ensure_add_summary_input(window, cx);
        let name_field = self.ensure_add_name_input(window, cx);

        let url = url_field.read(cx).value().trim().to_string();
        let summary = summary_field.read(cx).value().trim().to_string();
        let name_raw = name_field.read(cx).value().trim().to_string();
        let name = if name_raw.is_empty() {
            None
        } else {
            Some(name_raw)
        };

        if url.is_empty() || summary.is_empty() || summary.len() > SUMMARY_MAX {
            self.write_error = Some("Valid URL and summary (≤ 110 chars) required.".to_string());
            cx.notify();
            return;
        }

        let repo = self.repo.clone();
        let key = format!("watch:{}", url);

        self.notice = None;
        self.write_error = None;

        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Adding project to watch list…"),
            move |shell| async move { shell.watch_upstream(&url, &summary, name.as_deref()).await },
            move |this, result, cx| {
                match result {
                    Some(row) => {
                        this.notice =
                            Some(format!("Added '{}' to upstreams.toml as watch.", row.name));
                        this.show_add_drawer = false;
                    }
                    None => {
                        this.write_error = Some("Failed to add project to watch list.".to_string());
                    }
                }
                this.load(cx);
            },
        );
    }

    // ── Components ────────────────────────────────────────────────────────────

    fn render_tabs(&self, _palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let tab_btn = |tab: ResearchTab, label: &'static str, cx: &mut Context<Self>| {
            let active = self.active_tab == tab;
            Button::new(label)
                .small()
                .when(!active, |b| b.ghost())
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switch_tab(tab, cx);
                }))
        };

        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(tab_btn(ResearchTab::Articles, "Articles", cx))
            .child(tab_btn(ResearchTab::Projects, "Projects", cx))
            .child(tab_btn(ResearchTab::Benchmarks, "Benchmarks", cx))
            .into_any_element()
    }

    // ── Tab 1: Articles ───────────────────────────────────────────────────────

    fn render_articles_tab(
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
                    d.bg(rgb(palette.surface))
                        .border_l_2()
                        .border_color(rgb(palette.accent))
                })
                .when(!active, |d| d.hover(|h| h.bg(rgb(palette.raised))))
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
            .border_color(rgb(palette.border))
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
                        .border_color(rgb(palette.border))
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
                                .bg(rgb(palette.surface))
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

    // ── Tab 2: Projects ───────────────────────────────────────────────────────

    fn render_projects_tab(
        &mut self,
        systems: &[SystemRow],
        upstreams: &[UpstreamRow],
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let filter_field = self.ensure_filter_input(window, cx);
        let q = self.filter_query.to_lowercase();

        let filtered_systems: Vec<&SystemRow> = systems
            .iter()
            .filter(|s| {
                if q.is_empty() {
                    return true;
                }
                s.name.to_lowercase().contains(&q)
                    || s.why.to_lowercase().contains(&q)
                    || s.kind.to_lowercase().contains(&q)
            })
            .collect();

        let filtered_upstreams: Vec<UpstreamRow> = upstreams
            .iter()
            .filter(|u| {
                if q.is_empty() {
                    return true;
                }
                u.name.to_lowercase().contains(&q)
                    || u.summary.to_lowercase().contains(&q)
                    || u.verdict.to_lowercase().contains(&q)
                    || u.license.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();

        let groups = group_upstreams(&filtered_upstreams);

        let mut panels = Vec::new();

        // 1. Add drawer
        let show_drawer = self.show_add_drawer;
        let drawer_toggle = Button::new("toggle-add-project")
            .small()
            .ghost()
            .label(if show_drawer {
                "Hide Watch Form"
            } else {
                "+ Add a project to watch"
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_add_drawer = !this.show_add_drawer;
                cx.notify();
            }));

        let add_header = div()
            .flex()
            .justify_between()
            .items_center()
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Watch a Project"),
            )
            .child(drawer_toggle);

        let add_card = if show_drawer {
            let url_field = self.ensure_add_url_input(window, cx);
            let sum_field = self.ensure_add_summary_input(window, cx);
            let name_field = self.ensure_add_name_input(window, cx);

            let sum_len = sum_field.read(cx).value().trim().len();
            let sum_color = if sum_len > SUMMARY_MAX {
                palette.red
            } else {
                palette.muted
            };

            let form_body = div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(add_header)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child("Repository URL"),
                        )
                        .child(Input::new(&url_field)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(rgb(palette.muted))
                                        .child("Summary"),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(rgb(sum_color))
                                        .child(format!("{sum_len} / {SUMMARY_MAX}")),
                                ),
                        )
                        .child(Input::new(&sum_field)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child("Name (optional)"),
                        )
                        .child(Input::new(&name_field)),
                )
                .child(
                    div().flex().justify_end().pt(px(4.0)).child(
                        Button::new("submit-watch-project")
                            .small()
                            .label("Add to Watch")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_project(window, cx);
                            })),
                    ),
                );

            card(form_body.into_any_element(), palette)
        } else {
            card(add_header.into_any_element(), palette)
        };
        panels.push(add_card);

        // 2. Search filter
        panels.push(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .w_full()
                .child(div().flex_1().child(Input::new(&filter_field)))
                .into_any_element(),
        );

        // 3. Runs alongside section (systems.toml)
        let sys_rows = filtered_systems.iter().map(|s| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .p(px(10.0))
                .bg(rgb(palette.surface))
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(palette.border))
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
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(s.name.clone()),
                                )
                                .when(s.local, |d| {
                                    d.child(
                                        div()
                                            .px(px(5.0))
                                            .py(px(1.0))
                                            .bg(rgb(palette.raised))
                                            .rounded(px(3.0))
                                            .text_size(px(9.0))
                                            .text_color(rgb(palette.muted))
                                            .child("local"),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .bg(rgb(palette.raised))
                                .rounded(px(4.0))
                                .text_size(px(10.0))
                                .text_color(rgb(palette.accent))
                                .child(s.kind.clone()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(palette.text))
                        .child(s.why.clone()),
                )
                .when_some(s.url.clone(), |d, url| {
                    d.child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(palette.muted))
                            .child(url),
                    )
                })
                .into_any_element()
        });

        let systems_panel = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(format!("Runs alongside ({})", filtered_systems.len())),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(
                                "Systems Sjel connects to and leaves in charge of their own job.",
                            ),
                    ),
            )
            .child(div().flex().flex_col().gap(px(6.0)).children(sys_rows));
        panels.push(card(systems_panel.into_any_element(), palette));

        // 4. Upstreams grouped by verdict
        for group in groups {
            if group.rows.is_empty() {
                continue;
            }

            let g_badge_color = match group.id.as_str() {
                "built-on" => palette.green,
                "learned-from" => palette.accent,
                "watching" => palette.amber,
                _ => palette.muted,
            };

            let rows = group.rows.iter().map(|u| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .p(px(8.0))
                    .bg(rgb(palette.surface))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.text))
                                            .child(u.name.clone()),
                                    )
                                    .when(!u.license.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .px(px(4.0))
                                                .py(px(1.0))
                                                .bg(rgb(palette.raised))
                                                .rounded(px(3.0))
                                                .text_size(px(9.0))
                                                .text_color(rgb(palette.muted))
                                                .child(u.license.clone()),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .bg(rgb(palette.raised))
                                    .rounded(px(4.0))
                                    .text_size(px(10.0))
                                    .text_color(rgb(g_badge_color))
                                    .child(u.verdict.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(palette.text))
                            .child(u.summary.clone()),
                    )
                    .when(!u.url.is_empty(), |d| {
                        d.child(
                            div()
                                .text_size(px(10.0))
                                .text_color(rgb(palette.muted))
                                .child(u.url.clone()),
                        )
                    })
                    .into_any_element()
            });

            let g_panel = div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(palette.text))
                                .child(format!("{} ({})", group.label, group.rows.len())),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(group.blurb.clone()),
                        ),
                )
                .child(div().flex().flex_col().gap(px(6.0)).children(rows));

            panels.push(card(g_panel.into_any_element(), palette));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .children(panels)
            .into_any_element()
    }

    // ── Tab 3: Benchmarks ─────────────────────────────────────────────────────

    fn render_benchmarks_tab(&self, suites: &[BenchmarkSuite], palette: Palette) -> AnyElement {
        if suites.is_empty() {
            return note("No benchmark suites recorded.", palette.muted);
        }

        let mut panels = Vec::new();

        for suite in suites {
            // Runs table
            let run_rows = suite.runs.iter().map(|run| {
                let acc = run.result.accuracy;
                let acc_pct = format!("{:.0}%", acc * 100.0);
                let acc_color = if acc >= 0.90 {
                    palette.green
                } else if acc >= 0.75 {
                    palette.amber
                } else {
                    palette.red
                };

                div()
                    .flex()
                    .items_center()
                    .py(px(6.0))
                    .px(px(8.0))
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .w(px(180.0))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(run.result.model.clone()),
                    )
                    .child(
                        div()
                            .w(px(90.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(run.result.backend.clone()),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(12.0))
                            .text_color(rgb(palette.text))
                            .child(format!("{}/{}", run.result.correct, run.result.n)),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(12.0))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(acc_color))
                            .child(acc_pct),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(format!("{:.0} ms", run.result.latency_ms.p50)),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(format!("{:.0} ms", run.result.latency_ms.p95)),
                    )
                    .child(
                        div()
                            .w(px(110.0))
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(run.result.host.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(run.result.date.clone()),
                    )
                    .into_any_element()
            });

            let runs_table_header = div()
                .flex()
                .items_center()
                .py(px(4.0))
                .px(px(8.0))
                .border_b_1()
                .border_color(rgb(palette.border))
                .text_size(px(10.0))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(div().w(px(180.0)).child("MODEL"))
                .child(div().w(px(90.0)).child("BACKEND"))
                .child(div().w(px(80.0)).child("CORRECT"))
                .child(div().w(px(80.0)).child("ACCURACY"))
                .child(div().w(px(80.0)).child("P50"))
                .child(div().w(px(80.0)).child("P95"))
                .child(div().w(px(110.0)).child("HOST"))
                .child(div().flex_1().child("DATE"));

            // Per-case matrix table
            let case_rows = suite.case_ids.iter().map(|case_id| {
                let verdicts = suite.runs.iter().map(|run| {
                    let passed = run
                        .result
                        .cases
                        .iter()
                        .find(|c| c.id == *case_id)
                        .map_or(false, |c| c.expected == c.got);

                    let (symbol, color) = if passed {
                        ("✓", palette.green)
                    } else {
                        ("✗", palette.red)
                    };

                    div()
                        .w(px(80.0))
                        .text_size(px(12.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(color))
                        .child(symbol)
                        .into_any_element()
                });

                div()
                    .flex()
                    .items_center()
                    .py(px(4.0))
                    .px(px(8.0))
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    .child(
                        div()
                            .w(px(220.0))
                            .text_size(px(11.0))
                            .font_family(".AppleSystemUIFontMonospaced")
                            .text_color(rgb(palette.text))
                            .child(case_id.clone()),
                    )
                    .children(verdicts)
                    .into_any_element()
            });

            let matrix_header_runs = suite.runs.iter().map(|run| {
                div()
                    .w(px(80.0))
                    .text_size(px(10.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(run.result.model.clone())
                    .into_any_element()
            });

            let matrix_header = div()
                .flex()
                .items_center()
                .py(px(4.0))
                .px(px(8.0))
                .border_b_1()
                .border_color(rgb(palette.border))
                .child(
                    div()
                        .w(px(220.0))
                        .text_size(px(10.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("EVALUATION CASE"),
                )
                .children(matrix_header_runs);

            let suite_card = div()
                .flex()
                .flex_col()
                .gap(px(12.0))
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
                                        .text_size(px(15.0))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(rgb(palette.text))
                                        .child(format!("Suite: {}", suite.name)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(palette.muted))
                                .child(format!("{} runs evaluated", suite.runs.len())),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .bg(rgb(palette.surface))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(rgb(palette.border))
                        .child(runs_table_header)
                        .children(run_rows),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child(format!(
                                    "Per-Case Breakdown ({} test cases)",
                                    suite.case_ids.len()
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .bg(rgb(palette.surface))
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(rgb(palette.border))
                                .child(matrix_header)
                                .children(case_rows),
                        ),
                );

            panels.push(card(suite_card.into_any_element(), palette));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .children(panels)
            .into_any_element()
    }
}

fn render_markdown_prose(content: &str, palette: Palette) -> AnyElement {
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
                        .bg(rgb(palette.surface))
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

impl Render for ResearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());

        let Some(view) = self.research.value.clone() else {
            let body = match &self.research.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(320.),
            };
            return section("Research", body, palette).into_any_element();
        };

        let mut panels = Vec::new();

        if let Some(notice) = &self.notice {
            panels.push(note(notice.as_str(), palette.green));
        }
        if let Some(error) = &self.write_error {
            panels.push(note(error.as_str(), palette.red));
        }

        panels.push(self.render_tabs(palette, cx));

        let active_content = match self.active_tab {
            ResearchTab::Articles => self.render_articles_tab(&view.articles, palette, cx),
            ResearchTab::Projects => {
                self.render_projects_tab(&view.systems, &view.upstreams, palette, window, cx)
            }
            ResearchTab::Benchmarks => self.render_benchmarks_tab(&view.suites, palette),
        };
        panels.push(active_content);

        div()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .p(px(16.0))
            .children(panels)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_research_page_initial_state() {
        let repo = Repo::new(Err(sjel_gpui_core::shell::ShellError::NotSetUp));
        let page = ResearchPage::new(repo);
        assert_eq!(page.active_tab, ResearchTab::Articles);
        assert_eq!(page.selected_slug, "index");
        assert!(!page.show_add_drawer);
        assert!(page.filter_query.is_empty());
    }
}
