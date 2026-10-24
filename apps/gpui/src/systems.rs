//! Systems: what this Mac is doing now, and what it is carrying.
//!
//! Six panels, each one another owner's answer with the page's own wording on top:
//!
//! - **Runtime profile** — which profile is in force and what it admits (`sjel_runtime`).
//! - **Mac** — temperature, power, CPU and memory from macmon, every three seconds.
//! - **Agent** — the agent's reach on this machine: one mode per capability, the writes
//!   waiting for the owner, and the latest calls (ISA F10).
//! - **Storage** — what fills the disk, from `tools/storage report --json`, plus the disk's own
//!   hourly history so a slope reads where a snapshot cannot.
//! - **Updates** — installed software outside this checkout and who moves it, with one button
//!   per class. The one long-running write on this page.
//! - **Host watch** — the conditions the hourly watch has now, and since when.
//!
//! Storage, updates and host watch are read from sjel-status rather than from the tools, because
//! the tools measure and exit; the shell is the surface that stays up. macmon is a capability, so
//! `/macmon/json` reaches it through the same proxy every other capability uses.
//!
//! Nothing here is on the ten-second workspace beat: temperature wants three seconds and the
//! update report asks three registries, so this page runs its own clocks while it is visible and
//! stops them when it is not.

use chrono::{Local, TimeZone};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, IntoElement, SharedString, Window,
};

use crate::data::{self, Busy, Remote, Repo};
use crate::shell::{AgentGate, AgentView, Approval};
use crate::theme::{tint, Palette};
use crate::ui::{card, dot, list, note, placeholder, row, section};
use sjel_gpui_core::systems::{
    apply_summary, audit_needs_attention, clock_label, dated_absences, day_label, gigabytes,
    percent, storage_bytes, temperature, version_label, watts, Finding, MacmonSample,
    RuntimeStatus, Severity, StorageHistory, StorageReport, Temperature, UpdateStatus,
    UpdateSurface, UpdatesReport, RUNTIME_CATEGORIES, RUNTIME_SELECTIONS,
};

/// The live sample's beat. The page exists for temperature and power, and macmon samples the
/// SMC every five seconds upstream, so a slower beat would show a stale reading.
const MACMON_BEAT: std::time::Duration = std::time::Duration::from_secs(3);

/// Pending agent writes are what a person is waiting on.
const AGENT_BEAT: std::time::Duration = std::time::Duration::from_secs(5);

/// The runtime profile changes when its owner changes it; a slower beat is enough.
const RUNTIME_BEAT: std::time::Duration = std::time::Duration::from_secs(30);

/// While an apply runs, the report is the progress bar: the route answers before the work starts.
const APPLY_BEAT: std::time::Duration = std::time::Duration::from_secs(4);

/// An apply compiles for minutes, so the watch is bounded: an hour of beats, then it stops
/// rather than polling a dead shell forever.
const APPLY_BEATS: usize = 900;

const MACMON_START: &str = "macmon:start";

pub struct SystemsPage {
    repo: Repo,
    macmon: Remote<MacmonSample>,
    storage: Remote<StorageReport>,
    history: Remote<StorageHistory>,
    updates: Remote<UpdatesReport>,
    findings: Remote<Vec<Finding>>,
    agent: Remote<AgentView>,
    runtime: Remote<RuntimeStatus>,
    busy: Busy,
    /// Bumped on every activation. A loop that sees a different generation stops, which is how
    /// leaving the page stops three clocks without a handle to cancel.
    generation: u64,
}

impl SystemsPage {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            macmon: Remote::default(),
            storage: Remote::default(),
            history: Remote::default(),
            updates: Remote::default(),
            findings: Remote::default(),
            agent: Remote::default(),
            runtime: Remote::default(),
            busy: Busy::default(),
            generation: 0,
        }
    }

    /// Reads everything once and starts the page's clocks.
    ///
    /// One generation for all three, taken here rather than per clock: a clock stops as soon as
    /// it sees a different number, so bumping it once per `start_clock` would leave only the
    /// last one alive.
    pub fn activate(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.load(cx);
        self.start_clock(generation, MACMON_BEAT, |page, cx| page.read_macmon(cx), cx);
        self.start_clock(generation, AGENT_BEAT, |page, cx| page.read_agent(cx), cx);
        self.start_clock(
            generation,
            RUNTIME_BEAT,
            |page, cx| page.read_runtime(cx),
            cx,
        );
    }

    /// Stops the clocks. A page that is no longer drawn must not keep asking the machine.
    pub fn deactivate(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.read_macmon(cx);
        self.read_storage(cx);
        self.read_history(cx);
        self.read_updates(cx);
        self.read_findings(cx);
        self.read_agent(cx);
        self.read_runtime(cx);
    }

    /// One clock: `beat` apart, calling `tick` until the page is left.
    ///
    /// A `tick` that finds the read already in flight does nothing — `Remote::begin` is the
    /// guard — so a slow answer never stacks requests.
    fn start_clock(
        &mut self,
        generation: u64,
        beat: std::time::Duration,
        tick: fn(&mut Self, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor().timer(beat).await;
            let live = weak
                .update(cx, |page, cx| {
                    if page.generation != generation {
                        return false;
                    }
                    tick(page, cx);
                    true
                })
                .unwrap_or(false);
            if !live {
                return;
            }
        })
        .detach();
    }

    fn read_macmon(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.macmon,
            &repo,
            |shell| async move { shell.macmon().await },
        );
    }

    fn read_storage(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.storage,
            &repo,
            |shell| async move { shell.storage().await },
        );
    }

    fn read_history(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.history,
            &repo,
            |shell| async move { shell.storage_history().await },
        );
    }

    fn read_updates(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.updates,
            &repo,
            |shell| async move { shell.updates().await },
        );
    }

    fn read_findings(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.findings,
            &repo,
            |shell| async move { shell.host_watch().await },
        );
    }

    fn read_agent(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.agent,
            &repo,
            |shell| async move { shell.agent().await },
        );
    }

    fn read_runtime(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        data::read(
            cx,
            self,
            |s| &mut s.runtime,
            &repo,
            |shell| async move { shell.runtime().await },
        );
    }

    /// Starts macmon. A start waits for the service to come up, so the sample that follows the
    /// toast is the first live one.
    fn start_macmon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Starts one update class moving, then watches the report until the receipt settles.
    fn apply_updates(&mut self, class: String, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let key = format!("updates:apply:{class}");
        let sent = class.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Apply started."),
            move |shell| async move { shell.apply_updates(&sent).await },
            |this, started, cx| {
                if started.is_some() {
                    this.watch_apply(cx);
                }
                this.read_updates(cx);
            },
        );
    }

    /// The report *is* the progress: the route answered `202` and the tool writes a receipt the
    /// next report carries. Bounded, so a shell that stops answering cannot leave a poll running.
    fn watch_apply(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        cx.spawn(async move |weak, cx| {
            for _ in 0..APPLY_BEATS {
                cx.background_executor().timer(APPLY_BEAT).await;
                let running = weak
                    .update(cx, |page, cx| {
                        if page.generation != generation {
                            return false;
                        }
                        let running = page
                            .updates
                            .value
                            .as_ref()
                            .and_then(|report| report.last_apply.as_ref())
                            .and_then(|receipt| receipt.state.as_deref())
                            == Some("running");
                        if running {
                            page.read_updates(cx);
                        }
                        running
                    })
                    .unwrap_or(false);
                if !running {
                    return;
                }
            }
        })
        .detach();
    }

    fn set_agent_mode(
        &mut self,
        gate: AgentGate,
        mode: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let key = format!("agent:{}", gate.capability);
        let capability = gate.capability.clone();
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, "Mode saved."),
            move |shell| async move { shell.set_agent_mode(&capability, mode).await },
            |this, saved, cx| {
                if saved.is_some() {
                    this.read_agent(cx);
                }
            },
        );
    }

    fn decide_agent(
        &mut self,
        id: String,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let key = format!("approval:{id}");
        let done = if allow { "Allowed." } else { "Denied." };
        data::write(
            cx,
            window,
            self,
            &repo,
            data::Press::once(|s| &mut s.busy, key, done),
            move |shell| async move { shell.decide(&id, allow).await },
            |this, _, cx| this.read_agent(cx),
        );
    }

    fn save_runtime(
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

    fn set_selection(
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

    fn toggle_category(
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

    // ── panels ───────────────────────────────────────────────────────────────

    fn runtime_panel(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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

    fn mac_panel(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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

    fn agent_panel(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let view = self.agent.value.clone();
        let body = match &view {
            None => match &self.agent.error {
                Some(error) => note(error.to_string(), palette.red),
                None => placeholder(160.),
            },
            Some(agent) => {
                let mut rows: Vec<AnyElement> = Vec::new();

                if !agent.pending.is_empty() {
                    rows.push(heading("Waiting for you", palette));
                    for approval in &agent.pending {
                        rows.push(approval_row(approval, palette, &self.busy, cx));
                    }
                }

                rows.push(heading(
                    if agent.enrolled {
                        "What an agent may do"
                    } else {
                        "What an agent may do — no agent is enrolled"
                    },
                    palette,
                ));
                if agent.capabilities.is_empty() {
                    rows.push(
                        div()
                            .px_3()
                            .py_2()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("No capability has started an agent gate on this machine yet.")
                            .into_any_element(),
                    );
                } else {
                    for gate in &agent.capabilities {
                        rows.push(gate_row(gate, agent, palette, &self.busy, cx));
                    }
                }

                if !agent.calls.is_empty() {
                    rows.push(heading("Latest calls", palette));
                    for call in agent.calls.iter().take(8) {
                        let (tone, decision) = match call.decision.as_str() {
                            "read" => (palette.muted, "read"),
                            "allowed" | "auto" => (palette.green, "allowed"),
                            "denied" => (palette.red, "denied"),
                            // A decision this build does not know prints as the tool wrote it,
                            // so a fifth one reaches the reader instead of being swallowed.
                            other => (palette.amber, other),
                        };
                        rows.push(
                            row()
                                .text_xs()
                                .child(dot(tone))
                                .child(
                                    div()
                                        .w(px(120.))
                                        .truncate()
                                        .text_color(rgb(palette.muted))
                                        .child(call.capability.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(format!("{} {}", call.method, call.path)),
                                )
                                .child(
                                    div()
                                        .w(px(56.))
                                        .text_color(rgb(palette.muted))
                                        .child(call.status.to_string()),
                                )
                                .child(
                                    div()
                                        .w(px(64.))
                                        .text_color(rgb(tone))
                                        .child(decision.to_owned()),
                                )
                                .into_any_element(),
                        );
                    }
                }

                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(rows)
                    .into_any_element()
            }
        };
        section("Agent", body, palette).into_any_element()
    }

    fn storage_panel(&mut self, palette: Palette) -> AnyElement {
        let body = match self.storage.value.as_ref().map(StorageReport::view) {
            None => match &self.storage.error {
                // The report comes from sjel-status rather than macmon, so it answers even when
                // macmon is down — which is the state a full disk is hardest to see in anyway.
                Some(error) => note(error.to_string(), palette.red),
                // `report` walks every class with `du`, so the first answer takes seconds.
                None => note("Measuring the disk…", palette.muted),
            },
            Some(view) => {
                let mut rows: Vec<AnyElement> = Vec::new();
                let tone = match view.severity {
                    Severity::Ok => palette.green,
                    Severity::Warn => palette.amber,
                    Severity::Critical => palette.red,
                };
                rows.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(bar(view.used_pct, tone, palette))
                                .child(
                                    div()
                                        .w(px(72.))
                                        .text_xs()
                                        .text_color(rgb(tone))
                                        .child(view.state_label.clone()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{} used of {}", view.used, view.total))
                                .child(format!("{} free", view.free)),
                        )
                        .children(view.reconciliation.clone().map(|(named, unattributed)| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{named} named by policy · {unattributed} unattributed"
                                ))
                        }))
                        .into_any_element(),
                );

                rows.push(trend_row(self.history.value.as_ref(), palette));

                if view.classes.is_empty() {
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Nothing measured in any class.")
                            .into_any_element(),
                    );
                } else {
                    let deltas = self
                        .history
                        .value
                        .as_ref()
                        .map(StorageHistory::class_deltas)
                        .unwrap_or_default();
                    let largest = view.scale_bytes;
                    rows.push(heading("Reclaimable by class", palette));
                    let class_rows: Vec<AnyElement> = view
                        .classes
                        .iter()
                        .map(|class| {
                            let share = class.bytes as f32 / largest as f32;
                            let color = if class.flagged {
                                palette.amber
                            } else {
                                palette.accent
                            };
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_3()
                                .py_1()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(div().truncate().text_sm().child(class.name.clone()))
                                        .child(
                                            div()
                                                .flex()
                                                .gap_2()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .children(
                                                    (!class.applicable)
                                                        .then(|| "report-only".to_owned()),
                                                )
                                                .children(
                                                    class.flagged.then(|| "over flag".to_owned()),
                                                )
                                                .children(
                                                    (class.measured == Some(false))
                                                        .then(|| "partial".to_owned()),
                                                ),
                                        ),
                                )
                                .child(div().w(px(160.)).child(bar(share, color, palette)))
                                .child(
                                    div()
                                        .w(px(150.))
                                        .text_xs()
                                        .text_color(rgb(palette.text))
                                        .child(format!(
                                            "{}{}",
                                            storage_bytes(class.bytes),
                                            deltas
                                                .get(&class.name)
                                                .map(|delta| format!(" ({delta})"))
                                                .unwrap_or_default()
                                        )),
                                )
                                .into_any_element()
                        })
                        .collect();
                    rows.push(list(class_rows, palette));
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} reclaimable — `sjel storage apply`",
                                view.reclaimable_label
                            ))
                            .into_any_element(),
                    );
                }

                if !view.unmeasured.is_empty() {
                    let names: Vec<String> = view
                        .unmeasured
                        .iter()
                        .map(|class| class.name.clone())
                        .collect();
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(format!(
                                "{} name a path that is here and could not be read, so no size is reported rather than a zero: {}",
                                names.len(),
                                names.join(", ")
                            ))
                            .into_any_element(),
                    );
                }
                if !view.classes_absent.is_empty() {
                    let names: Vec<String> = view
                        .classes_absent
                        .iter()
                        .map(|class| class.name.clone())
                        .collect();
                    let dated = dated_absences(&names, "class", self.history.value.as_ref());
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(format!(
                                "The policy names a path that is no longer on this machine: {}",
                                dated.join(", ")
                            ))
                            .into_any_element(),
                    );
                }

                if !view.protected.is_empty() {
                    rows.push(heading("Protected — reported, never cleaned", palette));
                    let protected: Vec<AnyElement> = view
                        .protected
                        .iter()
                        .map(|entry| {
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
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_sm()
                                                .child(entry.path.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.text))
                                                .child(storage_bytes(entry.bytes)),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(entry.reason.clone()),
                                )
                                .into_any_element()
                        })
                        .collect();
                    rows.push(list(protected, palette));
                }

                if view.protected_absent > 0 {
                    let dated = dated_absences(
                        &view.protected_absent_paths,
                        "protected",
                        self.history.value.as_ref(),
                    );
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(format!(
                                "{} of these name a path that is no longer on this machine — {} — so the reason beside it was written when it still existed.",
                                view.protected_absent,
                                dated.join(", ")
                            ))
                            .into_any_element(),
                    );
                }
                if view.protected_unreadable > 0 {
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} of these could not be read, so their size reads 0 MB.",
                                view.protected_unreadable
                            ))
                            .into_any_element(),
                    );
                }
                if !view.services_absent.is_empty() {
                    let dated = dated_absences(
                        &view.services_absent,
                        "service",
                        self.history.value.as_ref(),
                    );
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(format!(
                                "{} declared service(s) not running — {} — so the policy expects something the machine is not running.",
                                view.services_absent.len(),
                                dated.join(", ")
                            ))
                            .into_any_element(),
                    );
                }

                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(rows)
                    .into_any_element()
            }
        };
        section("Storage", body, palette).into_any_element()
    }

    fn updates_panel(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let body = match self.updates.value.as_ref().map(UpdatesReport::view) {
            None => match &self.updates.error {
                Some(error) => note(error.to_string(), palette.red),
                None => note("Asking three registries…", palette.muted),
            },
            Some(view) => {
                let summary = apply_summary(&view, Local::now());
                let mut rows: Vec<AnyElement> = Vec::new();
                if !summary.is_empty() {
                    let attention = audit_needs_attention(
                        view.last_apply
                            .as_ref()
                            .and_then(|last| last.audit.as_deref()),
                    );
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(if attention {
                                palette.amber
                            } else {
                                palette.muted
                            }))
                            .child(summary)
                            .into_any_element(),
                    );
                }
                rows.push(
                    div()
                        .text_xs()
                        .text_color(rgb(if view.stale > 0 {
                            palette.amber
                        } else {
                            palette.muted
                        }))
                        .child(if view.stale > 0 {
                            format!("{} stale", view.stale)
                        } else {
                            "nothing stale".to_owned()
                        })
                        .into_any_element(),
                );

                for group in &view.groups {
                    let surface: UpdateSurface = group.surface.clone();
                    let actionable = group.actionable;
                    let busy =
                        view.busy || self.busy.contains(&format!("updates:apply:{}", surface.id));
                    rows.push(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .pt_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(surface.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(surface.owner_detail.clone()),
                                    ),
                            )
                            .when(actionable > 0 && !busy, |header| {
                                header.child(
                                    Button::new(SharedString::from(format!(
                                        "apply-{}",
                                        surface.id
                                    )))
                                    .small()
                                    .label(format!("Apply {actionable}"))
                                    .on_click(cx.listener(move |page, _, window, cx| {
                                        page.apply_updates(surface.id.clone(), window, cx);
                                    })),
                                )
                            })
                            .into_any_element(),
                    );

                    let update_rows: Vec<AnyElement> = group
                        .rows
                        .iter()
                        .map(|entry| {
                            let (tone, mark) = match entry.status() {
                                UpdateStatus::Stale => (palette.amber, "!"),
                                UpdateStatus::Current => (palette.green, "✓"),
                                UpdateStatus::Unknown => (palette.muted, "?"),
                                UpdateStatus::NotApplicable => (palette.muted, "·"),
                            };
                            row()
                                .text_xs()
                                .child(div().w(px(14.)).text_color(rgb(tone)).child(mark))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_sm()
                                        .child(entry.name.clone()),
                                )
                                .child(
                                    div()
                                        .w(px(150.))
                                        .truncate()
                                        .text_color(rgb(palette.muted))
                                        .child(version_label(entry)),
                                )
                                .child(
                                    div()
                                        .w(px(240.))
                                        .truncate()
                                        .text_color(rgb(palette.muted))
                                        .child(entry.note.clone().unwrap_or_default()),
                                )
                                .into_any_element()
                        })
                        .collect();
                    rows.push(list(update_rows, palette));
                }

                if view.unknown > 0 {
                    rows.push(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} row(s) were not checked — not stale, but not confirmed current either.",
                                view.unknown
                            ))
                            .into_any_element(),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(rows)
                    .into_any_element()
            }
        };
        section("Updates", body, palette).into_any_element()
    }

    fn findings_panel(&self, palette: Palette) -> AnyElement {
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

impl Render for SystemsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        // Read the whole page into locals first: each panel borrows `self` mutably for its
        // listeners, so they are built one at a time and only then assembled.
        let runtime = self.runtime_panel(palette, cx);
        let mac = self.mac_panel(palette, cx);
        let agent = self.agent_panel(palette, cx);
        let storage = self.storage_panel(palette);
        let updates = self.updates_panel(palette, cx);
        let findings = self.findings_panel(palette);
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(runtime)
            .child(mac)
            .child(agent)
            .child(storage)
            .child(updates)
            .child(findings)
            .into_any_element()
    }
}

/// One metric card: a heading and whatever the value is.
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
fn bar(ratio: f32, tone: u32, palette: Palette) -> AnyElement {
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

/// The disk's own hourly history as a row of bars: one per sample, its height the size it held.
///
/// Bars rather than a line because a native polyline is a paint hook, and the shape that matters
/// here is "is it still filling" — which a row of bars answers without one.
fn trend_row(history: Option<&StorageHistory>, palette: Palette) -> AnyElement {
    let Some(trend) = history.and_then(StorageHistory::trend) else {
        return div().into_any_element();
    };
    if !trend.enough {
        let days = history.map_or(0, |history| history.retention_days);
        return div()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(format!(
                "Growth history starts after the first hourly sample; {days} days are kept."
            ))
            .into_any_element();
    }
    let bars: Vec<AnyElement> = trend
        .bars
        .iter()
        .map(|share| {
            let height = (share * 24.0).max(2.0);
            div()
                .w(px(2.))
                .h(px(height))
                .rounded(px(1.))
                .bg(rgba(tint(palette.accent, 0xaa)))
                .into_any_element()
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_end()
                .gap(px(1.))
                .h(px(24.))
                .children(bars),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "{} used · {} unattributed over {}",
                    trend.delta_label, trend.unattributed_delta_label, trend.span_label
                )),
        )
        .into_any_element()
}

fn heading(title: impl Into<SharedString>, palette: Palette) -> AnyElement {
    div()
        .pt_2()
        .text_xs()
        .text_color(rgb(palette.muted))
        .child(title.into())
        .into_any_element()
}

fn gate_row(
    gate: &AgentGate,
    agent: &AgentView,
    palette: Palette,
    busy: &Busy,
    cx: &mut Context<SystemsPage>,
) -> AnyElement {
    let modes: Vec<Button> = agent
        .modes
        .iter()
        .map(|mode| {
            let chosen = gate.mode == *mode;
            let picked = mode.clone();
            let in_flight = busy.contains(&format!("agent:{}", gate.capability));
            let gate_clone = gate.clone();
            Button::new(SharedString::from(format!(
                "agent-{}-{mode}",
                gate.capability
            )))
            .small()
            .when(!chosen, |button| button.ghost())
            .label(mode.clone())
            .loading(in_flight && chosen)
            .on_click(cx.listener(move |page, _, window, cx| {
                if !chosen {
                    let mode: &'static str = match picked.as_str() {
                        "off" => "off",
                        "read-only" => "read-only",
                        "ask" => "ask",
                        _ => "auto",
                    };
                    page.set_agent_mode(gate_clone.clone(), mode, window, cx);
                }
            }))
        })
        .collect();
    let asks = if gate.confirm.is_empty() {
        None
    } else {
        Some(format!(
            "{} action(s) always ask: {}",
            gate.confirm.len(),
            gate.confirm.join(", ")
        ))
    };
    row()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().truncate().text_sm().child(gate.capability.clone()))
                .children(asks.map(|asks| {
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(asks)
                })),
        )
        .child(div().flex().flex_none().gap_1().children(modes))
        .into_any_element()
}

fn approval_row(
    approval: &Approval,
    palette: Palette,
    busy: &Busy,
    cx: &mut Context<SystemsPage>,
) -> AnyElement {
    let in_flight = busy.contains(&format!("approval:{}", approval.id));
    let allow_id = approval.id.clone();
    let deny_id = approval.id.clone();
    let when = Local
        .timestamp_opt(approval.created_at, 0)
        .single()
        .map(|at| at.format("%H:%M").to_string())
        .unwrap_or_default();
    row()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .child(format!("{} {}", approval.method, approval.path)),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{} · {when}", approval.capability)),
                )
                .children((!approval.preview.is_empty()).then(|| {
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(approval.preview.clone())
                })),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap_1()
                .child(
                    Button::new(SharedString::from(format!("deny-{}", approval.id)))
                        .small()
                        .ghost()
                        .label("Deny")
                        .loading(in_flight)
                        .on_click(cx.listener(move |page, _, window, cx| {
                            page.decide_agent(deny_id.clone(), false, window, cx);
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("allow-{}", approval.id)))
                        .small()
                        .label("Allow")
                        .loading(in_flight)
                        .on_click(cx.listener(move |page, _, window, cx| {
                            page.decide_agent(allow_id.clone(), true, window, cx);
                        })),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_slot_is_not_read_twice_at_once() {
        let mut page = SystemsPage::new(Repo::new(Err(crate::shell::ShellError::NotLoopback)));
        assert!(page.macmon.begin(), "the first read starts");
        assert!(!page.macmon.begin(), "a second read waits for the first");
        assert_eq!(page.generation, 0);
    }
}
