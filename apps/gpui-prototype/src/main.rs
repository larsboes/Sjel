use std::time::Duration;
use std::process::Command;

use gpui::{
    App, Bounds, ClickEvent, Context, Render, SharedString, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, rgb, size,
};
use gpui_platform::application;
use serde::Deserialize;

const BACKGROUND: u32 = 0x101318;
const PANEL: u32 = 0x191e25;
const PANEL_RAISED: u32 = 0x202731;
const TEXT: u32 = 0xe8ebef;
const MUTED: u32 = 0x9aa4b2;
const ACCENT: u32 = 0xb8d5b5;
const GREEN: u32 = 0x9acb9b;
const RED: u32 = 0xe49b91;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Capabilities,
    Settings,
}

impl Page {
    fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Capabilities => "Capabilities",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Debug, Deserialize)]
struct Health {
    ok: bool,
    service: String,
}

#[derive(Clone)]
struct Capability {
    name: SharedString,
    state: SharedString,
}

struct Prototype {
    page: Page,
    endpoint: SharedString,
    status: SharedString,
    detail: SharedString,
    checked: SharedString,
    healthy: Option<bool>,
    checking: bool,
    capabilities: Vec<Capability>,
    loading_capabilities: bool,
    capability_error: Option<SharedString>,
}

impl Prototype {
    fn new() -> Self {
        let endpoint = std::env::var("SJEL_STATUS_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8082".to_owned())
            .trim_end_matches('/')
            .to_owned();

        Self {
            page: Page::Overview,
            endpoint: endpoint.into(),
            status: "Not checked".into(),
            detail: "Connect to the local Sjel status service to read its health.".into(),
            checked: "—".into(),
            healthy: None,
            checking: false,
            capabilities: Vec::new(),
            loading_capabilities: false,
            capability_error: None,
        }
    }

    fn refresh(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.checking {
            return;
        }

        let endpoint = self.endpoint.to_string();
        self.checking = true;
        self.status = "Checking…".into();
        self.detail = "Requesting GET /health".into();
        cx.notify();

        let (result_tx, result_rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("Could not start the HTTP runtime: {error}"))
                .and_then(|runtime| runtime.block_on(check_health(&endpoint)));
            let _ = result_tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let result = result_rx
                .await
                .unwrap_or_else(|_| Err("The health request worker stopped unexpectedly.".to_owned()));
            let _ = this.update(cx, |view, cx| {
                view.checking = false;
                view.checked = chrono_label();
                match result {
                    Ok(health) => {
                        view.healthy = Some(health.ok);
                        view.status = if health.ok { "Connected" } else { "Needs attention" }.into();
                        view.detail = format!("{} answered /health", health.service).into();
                    }
                    Err(error) => {
                        view.healthy = Some(false);
                        view.status = "Unavailable".into();
                        view.detail = error.into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn reset_endpoint(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.endpoint = "http://127.0.0.1:8082".into();
        self.status = "Not checked".into();
        self.detail = "Using the local Sjel status service.".into();
        self.checked = "—".into();
        self.healthy = None;
        cx.notify();
    }

    fn refresh_capabilities(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.loading_capabilities {
            return;
        }
        self.loading_capabilities = true;
        self.capability_error = None;
        cx.notify();

        let (result_tx, result_rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let result = Command::new("sjel")
                .args(["capability", "list"])
                .output()
                .map_err(|error| format!("Could not run `sjel capability list`: {error}"))
                .and_then(|output| {
                    if !output.status.success() {
                        return Err(format!(
                            "`sjel capability list` exited with {}: {}",
                            output.status,
                            String::from_utf8_lossy(&output.stderr).trim()
                        ));
                    }
                    let rows = String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .filter_map(|line| {
                            let mut fields = line.split_whitespace();
                            let name = fields.next()?;
                            let state = fields.next_back()?;
                            Some(Capability {
                                name: name.to_owned().into(),
                                state: state.to_owned().into(),
                            })
                        })
                        .collect::<Vec<_>>();
                    if rows.is_empty() {
                        Err("The capability registry returned no rows.".to_owned())
                    } else {
                        Ok(rows)
                    }
                });
            let _ = result_tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let result = result_rx.await.unwrap_or_else(|_| {
                Err("The capability lookup worker stopped unexpectedly.".to_owned())
            });
            let _ = this.update(cx, |view, cx| {
                view.loading_capabilities = false;
                match result {
                    Ok(rows) => {
                        view.capabilities = rows;
                        view.capability_error = None;
                    }
                    Err(error) => view.capability_error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

async fn check_health(base: &str) -> Result<Health, String> {
    if !(base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:")) {
        return Err("For this prototype, SJEL_STATUS_URL must point to loopback.".to_owned());
    }

    let response = reqwest::Client::new()
        .get(format!("{base}/health"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map_err(|error| format!("Could not reach the local service: {error}"))?;

    if !response.status().is_success() {
        return Err(format!("The service returned HTTP {}.", response.status()));
    }

    response
        .json::<Health>()
        .await
        .map_err(|error| format!("The service returned an unreadable health response: {error}"))
}

fn chrono_label() -> SharedString {
    // Keep the prototype dependency-light and avoid reading any machine-specific state.
    "just now".into()
}

impl Render for Prototype {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status_color = match self.healthy {
            Some(true) => GREEN,
            Some(false) => RED,
            None => MUTED,
        };
        let status = self.status.clone();
        let detail = self.detail.clone();
        let endpoint = self.endpoint.clone();
        let checked = self.checked.clone();
        let button_label = if self.checking { "Checking" } else { "Check connection" };
        let page = self.page;
        let capabilities = self.capabilities.clone();
        let capability_error = self.capability_error.clone();
        let loading_capabilities = self.loading_capabilities;
        let page_content = match page {
            Page::Overview => overview_page(
                status,
                detail,
                endpoint.clone(),
                checked,
                status_color,
                button_label,
                cx,
            )
            .into_any_element(),
            Page::Capabilities => capabilities_page(
                capabilities,
                capability_error,
                loading_capabilities,
                cx,
            )
            .into_any_element(),
            Page::Settings => settings_page(endpoint, cx).into_any_element(),
        };

        let overview_nav = nav_item(Page::Overview, page, cx);
        let capabilities_nav = nav_item(Page::Capabilities, page, cx);
        let settings_nav = nav_item(Page::Settings, page, cx);

        div()
            .size_full()
            .flex()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .child(
                div()
                    .w(px(224.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap_8()
                    .p_6()
                    .bg(rgb(0x15191f))
                    .border_r_1()
                    .border_color(rgb(0x292f38))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .size(px(30.))
                                    .rounded(px(9.))
                                    .bg(rgb(ACCENT))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(rgb(BACKGROUND))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("S"),
                            )
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child("Sjel"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(label("WORKSPACE"))
                            .child(overview_nav)
                            .child(capabilities_nav)
                            .child(settings_nav),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(label("EVALUATION BUILD"))
                            .child(text("Native shell · GPUI", MUTED)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .gap_8()
                    .p_10()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(text("LOCAL NODE", MUTED))
                                    .child(div().text_2xl().font_weight(gpui::FontWeight::MEDIUM).child(page.label())),
                            )
                            .child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .rounded(px(8.))
                                    .bg(rgb(PANEL_RAISED))
                                    .text_sm()
                                    .text_color(rgb(MUTED))
                                    .child("Desktop prototype"),
                            ),
                    )
                    .child(page_content),
            )
    }
}

fn label(value: &'static str) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(rgb(MUTED))
        .child(value)
}

fn text(value: impl Into<SharedString>, color: u32) -> impl IntoElement {
    div().text_sm().text_color(rgb(color)).child(value.into())
}

fn nav_item(page: Page, selected_page: Page, cx: &mut Context<Prototype>) -> impl IntoElement {
    div()
        .id(match page {
            Page::Overview => "nav-overview",
            Page::Capabilities => "nav-capabilities",
            Page::Settings => "nav-settings",
        })
        .px_3()
        .py_2()
        .rounded(px(7.))
        .bg(if page == selected_page { rgb(PANEL_RAISED) } else { rgb(0x15191f) })
        .text_color(rgb(if page == selected_page { TEXT } else { MUTED }))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(PANEL_RAISED)).text_color(rgb(TEXT)))
        .on_click(cx.listener(move |this, _: &ClickEvent, _: &mut Window, cx| {
            this.page = page;
            cx.notify();
        }))
        .child(page.label())
}

fn action_button(
    id: &'static str,
    label: &'static str,
    listener: impl Fn(&mut Prototype, &ClickEvent, &mut Window, &mut Context<Prototype>) + 'static,
    cx: &mut Context<Prototype>,
) -> impl IntoElement {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded(px(8.))
        .bg(rgb(ACCENT))
        .text_color(rgb(0x182019))
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0xc9e4c6)))
        .on_click(cx.listener(listener))
        .child(label)
}

fn overview_page(
    status: SharedString,
    detail: SharedString,
    endpoint: SharedString,
    checked: SharedString,
    status_color: u32,
    button_label: &'static str,
    cx: &mut Context<Prototype>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_5()
                .p_6()
                .rounded(px(14.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(0x2b333d))
                .child(label("CONNECTION"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().size(px(9.)).rounded_full().bg(rgb(status_color)))
                        .child(div().text_xl().font_weight(gpui::FontWeight::MEDIUM).child(status)),
                )
                .child(div().text_color(rgb(MUTED)).child(detail))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .border_t_1()
                        .border_color(rgb(0x2b333d))
                        .pt_4()
                        .child(text(format!("Endpoint  {endpoint}/health"), MUTED))
                        .child(text(format!("Last checked  {checked}"), MUTED)),
                )
                .child(action_button(
                    "refresh-health",
                    button_label,
                    Prototype::refresh,
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .gap_4()
                .child(info_card("Transport", "Loopback HTTP", "Reads the public liveness endpoint only."))
                .child(info_card("Data access", "Capability APIs", "No direct database access in this prototype.")),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_6()
                .rounded(px(14.))
                .bg(rgb(PANEL))
                .border_1()
                .border_color(rgb(0x2b333d))
                .child(label("NEXT EVALUATION STEP"))
                .child(text("Add a read-only capability list", TEXT))
                .child(text("The current service registry can support a real Sjel view after the API boundary and auth behavior are agreed.", MUTED)),
        )
}

fn capabilities_page(
    capabilities: Vec<Capability>,
    error: Option<SharedString>,
    loading: bool,
    cx: &mut Context<Prototype>,
) -> impl IntoElement {
    let button_label = if loading { "Refreshing…" } else { "Refresh capabilities" };
    let has_rows = !capabilities.is_empty();
    let mut rows = div().flex().flex_col().gap_2();
    for capability in capabilities {
        let state = capability.state.to_string();
        let color = match state.as_str() {
            "up" => GREEN,
            "down" => RED,
            _ => MUTED,
        };
        rows = rows.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_4()
                .py_3()
                .rounded(px(9.))
                .bg(rgb(PANEL_RAISED))
                .child(text(capability.name, TEXT))
                .child(text(state, color)),
        );
    }

    let mut page = div()
        .flex()
        .flex_col()
        .gap_5()
        .p_6()
        .rounded(px(14.))
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(0x2b333d))
        .child(label("LIVE REGISTRY"))
        .child(div().text_xl().child("Capabilities"))
        .child(text("Read from `sjel capability list`; the app displays capability names and their Sjel health state.", MUTED))
        .child(action_button(
            "refresh-capabilities",
            button_label,
            Prototype::refresh_capabilities,
            cx,
        ));
    if let Some(error) = error {
        page = page.child(text(error, RED));
    } else if !has_rows {
        page = page.child(text("Refresh to load the local capability registry.", MUTED));
    } else {
        page = page.child(rows);
    }
    page
}

fn settings_page(endpoint: SharedString, cx: &mut Context<Prototype>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .p_6()
        .rounded(px(14.))
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(0x2b333d))
        .child(label("LOCAL CONNECTION"))
        .child(div().text_xl().child("Status endpoint"))
        .child(text(endpoint, TEXT))
        .child(text("Requests are restricted to localhost or 127.0.0.1. No credentials are sent.", MUTED))
        .child(action_button(
            "reset-endpoint",
            "Use default endpoint",
            Prototype::reset_endpoint,
            cx,
        ))
}

fn info_card(title: &'static str, value: &'static str, note: &'static str) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap_3()
        .p_5()
        .rounded(px(12.))
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(0x2b333d))
        .child(label(title))
        .child(div().text_base().child(value))
        .child(text(note, MUTED))
}

fn main() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1040.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Prototype::new()),
        )
        .expect("open GPUI prototype window");
        cx.activate(true);
    });
}
