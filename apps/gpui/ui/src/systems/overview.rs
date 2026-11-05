//! Runtime profile, Mac hardware metrics, and Host watch findings panels.
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, IntoElement, SharedString, Window,
};

use crate::data;
use crate::primitives::{card, list, note, placeholder, section};
use crate::theme::{tint, Palette};
use sjel_gpui_core::systems::{
    clock_label, day_label, gigabytes, percent, temperature, watts, MacmonSample, Temperature,
    RUNTIME_CATEGORIES, RUNTIME_SELECTIONS,
};

use super::types::{MACMON_BEAT, MACMON_START};
use super::SystemsPage;

impl SystemsPage {
    pub(super) fn read_macmon(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_quiet(
            cx,
            self,
            |s| &mut s.macmon,
            &repo,
            |shell| async move { shell.macmon().await },
        );
    }

    pub(super) fn read_findings(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_quiet(
            cx,
            self,
            |s| &mut s.findings,
            &repo,
            |shell| async move { shell.host_watch().await },
        );
    }

    pub(super) fn read_runtime(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read_quiet(
            cx,
            self,
            |s| &mut s.runtime,
            &repo,
            |shell| async move { shell.runtime().await },
        );
    }

    /// Starts macmon. A start waits for the service to come up, so the sample that follows the
    /// toast is the first live one.
    pub(super) fn start_macmon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, MACMON_START, "macmon started."),
            |shell| async move { shell.set_running("macmon", true).await },
            |this, started, cx| {
                if started.is_some() {
                    this.read_macmon(cx);
                }
            },
        );
    }

    pub(super) fn save_runtime(
        &mut self,
        body: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            // The page is a control the operator is watching: a toast per toggle is noise.
            data::Press::quiet(""),
            move |shell| async move { shell.set_runtime(body).await },
            |this, saved, cx| match saved {
                // The write's answer is the freshest state this surface has, so it replaces the
                // read rather than waiting for the next beat to confirm it.
                Some(status) => {
                    this.runtime.value = Some(status);
                    this.runtime.error = None;
                }
                None => this.read_runtime(cx),
            },
        );
    }

    pub(super) fn set_selection(
        &mut self,
        selection: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(status) = self.runtime.value.clone() else {
            return;
        };
        self.save_runtime(
            serde_json::json!({ "selection": selection, "expected_revision": status.revision }),
            window,
            cx,
        );
    }

    pub(super) fn toggle_category(
        &mut self,
        category: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(status) = self.runtime.value.clone() else {
            return;
        };
        let mut allow: Vec<String> = status.allow.clone();
        match allow.iter().position(|entry| entry == category) {
            Some(at) => {
                allow.remove(at);
            }
            None => allow.push(category.to_owned()),
        }
        self.save_runtime(
            serde_json::json!({ "allow": allow, "expected_revision": status.revision }),
            window,
            cx,
        );
    }

    pub(super) fn runtime_panel(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let status = self.runtime.value.clone();
        let body =
            match &status {
                None => match &self.runtime.error {
                    Some(error) => note(error.to_string(), palette.red),
                    None => placeholder(72.),
                },
                Some(status) => {
                    let view = status.view();
                    let selection = status.selection.clone();
                    let busy = self.busy.contains("runtime");
                    let modes: Vec<Button> = RUNTIME_SELECTIONS
                        .iter()
                        .map(|(id, label)| {
                            let chosen = selection == *id;
                            let choice = *id;
                            let selected_choice = selection.clone();
                            Button::new(SharedString::from(format!("runtime-{id}")))
                                .small()
                                .when(!chosen, |button| button.ghost())
                                .label(*label)
                                .loading(busy && chosen)
                                .on_click(cx.listener(move |page, _, window, cx| {
                                    if selected_choice != choice {
                                        page.set_selection(choice, window, cx);
                                    }
                                }))
                        })
                        .collect();
                    // One wrapped row of pills rather than five stacked switches: the set is
                    // small, and a page whose first panel is 300 px tall buries the machine it
                    // exists to describe.
                    let categories: Vec<AnyElement> = RUNTIME_CATEGORIES
                        .iter()
                        .map(|(id, label)| {
                            let allowed = status.allow.iter().any(|entry| entry == id);
                            let choice = *id;
                            let enabled = view.on_the_go && status.configured;
                            div()
                                .id(SharedString::from(format!("runtime-category-{id}")))
                                .px_2()
                                .py(px(3.))
                                .rounded(px(6.))
                                .border_1()
                                .text_xs()
                                .when(enabled, |pill| pill.cursor_pointer())
                                .when(allowed && enabled, |pill| {
                                    pill.bg(rgba(tint(palette.accent, 0x22)))
                                        .border_color(rgba(tint(palette.accent, 0x55)))
                                        .text_color(rgb(palette.accent))
                                })
                                .when(!(allowed && enabled), |pill| {
                                    pill.border_color(rgba(palette.border)).text_color(rgb(
                                        if enabled {
                                            palette.muted
                                        } else {
                                            palette.border
                                        },
                                    ))
                                })
                                .when(enabled, |pill| {
                                    pill.hover(move |style| style.text_color(rgb(palette.text)))
                                })
                                .on_click(cx.listener(move |page, _, window, cx| {
                                    if enabled {
                                        page.toggle_category(choice, window, cx);
                                    }
                                }))
                                .child(*label)
                                .into_any_element()
                        })
                        .collect();
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(palette.text))
                                .child(view.summary()),
                        )
                        .children(status.detail.clone().map(|detail| {
                            div().text_xs().text_color(rgb(palette.amber)).child(detail)
                        }))
                        .child(div().text_xs().text_color(rgb(palette.muted)).child(
                            if status.configured {
                                "Exceptions apply only while On the go. Normal is unrestricted."
                            } else {
                                "Not configured yet: Auto follows fresh power observations."
                            },
                        ))
                        .child(div().flex().flex_wrap().gap_1().children(modes))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .mr_1()
                                        .child("Allowed while On the go"),
                                )
                                .children(categories),
                        )
                        .into_any_element()
                }
            };
        section("Runtime profile", card(body, palette), palette).into_any_element()
    }

    pub(super) fn mac_panel(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let body = match &self.macmon.value {
            Some(sample) => {
                let cards: Vec<AnyElement> = vec![
                    metric(
                        "Temperature",
                        temps(sample, palette),
                        palette,
                    ),
                    metric("Power", power(sample, palette), palette),
                    metric("CPU", cpu(sample, palette), palette),
                    metric("Memory", memory(sample, palette), palette),
                ];
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().flex().flex_wrap().gap_3().children(cards))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "Last updated {} (every {} s)",
                                clock_label(&sample.timestamp),
                                MACMON_BEAT.as_secs()
                            )),
                    )
                    .into_any_element()
            }
            None => match &self.macmon.error {
                // macmon is on demand by design, so a refused request is a state to act on
                // rather than a fault: the button is the whole fix.
                Some(_) => div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.amber))
                            .child("macmon is not sampling this Mac."),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Start it to see temperature, power and utilisation. It stops again on reboot unless its service is kept running."),
                    )
                    .child(
                        div()
                            .w(px(120.))
                            .child(
                                Button::new("start-macmon")
                                    .small()
                                    .label("Start macmon")
                                    .loading(self.busy.contains(MACMON_START))
                                    .on_click(cx.listener(|page, _, window, cx| {
                                        page.start_macmon(window, cx);
                                    })),
                            ),
                    )
                    .into_any_element(),
                None => placeholder(224.),
            },
        };
        section("Mac", body, palette).into_any_element()
    }

    pub(super) fn findings_panel(&self, palette: Palette) -> AnyElement {
        let body = match &self.findings.value {
            None => match &self.findings.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(64.),
            },
            Some(findings) if findings.is_empty() => note("Nothing open.", palette.muted),
            Some(findings) => {
                let rows: Vec<AnyElement> = findings
                    .iter()
                    .map(|finding| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_3()
                            .py_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(div().flex_1().min_w_0().truncate().text_sm().child(
                                        if finding.title.is_empty() {
                                            finding.key.clone()
                                        } else {
                                            finding.title.clone()
                                        },
                                    ))
                                    .child(div().text_xs().text_color(rgb(palette.muted)).child(
                                        format!("since {}", day_label(&finding.first_seen)),
                                    )),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(finding.note.clone()),
                            )
                            .into_any_element()
                    })
                    .collect();
                list(rows, palette)
            }
        };
        section("Host watch", body, palette).into_any_element()
    }
}

fn metric(title: &'static str, body: AnyElement, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .flex_1()
        .min_w(px(240.))
        .child(div().text_xs().text_color(rgb(palette.muted)).child(title))
        .child(body)
        .into_any_element()
}

fn big(value: String, tone: u32) -> AnyElement {
    div()
        .text_xl()
        .text_color(rgb(tone))
        .child(value)
        .into_any_element()
}

fn temps(sample: &MacmonSample, palette: Palette) -> AnyElement {
    let row = |label: &'static str, celsius: f64| {
        let reading = temperature(celsius);
        let tone = match reading {
            Temperature::Cool => palette.green,
            Temperature::Warm => palette.amber,
            Temperature::Hot => palette.red,
        };
        div()
            .flex()
            .items_baseline()
            .gap_2()
            .child(big(format!("{celsius:.0}°"), tone))
            .child(div().text_xs().text_color(rgb(palette.muted)).child(label))
            .into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(row("CPU", sample.temp.cpu_temp_avg))
        .child(row("GPU", sample.temp.gpu_temp_avg))
        .into_any_element()
}

fn power(sample: &MacmonSample, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(big(watts(sample.all_power), palette.text))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_3()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("CPU {}", watts(sample.cpu_power)))
                .child(format!("GPU {}", watts(sample.gpu_power)))
                .child(format!("RAM {}", watts(sample.ram_power)))
                .child(format!("Sys {}", watts(sample.sys_power))),
        )
        .into_any_element()
}

fn cpu(sample: &MacmonSample, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(big(percent(sample.cpu_usage_pct), palette.text))
        .child(bar(sample.cpu_usage_pct as f32, palette.accent, palette))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "P-Cores {:.0} MHz · {}",
                    sample.pcpu_usage[0],
                    percent(sample.pcpu_usage[1])
                ))
                .child(format!(
                    "E-Cores {:.0} MHz · {}",
                    sample.ecpu_usage[0],
                    percent(sample.ecpu_usage[1])
                )),
        )
        .into_any_element()
}

fn memory(sample: &MacmonSample, palette: Palette) -> AnyElement {
    let pressure = if sample.under_pressure() {
        palette.amber
    } else {
        palette.muted
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .text_xs()
                .child(div().text_color(rgb(palette.muted)).child("RAM"))
                .child(div().text_color(rgb(palette.text)).child(format!(
                    "{} / {}",
                    gigabytes(sample.memory.ram_usage),
                    gigabytes(sample.memory.ram_total)
                ))),
        )
        .child(bar(sample.ram_ratio(), palette.accent, palette))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .text_xs()
                .child(div().text_color(rgb(palette.muted)).child("Swap"))
                .child(div().text_color(rgb(pressure)).child(format!(
                    "{} / {}",
                    gigabytes(sample.memory.swap_usage),
                    gigabytes(sample.memory.swap_total)
                ))),
        )
        .child(bar(sample.swap_ratio(), pressure, palette))
        .into_any_element()
}

/// A full-width track with its filled part. The width lives on the parent, so a card and a
/// class row can share one bar rather than each writing its own geometry.
pub(super) fn bar(ratio: f32, tone: u32, palette: Palette) -> AnyElement {
    let clamped = ratio.clamp(0.0, 1.0);
    div()
        .w_full()
        .h(px(6.))
        .rounded_full()
        .bg(rgba(palette.border))
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(rgb(tone))
                .w(gpui_kit::relative(clamped)),
        )
        .into_any_element()
}
